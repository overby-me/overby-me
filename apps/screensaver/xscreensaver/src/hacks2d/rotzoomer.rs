//! Port of `hacks/rotzoomer.c`.
//!
//! ```text
//! rotzoomer - creates a collage of rotated and scaled portions of the screen
//! Copyright (C) 2001-2016 Claudio Matsuoka <claudio@helllabs.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Circle-mode by jwz, 2014, 2016.
//! ```
//!
//! Rectangles of the picture are resampled through a rotation and a scale that
//! both wind round on their own counters, and the source coordinates wrap, so
//! each box fills with a tilted, repeating tiling of somewhere else on screen.
//! The boxes can sit still, wander, sweep across as a bar, or be discs that
//! spin what is under them and then hand the result back to the picture, so
//! the smearing compounds.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Gc, ImageLoad, Opt, Pixmap, Runner, SaverDef, Screenhack, SelectItem, StartArgs,
    XEvent, random, random_below, screenhack_event_helper,
};

#[derive(Default)]
struct ZoomArea {
    /// Rectangle width and height.
    w: i32,
    h: i32,
    /// Rotation and zoom angle increments.
    inc1: i32,
    inc2: i32,
    /// Translation increments.
    dx: i32,
    dy: i32,
    /// Rotation and zoom angular variables.
    a1: i32,
    a2: i32,
    /// Left-upper corner position, times 256, and rounded down.
    xx: i32,
    yy: i32,
    x: i32,
    y: i32,
    /// Valid area to place the left-upper corner in.
    ww: i32,
    hh: i32,
    /// Number of iterations left, and how many have run.
    n: i32,
    count: i32,
}

struct State {
    gc: Gc,
    orig_map: Pixmap,
    buffer_map: Pixmap,
    width: i32,
    height: i32,
    zoom_box: Vec<ZoomArea>,
    num_zoom: usize,
    move_p: bool,
    sweep: bool,
    circle: bool,
    delay: u32,
    anim: bool,
    duration: f64,
    start_time: f64,
    img_loader: Option<ImageLoad>,
    loading: bool,
}

/// `((2 * (random() & 1)) - 1)`: minus one or plus one.
fn sign() -> i32 {
    2 * (random() & 1) as i32 - 1
}

impl State {
    fn rotzoom(&mut self, i: usize) {
        let za = &self.zoom_box[i];
        let (x1, y1) = (za.x, za.y);
        let x2 = za.x + za.w - 1;
        let y2 = za.y + za.h - 1;
        let w2 = (za.w / 2) * (za.w / 2);
        let (cx, cy) = (za.x + za.w / 2, za.y + za.h / 2);
        let (a1, a2) = (za.a1, za.a2);
        let (inc1, inc2) = (za.inc1, za.inc2);
        let n = za.n;

        let z = 8100.0 * (std::f64::consts::PI * a2 as f64 / 8192.0).sin();
        let zoom = 8192.0 + z;
        // Upstream recomputes these inside the pixel loop, where nothing they
        // depend on changes.
        let a = std::f64::consts::PI * a1 as f64 / 8192.0;
        let c = (zoom * a.cos()) as i64;
        let s = (zoom * a.sin()) as i64;

        for y in y1..=y2 {
            for x in x1..=x2 {
                let mut copyp = true;
                let (mut ox, mut oy) = (0i32, 0i32);
                if self.circle {
                    let dx = x - cx;
                    let dy = y - cy;
                    let d2 = dx * dx + dy * dy;
                    if d2 > w2 {
                        copyp = false;
                    } else {
                        let r = (d2 as f64).sqrt();
                        let mut th = (dy as f64 / if dx == 0 { 1.0 } else { dx as f64 }).atan();
                        if dx < 0 {
                            th += std::f64::consts::PI;
                        }
                        th += std::f64::consts::PI * (a1 as f64 / 600.0);
                        ox = cx + (r * th.cos()) as i32;
                        oy = cy + (r * th.sin()) as i32;
                    }
                } else {
                    ox = ((x as i64 * c + y as i64 * s) >> 13) as i32;
                    oy = ((-(x as i64) * s + y as i64 * c) >> 13) as i32;
                }

                if copyp {
                    ox = ox.rem_euclid(self.width.max(1));
                    oy = oy.rem_euclid(self.height.max(1));
                    let p = self.orig_map.get_pixel(ox, oy);
                    self.buffer_map.put_pixel(x, y, p);
                }
            }
        }

        let za = &mut self.zoom_box[i];
        za.a1 = (za.a1 + inc1) & 0x3fff; // Rotation angle.
        za.a2 = (za.a2 + inc2) & 0x3fff; // Zoom.
        za.count += 1;

        if self.circle && n <= 1 {
            // Done rotating the circle: copy the bits from the working set
            // back into the origin, so that later rotations pick them up.
            for y in y1..y1 + self.zoom_box[i].h {
                for x in x1..x1 + self.zoom_box[i].w {
                    let dx = x - cx;
                    let dy = y - cy;
                    if dx * dx + dy * dy <= w2 {
                        let p = self.buffer_map.get_pixel(x, y);
                        self.orig_map.put_pixel(x, y, p);
                    }
                }
            }
        }
    }

    fn reset_zoom(&mut self, i: usize) {
        let (width, height) = (self.width, self.height);
        let za = &mut self.zoom_box[i];
        if self.sweep {
            let speed = random_below(100) + 100;
            match random_below(4) {
                0 => {
                    *za = ZoomArea {
                        w: width,
                        h: 10,
                        x: 0,
                        y: 0,
                        dx: 0,
                        dy: speed,
                        n: (height - 10) * 256 / speed,
                        ..ZoomArea::default()
                    }
                }
                1 => {
                    *za = ZoomArea {
                        w: 10,
                        h: height,
                        x: width - 10,
                        y: 0,
                        dx: -speed,
                        dy: 0,
                        n: (width - 10) * 256 / speed,
                        ..ZoomArea::default()
                    }
                }
                2 => {
                    *za = ZoomArea {
                        w: width,
                        h: 10,
                        x: 0,
                        y: height - 10,
                        dx: 0,
                        dy: -speed,
                        n: (height - 10) * 256 / speed,
                        ..ZoomArea::default()
                    }
                }
                _ => {
                    *za = ZoomArea {
                        w: 10,
                        h: height,
                        x: 0,
                        y: 0,
                        dx: speed,
                        dy: 0,
                        n: (width - 10) * 256 / speed,
                        ..ZoomArea::default()
                    }
                }
            }
            za.ww = width - za.w;
            za.hh = height - za.h;

            // Smaller angle increments in sweep mode; it looks better.
            za.a1 = 0;
            za.a2 = 0;
            za.inc1 = sign() * (1 + random_below(7));
            za.inc2 = sign() * (1 + random_below(7));
        } else if self.circle {
            za.w = 50 + random_below(300);
            za.w = za.w.min(width / 3).min(height / 3);
            za.h = za.w;
            za.ww = width - za.w;
            za.hh = height - za.h;
            za.x = if za.ww != 0 { random_below(za.ww) } else { 0 };
            za.y = if za.hh != 0 { random_below(za.hh) } else { 0 };
            za.dx = 0;
            za.dy = 0;
            za.a1 = 0;
            za.a2 = 0;
            za.count = 0;

            // Going clockwise does not start rotating from 0, so upstream only
            // goes counter-clockwise.
            za.inc1 = random_below(30);
            za.inc2 = 0;
            za.n = 50 + random_below(100);

            if !self.anim {
                za.count = random_below((za.n / 2).max(1));
                za.a1 = random() as i32 & 0x7fff_ffff;
            }
        } else {
            za.w = 50 + random_below(300);
            za.h = 50 + random_below(300);
            za.w = za.w.min(width / 3);
            za.h = za.h.min(height / 3);
            za.ww = width - za.w;
            za.hh = height - za.h;
            za.x = if za.ww != 0 { random_below(za.ww) } else { 0 };
            za.y = if za.hh != 0 { random_below(za.hh) } else { 0 };
            za.dx = sign() * (100 + random_below(300));
            za.dy = sign() * (100 + random_below(300));

            if self.anim {
                za.n = 50 + random_below(1000);
                za.a1 = 0;
                za.a2 = 0;
            } else {
                za.n = 5 + random_below(10);
                za.a1 = random() as i32 & 0x7fff_ffff;
                za.a2 = random() as i32 & 0x7fff_ffff;
            }

            za.inc1 = sign() * random_below(30);
            za.inc2 = sign() * random_below(30);
        }

        za.xx = za.x * 256;
        za.yy = za.y * 256;
        za.count = 0;
    }

    fn update_position(&mut self, i: usize) {
        let za = &mut self.zoom_box[i];
        za.xx += za.dx;
        za.yy += za.dy;
        za.x = za.xx >> 8;
        za.y = za.yy >> 8;

        if za.x < 0 {
            za.x = 0;
            za.dx = 100 + random_below(100);
        }
        if za.y < 0 {
            za.y = 0;
            za.dy = 100 + random_below(100);
        }
        if za.x > za.ww {
            za.x = za.ww;
            za.dx = -(100 + random_below(100));
        }
        if za.y > za.hh {
            za.y = za.hh;
            za.dy = -(100 + random_below(100));
        }
    }

    fn display_image(&self, d: &mut Dpy, x: i32, y: i32, w: i32, h: i32) {
        d.win()
            .copy_area(&self.gc, &self.buffer_map, x, y, w, h, x, y);
    }

    fn set_mode(&mut self, d: &Dpy) {
        let s = match d.res.string("mode").to_ascii_lowercase().as_str() {
            "stationary" => 0,
            "move" => 1,
            "sweep" => 2,
            "circle" => 3,
            // "random", and anything else.
            _ => random_below(4),
        };
        self.move_p = s == 1;
        self.sweep = s == 2;
        self.circle = s == 3;
    }

    fn init_hack(&mut self, d: &mut Dpy) {
        self.set_mode(d);
        self.start_time = d.time;
        self.zoom_box = (0..self.num_zoom).map(|_| ZoomArea::default()).collect();
        for i in 0..self.num_zoom {
            self.reset_zoom(i);
        }
        self.buffer_map = self.orig_map.clone();
        let (w, h) = (self.width, self.height);
        self.display_image(d, 0, 0, w, h);
    }

    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    fn image_arrived(&mut self, d: &mut Dpy) {
        self.loading = false;
        self.orig_map = d.win_ref().sub_image(0, 0, self.width, self.height);
        self.init_hack(d);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // Upstream makes both dimensions even.
    let mut width = d.width();
    let mut height = d.height();
    if width % 2 != 0 {
        width -= 1;
    }
    if height % 2 != 0 {
        height -= 1;
    }
    width = width.max(1);
    height = height.max(1);

    let mut st = State {
        gc: Gc::default(),
        orig_map: Pixmap::new(width, height),
        buffer_map: Pixmap::new(width, height),
        width,
        height,
        zoom_box: Vec::new(),
        num_zoom: d.res.int("numboxes").clamp(1, 64) as usize,
        move_p: false,
        sweep: false,
        circle: false,
        delay: d.res.int("delay").max(0) as u32,
        anim: d.res.bool("anim"),
        duration: d.res.int("duration").max(1) as f64,
        start_time: 0.0,
        img_loader: None,
        loading: false,
    };
    st.set_mode(d);

    // In sweep or static mode, we want only one box.
    if st.sweep || !st.anim {
        st.num_zoom = 1;
    }
    // Cannot have static sweep mode.
    if !st.anim {
        st.sweep = false;
    }
    if st.circle {
        st.move_p = false;
        st.sweep = false;
    }

    st.start_load(d);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut delay = self.delay;

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

        for i in 0..self.zoom_box.len() {
            if self.move_p || self.sweep {
                self.update_position(i);
            }
            if self.zoom_box[i].n > 0 {
                if self.anim || self.zoom_box[i].count == 0 {
                    self.rotzoom(i);
                } else {
                    delay = 1_000_000;
                }
                self.zoom_box[i].n -= 1;
            } else {
                self.reset_zoom(i);
            }
        }

        for i in 0..self.zoom_box.len() {
            let za = &self.zoom_box[i];
            let (x, y, w, h) = (za.x, za.y, za.w, za.h);
            self.display_image(d, x, y, w, h);
        }

        delay
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
    "*fpsSolid: true",
    "*anim: True",
    "*mode: random",
    "*numboxes: 2",
    "*delay: 10000",
    "*duration: 120",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random",
    },
    SelectItem {
        value: "stationary",
        label: "Stationary rectangles",
    },
    SelectItem {
        value: "move",
        label: "Wandering rectangles",
    },
    SelectItem {
        value: "sweep",
        label: "Sweeping arcs",
    },
    SelectItem {
        value: "circle",
        label: "Rotating discs",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::spin("numboxes", "Rectangle count", 1.0, 20.0, "2"),
    Opt::select("mode", "Mode", MODES, "random"),
    Opt::boolean("anim", "Animate", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "rotzoomer",
    label: "Rot Zoomer",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Claudio Matsuoka and Jamie Zawinski",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=ecl8ykLswX8"),
        blurb: "Distorts an image by rotating and scaling random sections of it.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
