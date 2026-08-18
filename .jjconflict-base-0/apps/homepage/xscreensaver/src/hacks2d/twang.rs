//! Port of `hacks/twang.c`.
//!
//! ```text
//! twang, twist around screen bits, v1.3
//! by Dan Bornstein, danfuzz@milk.com
//! Copyright (c) 2003 Dan Bornstein. All rights reserved.
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! See the included man page for more details.
//! ```
//!
//! The picture is cut into square tiles and each one is hung on a spring. A
//! tile has two things it can do, turn and zoom, and each is a spring pulling
//! it back to rest, damped by friction, and coupled to its four neighbours so
//! a disturbance travels across the grid as a wave. Every so often something
//! is plucked: one tile, one row, one column, or the lot, and the wave spreads
//! out from there.
//!
//! The tiles are drawn back to front in order of zoom, so a tile that has
//! swelled towards you covers its neighbours, and each is rendered by walking
//! its destination pixels and rotating backwards into the picture, which is
//! why the rotation never leaves a gap. A tile is drawn from the picture
//! inside its inner border and in the border colour between the inner and
//! outer, so the grid reads as a set of framed tiles rather than a cut-up.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::ALPHA;
use crate::runtime::{
    About, Dpy, ImageLoad, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XEvent, XImage,
    random, screenhack_event_helper,
};

/// Random float in the range (-1..1).
fn rand_pm1() -> f64 {
    (((random() >> 8) & 0xffff) as f64) / 65536.0 * 2.0 - 1.0
}

/// Random float in the range (0..1).
fn rand_01() -> f64 {
    (((random() >> 8) & 0xffff) as f64) / 65536.0
}

const MAX_VANGLE: f64 = std::f64::consts::FRAC_PI_4;
const MAX_VZOOM: f64 = 0.25;

fn rand_angle() -> f64 {
    rand_pm1() * std::f64::consts::PI
}
fn rand_zoom() -> f64 {
    rand_pm1()
}
fn rand_vangle() -> f64 {
    rand_pm1() * MAX_VANGLE
}
fn rand_vzoom() -> f64 {
    rand_pm1() * MAX_VZOOM
}

#[derive(Clone, Copy, Default)]
struct Tile {
    /// Coordinates of the centre of the tile.
    x: i32,
    y: i32,
    /// Angle of the tile, in the range -pi..pi.
    angle: f64,
    /// Log of the zoom of the tile, in the range -1..1.
    zoom: f64,
    /// Angular velocity, in the range -pi/4..pi/4.
    v_angle: f64,
    /// Zoomular velocity, in the range -0.25..0.25.
    v_zoom: f64,
}

struct State {
    delay: u32,
    /// Time before loading a new image.
    duration: f64,
    start_time: f64,
    max_columns: i32,
    max_rows: i32,
    tile_size: i32,
    border_width: i32,
    /// The chance, per iteration, of an interesting event happening.
    event_chance: f64,
    /// The fraction by which velocity decreases per iteration.
    friction: f64,
    /// The fraction of the orientation that turns into velocity towards the
    /// centre.
    springiness: f64,
    /// The fraction of the orientations of orthogonal neighbours that turns
    /// into velocity, in the same direction as the orientation.
    transference: f64,
    window_width: i32,
    window_height: i32,
    /// The picture the tiles are cut from.
    source: Option<XImage>,
    border_pixel: Pixel,
    /// The tiles, left to right then top to bottom, row major.
    tiles: Vec<Tile>,
    /// Indices into `tiles`, kept sorted by zoom.
    sorted: Vec<usize>,
    rows: i32,
    columns: i32,
    img_loader: Option<ImageLoad>,
    loading: bool,
}

impl State {
    fn tile_at(&mut self, c: i32, r: i32) -> &mut Tile {
        &mut self.tiles[(r * self.columns + c) as usize]
    }

    fn randomize_all_angular_velocities(&mut self) {
        for r in 0..self.rows {
            for c in 0..self.columns {
                let v = rand_vangle();
                self.tile_at(c, r).v_angle = v;
            }
        }
    }

    fn randomize_all_zoomular_velocities(&mut self) {
        for r in 0..self.rows {
            for c in 0..self.columns {
                let v = rand_vzoom();
                self.tile_at(c, r).v_zoom = v;
            }
        }
    }

    fn randomize_all_velocities(&mut self) {
        self.randomize_all_angular_velocities();
        self.randomize_all_zoomular_velocities();
    }

    fn randomize_all_angular_orientations(&mut self) {
        for r in 0..self.rows {
            for c in 0..self.columns {
                let v = rand_angle();
                self.tile_at(c, r).angle = v;
            }
        }
    }

    fn randomize_all_zoomular_orientations(&mut self) {
        for r in 0..self.rows {
            for c in 0..self.columns {
                let v = rand_zoom();
                self.tile_at(c, r).zoom = v;
            }
        }
    }

    fn randomize_all_orientations(&mut self) {
        self.randomize_all_angular_orientations();
        self.randomize_all_zoomular_orientations();
    }

    fn randomize_everything(&mut self) {
        self.randomize_all_velocities();
        self.randomize_all_orientations();
    }

    fn pluck(&mut self, c: i32, r: i32) {
        let (a, z, va, vz) = (rand_angle(), rand_zoom(), rand_vangle(), rand_vzoom());
        let t = self.tile_at(c, r);
        t.angle = a;
        t.zoom = z;
        t.v_angle = va;
        t.v_zoom = vz;
    }

    fn randomize_one_tile(&mut self) {
        let c = (rand_01() * self.columns as f64) as i32;
        let r = (rand_01() * self.rows as f64) as i32;
        self.pluck(c, r);
    }

    fn randomize_one_row(&mut self) {
        let r = (rand_01() * self.rows as f64) as i32;
        for c in 0..self.columns {
            self.pluck(c, r);
        }
    }

    fn randomize_one_column(&mut self) {
        let c = (rand_01() * self.columns as f64) as i32;
        for r in 0..self.rows {
            self.pluck(c, r);
        }
    }

    fn model_events(&mut self) {
        if rand_01() > self.event_chance {
            return;
        }
        match (rand_01() * 10.0) as i32 {
            0 => self.randomize_all_angular_velocities(),
            1 => self.randomize_all_zoomular_velocities(),
            2 => self.randomize_all_velocities(),
            3 => self.randomize_all_angular_orientations(),
            4 => self.randomize_all_zoomular_orientations(),
            5 => self.randomize_all_orientations(),
            6 => self.randomize_everything(),
            7 => self.randomize_one_tile(),
            8 => self.randomize_one_column(),
            9 => self.randomize_one_row(),
            _ => {}
        }
    }

    fn update_model(&mut self) {
        // For each tile, decrease its velocities according to the friction,
        // and increase them based on its current orientation and the
        // orientations of its orthogonal neighbours.
        for r in 0..self.rows {
            for c in 0..self.columns {
                let i = (r * self.columns + c) as usize;
                let t = self.tiles[i];
                let (a, z) = (t.angle, t.zoom);
                let mut va = t.v_angle - a * self.springiness;
                let mut vz = t.v_zoom - z * self.springiness;

                let mut pull = |j: usize| {
                    let t2 = self.tiles[j];
                    va += (t2.angle - a) * self.transference;
                    vz += (t2.zoom - z) * self.transference;
                };
                if c > 0 {
                    pull(i - 1);
                }
                if c < self.columns - 1 {
                    pull(i + 1);
                }
                if r > 0 {
                    pull(i - self.columns as usize);
                }
                if r < self.rows - 1 {
                    pull(i + self.columns as usize);
                }

                va *= 1.0 - self.friction;
                vz *= 1.0 - self.friction;

                self.tiles[i].v_angle = va.clamp(-MAX_VANGLE, MAX_VANGLE);
                self.tiles[i].v_zoom = vz.clamp(-MAX_VZOOM, MAX_VZOOM);
            }
        }

        // For each tile, update its orientation based on its velocities.
        for t in &mut self.tiles {
            t.angle = (t.angle + t.v_angle).clamp(-std::f64::consts::PI, std::f64::consts::PI);
            t.zoom = (t.zoom + t.v_zoom).clamp(-1.0, 1.0);
        }
    }

    /// Walk the tile's destination pixels and rotate each backwards into the
    /// picture, which is why a turned tile never shows a gap.
    fn render_tile(&self, d: &mut Dpy, i: usize) {
        let Some(source) = &self.source else { return };
        let t = self.tiles[i];
        let (tx, ty) = (t.x, t.y);

        // The zoom as stored per tile is log-based, centred on zero, but the
        // range for zoom-as-drawn is 0.4 to 2.5.
        let zoom = 2.5f64.powf(t.zoom);
        let ang = -t.angle;
        let mut sin_ang = ang.sin();
        let mut cos_ang = ang.cos();

        let inner_border = (self.tile_size - self.border_width) as f64 / 2.0;
        let outer_border = inner_border + self.border_width as f64;

        let max_coord = (outer_border * zoom * (sin_ang.abs() + cos_ang.abs())) as i32;
        let min_x = (tx - max_coord).max(0);
        let max_x = (tx + max_coord).min(self.window_width);
        let min_y = (ty - max_coord).max(0);
        let max_y = (ty + max_coord).min(self.window_height);

        sin_ang /= zoom;
        cos_ang /= zoom;

        let mut prey = (min_y - ty) as f64;
        for y in min_y..max_y {
            let prex = (min_x - tx) as f64;
            let mut srcx = prex * cos_ang - prey * sin_ang;
            let mut srcy = prex * sin_ang + prey * cos_ang;

            for x in min_x..max_x {
                if srcx < -inner_border
                    || srcx >= inner_border
                    || srcy < -inner_border
                    || srcy >= inner_border
                {
                    if !(srcx < -outer_border
                        || srcx >= outer_border
                        || srcy < -outer_border
                        || srcy >= outer_border)
                    {
                        d.win().put_pixel(x, y, self.border_pixel);
                    }
                } else {
                    let p = source.get_pixel(srcx as i32 + tx, srcy as i32 + ty);
                    d.win().put_pixel(x, y, p);
                }
                srcx += cos_ang;
                srcy += sin_ang;
            }
            prey += 1.0;
        }
    }

    fn render_frame(&mut self, d: &mut Dpy) {
        // Upstream clears its work image with a memset, on the assumption that
        // black is zero.
        d.win().clear(ALPHA);

        self.sorted
            .sort_by(|&a, &b| self.tiles[a].zoom.total_cmp(&self.tiles[b].zoom));

        for n in 0..self.sorted.len() {
            let i = self.sorted[n];
            self.render_tile(d, i);
        }
    }

    fn setup_model(&mut self) {
        if self.tile_size > self.window_width / 2 {
            self.tile_size = self.window_width / 2;
        }
        if self.tile_size > self.window_height / 2 {
            self.tile_size = self.window_height / 2;
        }

        self.columns = if self.tile_size != 0 {
            self.window_width / self.tile_size
        } else {
            0
        };
        self.rows = if self.tile_size != 0 {
            self.window_height / self.tile_size
        } else {
            0
        };
        if self.max_columns != 0 && self.columns > self.max_columns {
            self.columns = self.max_columns;
        }
        if self.max_rows != 0 && self.rows > self.max_rows {
            self.rows = self.max_rows;
        }

        let tile_count = (self.rows * self.columns).max(1) as usize;
        let left_x = (self.window_width - (self.columns * self.tile_size) + self.tile_size) / 2;
        let top_y = (self.window_height - (self.rows * self.tile_size) + self.tile_size) / 2;

        self.tiles = vec![Tile::default(); tile_count];
        self.sorted = (0..tile_count).collect();

        for r in 0..self.rows {
            for c in 0..self.columns {
                let (x, y) = (left_x + c * self.tile_size, top_y + r * self.tile_size);
                let t = self.tile_at(c, r);
                t.x = x;
                t.y = y;
            }
        }

        self.randomize_everything();
    }

    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        self.start_time = d.time;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    fn image_arrived(&mut self, d: &mut Dpy) {
        let (w, h) = (self.window_width, self.window_height);
        self.source = Some(d.win_ref().sub_image(0, 0, w, h));
        self.start_time = d.time;
        self.loading = false;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (window_width, window_height) = (d.width(), d.height());
    let mut tile_size = d.res.int("tileSize").max(1);
    if window_width > 2560 || window_height > 2560 {
        tile_size *= 3; // Retina displays.
    }

    let border_pixel = d.res.pixel("borderColor");

    // Upstream prints a message and exits on each of these; clamp instead.
    let mut st = State {
        delay: d.res.int("delay").max(0) as u32,
        duration: d.res.int("duration").max(1) as f64,
        start_time: 0.0,
        max_columns: d.res.int("maxColumns").max(0),
        max_rows: d.res.int("maxRows").max(0),
        tile_size,
        border_width: d.res.int("borderWidth").max(0),
        event_chance: d.res.float("eventChance").clamp(0.0, 1.0),
        friction: d.res.float("friction").clamp(0.0, 1.0),
        springiness: d.res.float("springiness").clamp(0.0, 1.0),
        transference: d.res.float("transference").clamp(0.0, 1.0),
        window_width,
        window_height,
        source: None,
        border_pixel,
        tiles: Vec::new(),
        sorted: Vec::new(),
        rows: 0,
        columns: 0,
        img_loader: None,
        loading: false,
    };
    st.start_load(d);
    st.setup_model();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.loading {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return self.delay;
        }

        if self.start_time + self.duration < d.time {
            self.start_load(d);
            return self.delay;
        }

        self.model_events();
        self.update_model();
        self.render_frame(d);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape, which leaves the grid laid out for the old
        // window. The picture has to be fetched again at the new size anyway,
        // so re-cut the grid too.
        self.window_width = width;
        self.window_height = height;
        self.setup_model();
        self.start_load(d);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.start_time = f64::NEG_INFINITY;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*borderColor: blue",
    "*borderWidth: 3",
    "*delay: 10000",
    "*duration: 120",
    "*eventChance: 0.01",
    "*friction: 0.05",
    "*maxColumns: 0",
    "*maxRows: 0",
    "*springiness: 0.1",
    "*tileSize: 120",
    "*transference: 0.025",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::slider("eventChance", "Randomness", 0.0, 0.1, 0.005, 3, "0.01"),
    Opt::slider("friction", "Friction", 0.0, 0.2, 0.01, 2, "0.05"),
    Opt::slider("springiness", "Springiness", 0.0, 1.0, 0.05, 2, "0.1"),
    Opt::slider("transference", "Transference", 0.0, 0.1, 0.005, 3, "0.025"),
    Opt::slider("tileSize", "Tile size", 10.0, 512.0, 10.0, 0, "120"),
    Opt::spin("borderWidth", "Border width", 0.0, 20.0, "3"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "twang",
    label: "Twang",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Dan Bornstein",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=7pxDMSduQoU"),
        blurb: "Divides the screen into a grid, and plucks them.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
