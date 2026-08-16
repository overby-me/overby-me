//! Port of `hacks/hypercube.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2008 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! This code derived from TI Explorer Lisp code by Joe Keane, Fritz Mueller,
//! and Jamie Zawinski.
//! ```
//!
//! A tesseract, projected from four dimensions to two. Sixteen corners, thirty
//! two edges, and a rotation running in any of the six planes a four
//! dimensional space affords. Each of the eight cubes gets its own edge colour,
//! which is the only way to keep track of what is turning into what. Only the
//! edges whose endpoints actually moved are erased and redrawn, which is what
//! keeps it cheap.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::parse_color;
use crate::runtime::{About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs};

const POINT_COUNT: usize = 16;
const ANGLE_SCALE: f64 = 0.001;

/// Which two corners each edge joins, and which cube it belongs to.
const LINE_TABLE: [(usize, usize, usize); 32] = [
    (0, 1, 0),
    (0, 2, 0),
    (1, 3, 0),
    (2, 3, 0),
    (4, 5, 1),
    (4, 6, 1),
    (5, 7, 1),
    (6, 7, 1),
    (0, 4, 4),
    (0, 8, 4),
    (4, 12, 4),
    (8, 12, 4),
    (1, 5, 5),
    (1, 9, 5),
    (5, 13, 5),
    (9, 13, 5),
    (2, 6, 6),
    (2, 10, 6),
    (6, 14, 6),
    (10, 14, 6),
    (3, 7, 7),
    (3, 11, 7),
    (7, 15, 7),
    (11, 15, 7),
    (8, 9, 2),
    (8, 10, 2),
    (9, 11, 2),
    (10, 11, 2),
    (12, 13, 3),
    (12, 14, 3),
    (13, 15, 3),
    (14, 15, 3),
];

/// The eight edge colours, in the order the resource database names them.
const COLOR_SPECS: [&str; 8] = [
    "magenta", "yellow", "#FF9300", "#FF0093", "green", "#8080FF", "#00D0FF", "#00FFD0",
];

/// The six planes a rotation can run in, as pairs of axes.
const PLANES: [(&str, usize, usize); 6] = [
    ("xy", 0, 1),
    ("xz", 0, 2),
    ("yz", 1, 2),
    ("xw", 0, 3),
    ("yw", 1, 3),
    ("zw", 2, 3),
];

#[derive(Clone, Copy, Default)]
struct PointState {
    old_x: i32,
    old_y: i32,
    new_x: i32,
    new_y: i32,
}

struct Hypercube {
    two_observer_z: f64,
    offset_x: f64,
    offset_y: f64,
    unit_scale: f64,
    delay: u32,
    colors: [Pixel; 8],
    gc: Gc,
    black: Pixel,
    /// The cosine and sine of each plane's per-frame rotation.
    cos_sin: [(f64, f64); 6],
    /// The four reference vectors, each with an x, y, z and w component.
    reference: [[f64; 4]; 4],
    points: [PointState; POINT_COUNT],
    /// Whether anything moved last frame, and whether the user has paused it.
    roted: bool,
    stop: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let observer_z = d.res.float("observer-z").max(1.125);
    let fg = d.res.pixel("foreground");
    let black = d.res.pixel("background");

    let mut colors = [fg; 8];
    for (i, spec) in COLOR_SPECS.iter().enumerate() {
        let key = format!("color{i}");
        let named = d.res.get(&key).map(|s| s.to_string());
        let spec = named.as_deref().unwrap_or(spec);
        colors[i] = parse_color(spec).unwrap_or(fg);
    }

    let mut cos_sin = [(1.0, 0.0); 6];
    for (i, (name, _, _)) in PLANES.iter().enumerate() {
        let a = d.res.float(name) * ANGLE_SCALE;
        cos_sin[i] = (a.cos(), a.sin());
    }

    let mut st = Hypercube {
        two_observer_z: 2.0 * observer_z,
        offset_x: 0.0,
        offset_y: 0.0,
        unit_scale: 1.0,
        delay: d.res.int("delay").max(0) as u32,
        colors,
        gc: Gc::new(fg, black),
        black,
        cos_sin,
        // Start as the identity: a is the x axis, b the y, and so on.
        reference: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        points: [PointState::default(); POINT_COUNT],
        roted: false,
        stop: false,
    };
    let (w, h) = (d.width(), d.height());
    st.set_sizes(w, h);
    Box::new(st)
}

impl Hypercube {
    fn set_sizes(&mut self, width: i32, height: i32) {
        let observer_z = 0.5 * self.two_observer_z;
        let min_dim = width.min(height) as f64;
        let var = (observer_z * observer_z - 1.0).max(0.0).sqrt();
        self.offset_x = 0.5 * (width - 1) as f64;
        self.offset_y = 0.5 * (height - 1) as f64;
        self.unit_scale = 0.4 * min_dim * var;
    }
}

impl Screenhack for Hypercube {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut moved = [false; POINT_COUNT];

        if self.roted {
            // Project each corner: the sign pattern of its index picks which
            // way it lies along each of the four reference vectors.
            for (i, m) in moved.iter_mut().enumerate() {
                let sign = |k: usize| if (i >> (3 - k)) & 1 == 1 { 1.0 } else { -1.0 };
                let mut sum = [0.0f64; 4];
                for (n, s) in sum.iter_mut().enumerate() {
                    // n indexes x, y, z, w.
                    for k in 0..4 {
                        *s += sign(k) * self.reference[k][n];
                    }
                }
                let (sum_x, sum_y, sum_z) = (sum[0], sum[1], sum[2]);

                let mul = self.unit_scale / (self.two_observer_z - sum_z);
                let ps = &mut self.points[i];
                let old_x = ps.new_x;
                let old_y = ps.new_y;
                let new_x = (sum_x * mul + self.offset_x).round() as i32;
                let new_y = (sum_y * mul + self.offset_y).round() as i32;
                ps.old_x = old_x;
                ps.old_y = old_y;
                ps.new_x = new_x;
                ps.new_y = new_y;
                *m = old_x != new_x || old_y != new_y;
            }

            for (ip, iq, col) in LINE_TABLE {
                if !(moved[ip] || moved[iq]) {
                    continue;
                }
                let (sp, sq) = (self.points[ip], self.points[iq]);

                let black = self.black;
                self.gc.set_foreground(black);
                d.win()
                    .draw_line(&self.gc, sp.old_x, sp.old_y, sq.old_x, sq.old_y);

                let c = self.colors[col];
                self.gc.set_foreground(c);
                d.win()
                    .draw_line(&self.gc, sp.new_x, sp.new_y, sq.new_x, sq.new_y);
            }
        }

        self.roted = false;
        let mut this_delay = self.delay;
        if self.stop {
            if this_delay < 10000 {
                this_delay = 10000;
            }
            return this_delay;
        }
        self.roted = true;

        // Turn the reference frame in each plane that has a rotation set.
        for (i, (_, dim0, dim1)) in PLANES.iter().enumerate() {
            let (cos_a, sin_a) = self.cos_sin[i];
            if sin_a == 0.0 {
                continue;
            }
            for r in self.reference.iter_mut() {
                let old_u = r[*dim0];
                let old_v = r[*dim1];
                r[*dim0] = old_u * cos_a + old_v * sin_a;
                r[*dim1] = old_v * cos_a - old_u * sin_a;
            }
        }

        this_delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.set_sizes(width, height);
        d.clear_window();
    }

    fn event(&mut self, _d: &mut Dpy, event: &crate::runtime::XEvent) -> bool {
        // Upstream pauses on the middle button.
        if let crate::runtime::XEvent::ButtonPress { button: 2, .. } = event {
            self.stop = !self.stop;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*observer-z: 3.0",
    "*delay: 10000",
    "*xy: 3",
    "*xz: 5",
    "*yw: 10",
    "*yz: 0",
    "*xw: 0",
    "*zw: 0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("observer-z", "Zoom", 1.125, 10.0, 0.125, 3, "3.0"),
    Opt::slider("xw", "XW rotation", 0.0, 20.0, 1.0, 0, "0"),
    Opt::slider("xy", "XY rotation", 0.0, 20.0, 1.0, 0, "3"),
    Opt::slider("xz", "XZ rotation", 0.0, 20.0, 1.0, 0, "5"),
    Opt::slider("yw", "YW rotation", 0.0, 20.0, 1.0, 0, "10"),
    Opt::slider("yz", "YZ rotation", 0.0, 20.0, 1.0, 0, "0"),
    Opt::slider("zw", "ZW rotation", 0.0, 20.0, 1.0, 0, "0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hypercube",
    label: "Hypercube",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Joe Keane, Fritz Mueller, and Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=tOLzz_D4-0E"),
        blurb: "A four-dimensional cube, projected down to two.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
