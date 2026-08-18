//! Clipboard protocol layer built on top of the raw [`wire`] connection.
//!
//! Two compositor protocols expose the clipboard to focus-less clients, and
//! both share an identical opcode layout, so a single code path drives either:
//!
//! * `ext-data-control-v1` — the standardized protocol (GNOME/Mutter, and the
//!   eventual common baseline). Manager version 1; primary selection is core.
//! * `wlr-data-control-unstable-v1` — the wlroots protocol (sway, Hyprland,
//!   …). Manager version 2 adds primary-selection support.
//!
//! When both are advertised, `ext` is preferred.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::AsRawFd;

use crate::wire::{ArgReader, Connection, DISPLAY_ID, Message, Msg};

// wl_display requests
const DISPLAY_SYNC: u16 = 0;
const DISPLAY_GET_REGISTRY: u16 = 1;
// wl_display events
const DISPLAY_ERROR: u16 = 0;

// wl_registry request / events
const REGISTRY_BIND: u16 = 0;
const REGISTRY_GLOBAL: u16 = 0;

// data_control_manager requests (ext and wlr share these)
const MANAGER_CREATE_SOURCE: u16 = 0;
const MANAGER_GET_DEVICE: u16 = 1;

// data_control_device requests
const DEVICE_SET_SELECTION: u16 = 0;
const DEVICE_SET_PRIMARY_SELECTION: u16 = 2;
// data_control_device events
const DEVICE_DATA_OFFER: u16 = 0;
const DEVICE_SELECTION: u16 = 1;
const DEVICE_FINISHED: u16 = 2;
const DEVICE_PRIMARY_SELECTION: u16 = 3;

// data_control_source request / events
const SOURCE_OFFER: u16 = 0;
const SOURCE_SEND: u16 = 0;
const SOURCE_CANCELLED: u16 = 1;

// data_control_offer request / event
const OFFER_RECEIVE: u16 = 0;
const OFFER_OFFER: u16 = 0;

const EXT_MANAGER: &str = "ext_data_control_manager_v1";
const WLR_MANAGER: &str = "zwlr_data_control_manager_v1";

/// A data-control manager global discovered in the registry.
#[derive(Clone)]
struct Manager {
    name: u32,
    interface: String,
    /// Version we will bind at (capped to what we understand).
    version: u32,
    /// Higher wins when several managers are advertised.
    rank: u32,
}

/// All mutable state threaded through the event loop.
pub struct State {
    registry_id: u32,
    seat_global: Option<u32>,
    manager: Option<Manager>,

    pub manager_id: u32,
    pub device_id: u32,
    pub source_id: u32,

    /// Pending data offers and the MIME types each advertises (paste side).
    offers: HashMap<u32, Vec<String>>,
    /// The offer backing the regular selection, or `None` if empty.
    pub selection: Option<u32>,
    /// The offer backing the primary selection, or `None` if empty.
    pub primary_selection: Option<u32>,

    /// Bytes served to pasting clients (copy side).
    pub data: Vec<u8>,
    /// Number of paste requests to serve before stopping; 0 means unlimited.
    pub loops: u32,
    served: u32,

    /// Set once the copy daemon should stop (loops reached, or selection lost).
    pub finished: bool,
    /// Set when another client took over our selection.
    pub cancelled: bool,
}

impl State {
    pub fn new() -> State {
        State {
            registry_id: 0,
            seat_global: None,
            manager: None,
            manager_id: 0,
            device_id: 0,
            source_id: 0,
            offers: HashMap::new(),
            selection: None,
            primary_selection: None,
            data: Vec::new(),
            loops: 0,
            served: 0,
            finished: false,
            cancelled: false,
        }
    }

    /// MIME types advertised by a given offer (paste side).
    pub fn offer_mimes(&self, offer: u32) -> &[String] {
        self.offers.get(&offer).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Whether the negotiated protocol/version supports the primary selection.
    pub fn primary_supported(&self) -> bool {
        match &self.manager {
            // ext-data-control has primary selection in its core (v1);
            // wlr-data-control gained it in version 2.
            Some(m) => m.interface == EXT_MANAGER || m.version >= 2,
            None => false,
        }
    }
}

/// Discover globals and bind the seat, manager, and data device.
pub fn setup(conn: &mut Connection, st: &mut State) -> io::Result<()> {
    st.registry_id = conn.alloc_id();
    conn.send(Msg::new(DISPLAY_ID, DISPLAY_GET_REGISTRY).new_id(st.registry_id))?;
    roundtrip(conn, st)?;

    let manager = st.manager.clone().ok_or_else(|| {
        io::Error::new(
            ErrorKind::Unsupported,
            "the compositor does not support the wlr-data-control or ext-data-control protocol",
        )
    })?;
    let seat = st.seat_global.ok_or_else(|| {
        io::Error::new(ErrorKind::Unsupported, "the compositor exposes no wl_seat")
    })?;

    // Binding any global uses the same encoding: name, interface, version, id.
    let seat_id = conn.alloc_id();
    conn.send(
        Msg::new(st.registry_id, REGISTRY_BIND)
            .uint(seat)
            .string("wl_seat")
            .uint(1)
            .new_id(seat_id),
    )?;

    st.manager_id = conn.alloc_id();
    conn.send(
        Msg::new(st.registry_id, REGISTRY_BIND)
            .uint(manager.name)
            .string(&manager.interface)
            .uint(manager.version)
            .new_id(st.manager_id),
    )?;

    st.device_id = conn.alloc_id();
    conn.send(
        Msg::new(st.manager_id, MANAGER_GET_DEVICE)
            .new_id(st.device_id)
            .object(seat_id),
    )?;
    Ok(())
}

/// Create a data source offering `mimes` and install it as the (primary)
/// selection. The source must be served afterwards via [`serve`].
pub fn set_selection(
    conn: &mut Connection,
    st: &mut State,
    mimes: &[String],
    primary: bool,
) -> io::Result<()> {
    st.source_id = conn.alloc_id();
    conn.send(Msg::new(st.manager_id, MANAGER_CREATE_SOURCE).new_id(st.source_id))?;
    for mime in mimes {
        conn.send(Msg::new(st.source_id, SOURCE_OFFER).string(mime))?;
    }
    let opcode = if primary {
        DEVICE_SET_PRIMARY_SELECTION
    } else {
        DEVICE_SET_SELECTION
    };
    conn.send(Msg::new(st.device_id, opcode).object(st.source_id))?;
    roundtrip(conn, st)
}

/// Ask the offer to deliver `mime`, returning the received bytes.
///
/// The data travels through a pipe whose write end is handed to the owning
/// client via the `receive` request; we drain the read end to EOF.
pub fn receive(conn: &mut Connection, offer: u32, mime: &str) -> io::Result<Vec<u8>> {
    let (read_end, write_end) = crate::wire::pipe()?;
    conn.send(
        Msg::new(offer, OFFER_RECEIVE)
            .string(mime)
            .fd(write_end.as_raw_fd()),
    )?;
    // Drop our write end so we see EOF once the source finishes writing.
    drop(write_end);
    let mut data = Vec::new();
    File::from(read_end).read_to_end(&mut data)?;
    Ok(data)
}

/// Serve paste requests for an installed selection until [`State::finished`].
pub fn serve(conn: &mut Connection, st: &mut State) -> io::Result<()> {
    while !st.finished {
        let msg = conn.next_message()?;
        dispatch(conn, st, &msg)?;
    }
    Ok(())
}

/// Issue `wl_display.sync` and pump events until the matching callback fires,
/// guaranteeing every preceding request has been processed.
pub fn roundtrip(conn: &mut Connection, st: &mut State) -> io::Result<()> {
    let callback = conn.alloc_id();
    conn.send(Msg::new(DISPLAY_ID, DISPLAY_SYNC).new_id(callback))?;
    loop {
        let msg = conn.next_message()?;
        // wl_callback.done (opcode 0) on our sync callback ends the roundtrip.
        if msg.sender == callback && msg.opcode == 0 {
            return Ok(());
        }
        dispatch(conn, st, &msg)?;
    }
}

/// Route one event to the right handler based on its sender object.
fn dispatch(conn: &mut Connection, st: &mut State, msg: &Message) -> io::Result<()> {
    let mut r = ArgReader::new(&msg.args);

    if msg.sender == DISPLAY_ID {
        if msg.opcode == DISPLAY_ERROR {
            let object = r.uint();
            let code = r.uint();
            let text = r.string().unwrap_or_default();
            return Err(io::Error::other(format!(
                "Wayland protocol error from object {object} (code {code}): {text}"
            )));
        }
        // delete_id and any other display events need no action here.
        return Ok(());
    }

    if msg.sender == st.registry_id {
        if msg.opcode == REGISTRY_GLOBAL {
            let name = r.uint();
            let interface = r.string().unwrap_or_default();
            let version = r.uint();
            handle_global(st, name, &interface, version);
        }
        return Ok(());
    }

    if st.device_id != 0 && msg.sender == st.device_id {
        match msg.opcode {
            DEVICE_DATA_OFFER => {
                let id = r.new_id();
                st.offers.insert(id, Vec::new());
            }
            DEVICE_SELECTION => {
                let id = r.object();
                st.selection = (id != 0).then_some(id);
            }
            DEVICE_PRIMARY_SELECTION => {
                let id = r.object();
                st.primary_selection = (id != 0).then_some(id);
            }
            DEVICE_FINISHED => st.finished = true,
            _ => {}
        }
        return Ok(());
    }

    if st.source_id != 0 && msg.sender == st.source_id {
        match msg.opcode {
            SOURCE_SEND => {
                let _mime = r.string();
                if let Some(fd) = conn.take_fd() {
                    // Errors (e.g. the reader closed early) are not fatal.
                    let _ = File::from(fd).write_all(&st.data);
                }
                st.served += 1;
                if st.loops != 0 && st.served >= st.loops {
                    st.finished = true;
                }
            }
            SOURCE_CANCELLED => {
                st.cancelled = true;
                st.finished = true;
            }
            _ => {}
        }
        return Ok(());
    }

    if let Some(mimes) = st.offers.get_mut(&msg.sender)
        && msg.opcode == OFFER_OFFER
        && let Some(mime) = r.string()
    {
        mimes.push(mime);
    }
    Ok(())
}

/// Record a useful registry global (seat or a data-control manager).
fn handle_global(st: &mut State, name: u32, interface: &str, version: u32) {
    if interface == "wl_seat" {
        st.seat_global.get_or_insert(name);
        return;
    }
    // Prefer ext-data-control over wlr-data-control when both exist.
    let (rank, max_version) = match interface {
        EXT_MANAGER => (2, 1),
        WLR_MANAGER => (1, 2),
        _ => return,
    };
    if st.manager.as_ref().is_none_or(|m| rank > m.rank) {
        st.manager = Some(Manager {
            name,
            interface: interface.to_string(),
            version: version.min(max_version),
            rank,
        });
    }
}
