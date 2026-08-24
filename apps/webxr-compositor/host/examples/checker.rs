//! A deterministic shm test client: four solid colour quadrants that rotate
//! one step every [`TICKS_PER_STEP`] frame callbacks. The browser tests
//! sample known pixels of it, so keep the geometry and palette stable.
//!
//! Test fixture: panicking is its failure reporting.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::io::AsFd;

use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const WIDTH: usize = 320;
const HEIGHT: usize = 240;
const STRIDE: usize = WIDTH * 4;
/// Frame callbacks between rotations; the host acks at roughly 60 Hz, so a
/// step lands about twice a second.
const TICKS_PER_STEP: u32 = 30;
/// Quadrant palette in XRGB8888 little-endian bytes [B, G, R, X]:
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
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    xdg: Option<(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel)>,
    pool_file: Option<File>,
    buffers: Vec<wl_buffer::WlBuffer>,
    active: usize,
    phase: u32,
    ticks: u32,
    mapped: bool,
}

impl App {
    /// Once the three needed globals exist, build the window.
    fn try_init(&mut self, qh: &QueueHandle<Self>) {
        if self.surface.is_some() {
            return;
        }
        let (Some(compositor), Some(wm_base)) = (&self.compositor, &self.wm_base) else {
            return;
        };
        let surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        toplevel.set_title("checker".into());
        toplevel.set_app_id("org.overby.checker".into());
        surface.commit();
        self.surface = Some(surface);
        self.xdg = Some((xdg_surface, toplevel));
    }

    fn ensure_buffers(&mut self, qh: &QueueHandle<Self>) {
        if !self.buffers.is_empty() {
            return;
        }
        let shm = self.shm.as_ref().expect("wl_shm was never advertised");
        let file = tempfile::tempfile().expect("no tempfile for the shm pool");
        file.set_len((2 * STRIDE * HEIGHT) as u64).unwrap();
        let pool = shm.create_pool(file.as_fd(), (2 * STRIDE * HEIGHT) as i32, qh, ());
        for index in 0..2usize {
            let buffer = pool.create_buffer(
                (index * STRIDE * HEIGHT) as i32,
                WIDTH as i32,
                HEIGHT as i32,
                STRIDE as i32,
                wl_shm::Format::Xrgb8888,
                qh,
                (),
            );
            self.buffers.push(buffer);
        }
        self.pool_file = Some(file);
    }

    fn draw(&mut self, index: usize) {
        let file = self.pool_file.as_mut().expect("pool file exists");
        file.seek(SeekFrom::Start((index * STRIDE * HEIGHT) as u64)).unwrap();
        let mut row = Vec::with_capacity(STRIDE);
        for y in 0..HEIGHT {
            row.clear();
            for x in 0..WIDTH {
                let quadrant = usize::from(x >= WIDTH / 2) + 2 * usize::from(y >= HEIGHT / 2);
                let color = COLORS[(quadrant + self.phase as usize) % 4];
                row.extend_from_slice(&color);
            }
            file.write_all(&row).unwrap();
        }
        file.flush().unwrap();
    }

    fn present(&mut self, qh: &QueueHandle<Self>) {
        self.ensure_buffers(qh);
        self.active ^= 1;
        let index = self.active;
        self.draw(index);
        let surface = self.surface.as_ref().expect("surface exists");
        surface.attach(Some(&self.buffers[index]), 0, 0);
        surface.damage_buffer(0, 0, WIDTH as i32, HEIGHT as i32);
        surface.frame(qh, ());
        surface.commit();
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
                app.present(qh);
            } else {
                // Keep the callback chain alive without repainting.
                let surface = app.surface.as_ref().expect("surface exists");
                surface.frame(qh, ());
                surface.commit();
            }
        }
    }
}

delegate_noop!(App: ignore wl_compositor::WlCompositor);
delegate_noop!(App: ignore wl_surface::WlSurface);
delegate_noop!(App: ignore wl_shm::WlShm);
delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
delegate_noop!(App: ignore wl_buffer::WlBuffer);
