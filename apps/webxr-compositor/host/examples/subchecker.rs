//! A deterministic wl_subsurface test client: a solid navy parent with a
//! child subsurface that cycles the checker palette every
//! [`TICKS_PER_STEP`] of its own frame callbacks and jumps between two
//! anchor points every [`TICKS_PER_MOVE`]. The browser tests sample known
//! pixels and offsets of it, so keep the geometry and palette stable.
//!
//! Test fixture: panicking is its failure reporting.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::io::AsFd;

use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_subcompositor,
    wl_subsurface, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const PARENT_W: usize = 320;
const PARENT_H: usize = 240;
const PARENT_STRIDE: usize = PARENT_W * 4;
const SUB_W: usize = 120;
const SUB_H: usize = 90;
const SUB_STRIDE: usize = SUB_W * 4;
/// Where the subsurface sits, alternating: the browser test asserts both.
const SPOTS: [(i32, i32); 2] = [(60, 40), (140, 90)];
const TICKS_PER_STEP: u32 = 30;
const TICKS_PER_MOVE: u32 = 45;
/// Parent fill, XRGB little-endian [B, G, R, X]: navy #102030.
const PARENT_COLOR: [u8; 4] = [0x30, 0x20, 0x10, 0xFF];
/// Child palette, same encoding as the checker example:
/// red #E53936, green #43A047, blue #1E88E5, yellow #FDD835.
const COLORS: [[u8; 4]; 4] = [
    [0x36, 0x39, 0xE5, 0xFF],
    [0x47, 0xA0, 0x43, 0xFF],
    [0xE5, 0x88, 0x1E, 0xFF],
    [0x35, 0xD8, 0xFD, 0xFF],
];

fn main() {
    let conn = Connection::connect_to_env().expect("no wayland socket to connect to");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());

    let mut app = App {
        running: true,
        ..App::default()
    };
    while app.running {
        queue.blocking_dispatch(&mut app).expect("wayland dispatch failed");
    }
}

#[derive(Default)]
struct App {
    running: bool,
    compositor: Option<wl_compositor::WlCompositor>,
    subcompositor: Option<wl_subcompositor::WlSubcompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    child: Option<wl_surface::WlSurface>,
    subsurface: Option<wl_subsurface::WlSubsurface>,
    xdg: Option<(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel)>,
    pool_file: Option<File>,
    parent_buffer: Option<wl_buffer::WlBuffer>,
    child_buffers: Vec<wl_buffer::WlBuffer>,
    active: usize,
    phase: u32,
    ticks: u32,
    spot: usize,
    mapped: bool,
}

impl App {
    fn try_init(&mut self, qh: &QueueHandle<Self>) {
        if self.surface.is_some() {
            return;
        }
        let (Some(compositor), Some(subcompositor), Some(wm_base)) =
            (&self.compositor, &self.subcompositor, &self.wm_base)
        else {
            return;
        };
        let surface = compositor.create_surface(qh, ());
        let child = compositor.create_surface(qh, ());
        let subsurface = subcompositor.get_subsurface(&child, &surface, qh, ());
        subsurface.set_position(SPOTS[0].0, SPOTS[0].1);
        // Desync: the child animates on its own commits, without the parent
        // having to repaint.
        subsurface.set_desync();
        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        toplevel.set_title("subchecker".into());
        toplevel.set_app_id("org.overby.subchecker".into());
        surface.commit();
        self.surface = Some(surface);
        self.child = Some(child);
        self.subsurface = Some(subsurface);
        self.xdg = Some((xdg_surface, toplevel));
    }

    fn ensure_buffers(&mut self, qh: &QueueHandle<Self>) {
        if self.parent_buffer.is_some() {
            return;
        }
        let shm = self.shm.as_ref().expect("wl_shm was never advertised");
        let parent_len = PARENT_STRIDE * PARENT_H;
        let child_len = SUB_STRIDE * SUB_H;
        let total = parent_len + 2 * child_len;
        let file = tempfile::tempfile().expect("no tempfile for the shm pool");
        file.set_len(total as u64).unwrap();
        let pool = shm.create_pool(file.as_fd(), total as i32, qh, ());
        self.parent_buffer = Some(pool.create_buffer(
            0,
            PARENT_W as i32,
            PARENT_H as i32,
            PARENT_STRIDE as i32,
            wl_shm::Format::Xrgb8888,
            qh,
            (),
        ));
        for index in 0..2usize {
            self.child_buffers.push(pool.create_buffer(
                (parent_len + index * child_len) as i32,
                SUB_W as i32,
                SUB_H as i32,
                SUB_STRIDE as i32,
                wl_shm::Format::Xrgb8888,
                qh,
                (),
            ));
        }
        let mut file = file;
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut row = Vec::with_capacity(PARENT_STRIDE);
        for _ in 0..PARENT_W {
            row.extend_from_slice(&PARENT_COLOR);
        }
        for _ in 0..PARENT_H {
            file.write_all(&row).unwrap();
        }
        file.flush().unwrap();
        self.pool_file = Some(file);
    }

    fn draw_child(&mut self, index: usize) {
        let color = COLORS[self.phase as usize % 4];
        let file = self.pool_file.as_mut().expect("pool file exists");
        let offset = PARENT_STRIDE * PARENT_H + index * SUB_STRIDE * SUB_H;
        file.seek(SeekFrom::Start(offset as u64)).unwrap();
        let mut row = Vec::with_capacity(SUB_STRIDE);
        for _ in 0..SUB_W {
            row.extend_from_slice(&color);
        }
        for _ in 0..SUB_H {
            file.write_all(&row).unwrap();
        }
        file.flush().unwrap();
    }

    /// First map: attach the parent fill and the child, then hand the
    /// animation to the child's frame callbacks.
    fn present(&mut self, qh: &QueueHandle<Self>) {
        self.ensure_buffers(qh);
        let surface = self.surface.as_ref().expect("surface exists");
        surface.attach(self.parent_buffer.as_ref(), 0, 0);
        surface.damage_buffer(0, 0, PARENT_W as i32, PARENT_H as i32);
        surface.commit();
        self.present_child(qh);
    }

    fn present_child(&mut self, qh: &QueueHandle<Self>) {
        self.active ^= 1;
        let index = self.active;
        self.draw_child(index);
        let child = self.child.as_ref().expect("child exists");
        child.attach(Some(&self.child_buffers[index]), 0, 0);
        child.damage_buffer(0, 0, SUB_W as i32, SUB_H as i32);
        child.frame(qh, ());
        child.commit();
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        app: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    app.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, 4, qh, ()));
                }
                "wl_subcompositor" => {
                    app.subcompositor = Some(
                        registry.bind::<wl_subcompositor::WlSubcompositor, _, _>(name, 1, qh, ()),
                    );
                }
                "wl_shm" => {
                    app.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                }
                "xdg_wm_base" => {
                    app.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                }
                _ => {}
            }
            app.try_init(qh);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        app: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if !app.mapped {
                app.mapped = true;
                app.present(qh);
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        app: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            app.running = false;
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            app.ticks += 1;
            if app.ticks.is_multiple_of(TICKS_PER_STEP) {
                app.phase = app.phase.wrapping_add(1);
            }
            if app.ticks.is_multiple_of(TICKS_PER_MOVE) {
                app.spot ^= 1;
                let subsurface = app.subsurface.as_ref().expect("subsurface exists");
                subsurface.set_position(SPOTS[app.spot].0, SPOTS[app.spot].1);
                // set_position is double-buffered on the parent, so the jump
                // needs a parent commit even though its content is unchanged.
                app.surface.as_ref().expect("surface exists").commit();
            }
            app.present_child(qh);
        }
    }
}

delegate_noop!(App: ignore wl_compositor::WlCompositor);
delegate_noop!(App: ignore wl_subcompositor::WlSubcompositor);
delegate_noop!(App: ignore wl_subsurface::WlSubsurface);
delegate_noop!(App: ignore wl_surface::WlSurface);
delegate_noop!(App: ignore wl_shm::WlShm);
delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
delegate_noop!(App: ignore wl_buffer::WlBuffer);
