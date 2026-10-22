//! Port of `hacks/starfish.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1997-2015 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! A closed spline whose control points sit on rays from the middle, each ray
//! breathing in and out at its own pace, so the outline undulates and turns
//! inside out. Every second control point is pinned at the centre, which is
//! what makes the arms. Each frame fills the region between this outline and
//! the last one with an even-odd rule, so the shape is drawn as a band that
//! leaves a trail of colour behind it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{make_smooth_colormap, make_uniform_colormap};
use crate::runtime::fb::FillRule;
use crate::runtime::spline::Spline;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XColor, XPoint,
    frand, random,
};

/// Fixed-point math, for sub-pixel motion.
const SCALE: i64 = 1000;

/// How many control points to skip between arms, weighted.
const SKIPS: [i32; 10] = [2, 2, 2, 2, 3, 3, 3, 6, 6, 12];

/// Control points per arm, weighted. The last four are dropped for the
/// higher skips, which would otherwise make far too many points.
const SIZES: [i32; 20] = [3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 8, 8, 8, 10, 35];

fn rand_below(n: i64) -> i64 {
    if n <= 1 {
        return 0;
    }
    ((random() & 0x7fff_ffff) as i64) % n
}

fn randsign() -> f64 {
    if random() & 1 == 1 { 1.0 } else { -1.0 }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Pulse,
    Zoom,
}

struct Starfish {
    mode: Mode,
    blob: bool,
    skip: i32,
    /// Position of the midpoint, in fixed point.
    x: i64,
    y: i64,
    /// Angle of rotation, and its velocity and acceleration.
    th: f64,
    rotv: f64,
    rota: f64,
    /// How fast it deforms: radial velocity.
    elasticity: i64,
    rot_max: f64,
    min_r: i64,
    max_r: i64,
    npoints: usize,
    /// One radius per control point. A negative one is shrinking.
    r: Vec<i64>,
    spline: Spline,
    /// The outline drawn last frame, which this frame's band closes against.
    prev: Vec<XPoint>,
}

struct State {
    colors: Vec<XColor>,
    ncolors: usize,
    fg_index: usize,
    gc: Gc,
    delay: u32,
    duration: f64,
    blob: bool,
    thickness: f64,
    rotation: f64,
    ncolors_res: i32,
    start_time: f64,
    width: i32,
    height: i32,
    fish: Starfish,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mode = d.res.string("mode").to_ascii_lowercase();
    let blob = match mode.as_str() {
        "blob" => true,
        "zoom" => false,
        // "random" and anything else.
        _ => random().is_multiple_of(3),
    };

    let mut delay = d.res.int("delay").max(0) as u32;
    if blob {
        delay *= 3;
    }

    let mut st = State {
        colors: Vec::new(),
        ncolors: 2,
        fg_index: 0,
        gc: {
            let mut gc = Gc::new(d.res.pixel("foreground"), d.res.pixel("background"));
            gc.fill_rule = FillRule::EvenOdd;
            gc
        },
        delay,
        duration: d.res.int("duration") as f64,
        blob,
        thickness: d.res.float("thickness"),
        rotation: d.res.float("rotation"),
        ncolors_res: d.res.int("colors"),
        start_time: 0.0,
        width: d.width(),
        height: d.height(),
        fish: Starfish::empty(),
    };
    st.reset(d);
    Box::new(st)
}

impl Starfish {
    fn empty() -> Self {
        Self {
            mode: Mode::Pulse,
            blob: false,
            skip: 2,
            x: 0,
            y: 0,
            th: 0.0,
            rotv: 0.0,
            rota: 0.0,
            elasticity: 0,
            rot_max: 0.0,
            min_r: 0,
            max_r: 0,
            npoints: 0,
            r: Vec::new(),
            spline: Spline::new(0),
            prev: Vec::new(),
        }
    }

    fn make(st: &State, maxx: i32, maxy: i32, size: i32) -> Self {
        let blob = st.blob;
        let mut elasticity = (SCALE as f64 * st.thickness) as i64;
        if elasticity == 0 {
            // Bell curve from zero to fifteen, averaging seven and a half.
            elasticity = rand_below(5 * SCALE) + rand_below(5 * SCALE) + rand_below(5 * SCALE);
        }

        let mut rotv = st.rotation;
        if rotv == -1.0 {
            // Bell curve from zero to twelve degrees, averaging six.
            rotv = frand(4.0) + frand(4.0) + frand(4.0);
        }
        rotv /= 360.0; // Degrees to a fraction of a turn.

        if blob {
            elasticity *= 3;
            rotv *= 3.0;
        }

        let rot_max = rotv * 2.0;
        let rota = 0.0004 + frand(0.0002);

        let mut size = size;
        if random().is_multiple_of(20) {
            size = (size as f64 * (frand(0.35) + frand(0.35) + 0.3)) as i32;
        }

        let skip = SKIPS[(random() % SKIPS.len() as u32) as usize];
        let mode = if random().is_multiple_of(if skip == 2 { 3 } else { 12 }) {
            Mode::Zoom
        } else {
            Mode::Pulse
        };

        let maxx = maxx as i64 * SCALE;
        let maxy = maxy as i64 * SCALE;
        let size = size as i64 * SCALE;

        let nsizes = if skip > 3 {
            SIZES.len() - 4
        } else {
            SIZES.len()
        };
        let npoints = (skip * SIZES[(random() % nsizes as u32) as usize]).max(3) as usize;

        Self {
            mode,
            blob,
            skip,
            x: maxx / 2,
            y: maxy / 2,
            th: frand(std::f64::consts::TAU) * randsign(),
            rotv,
            rota,
            elasticity,
            rot_max,
            min_r: 5 * SCALE,
            max_r: size,
            npoints,
            r: (0..npoints)
                .map(|i| if i as i32 % skip == 0 { 0 } else { size })
                .collect(),
            spline: Spline::new(npoints),
            prev: Vec::new(),
        }
    }

    /// Move every control point along its ray, slowest at the ends of its
    /// travel and fastest in the middle, reversing when it runs out of room.
    fn throb(&mut self) {
        let frac = std::f64::consts::TAU / self.npoints as f64;
        for i in 0..self.npoints {
            let mut r = self.r[i];
            let mut ra = r.abs();
            let th = self.th.abs();

            // Place the control points evenly around the perimeter, shifted
            // by theta.
            let x = self.x + (ra as f64 * (i as f64 * frac + th).cos()) as i64;
            let y = self.y + (ra as f64 * (i as f64 * frac + th).sin()) as i64;
            self.spline.control_x[i] = (x / SCALE) as f64;
            self.spline.control_y[i] = (y / SCALE) as f64;

            if self.mode == Mode::Zoom && (i as i32 % self.skip) == 0 {
                continue;
            }

            let mut elasticity = self.elasticity as f64;
            {
                let span = (self.max_r - self.min_r) as f64;
                let mut ratio = if span != 0.0 { ra as f64 / span } else { 0.0 };
                if ratio > 0.5 {
                    ratio = 1.0 - ratio; // Flip.
                }
                ratio *= 2.0; // Normalize.
                ratio = (ratio * 0.9) + 0.1; // Fudge.
                elasticity *= ratio;
            }
            let elasticity = elasticity as i64;

            ra += if r >= 0 { elasticity } else { -elasticity };
            if (i as i32 % self.skip) == 0 {
                ra += elasticity / 2;
            }
            r = ra * if r >= 0 { 1 } else { -1 };

            // Too long or too short: turn around.
            if (ra > self.max_r && r >= 0) || (ra < self.min_r && r < 0) {
                r = -r;
            }
            self.r[i] = r;
        }
    }

    fn spin(&mut self) {
        let mut th = self.th;
        if th < 0.0 {
            th = -(th + self.rotv);
        } else {
            th += self.rotv;
        }
        if th > std::f64::consts::TAU {
            th -= std::f64::consts::TAU;
        } else if th < 0.0 {
            th += std::f64::consts::TAU;
        }
        self.th = if self.th > 0.0 { th } else { -th };

        self.rotv += self.rota;

        if self.rotv > self.rot_max || self.rotv < -self.rot_max {
            self.rota = -self.rota;
        } else if self.rotv < 0.0 {
            // It stopped: start it going the other way, or keep it going the
            // same way from rest.
            if random() & 1 == 1 {
                self.rotv = 0.0;
                if self.rota < 0.0 {
                    self.rota = -self.rota;
                }
            } else {
                self.rotv = -self.rotv;
                self.rota = -self.rota;
                self.th = -self.th;
            }
        }

        if random().is_multiple_of(120) {
            self.rota = -self.rota;
        }
        if random().is_multiple_of(200) {
            if random() & 1 == 1 {
                self.rota *= 1.2;
            } else {
                self.rota *= 0.8;
            }
        }
    }
}

impl State {
    fn reset(&mut self, d: &mut Dpy) {
        self.ncolors = self.ncolors_res.max(2) as usize;
        // Two thirds smooth, one third uniform.
        self.colors = if !random().is_multiple_of(3) {
            make_smooth_colormap(self.ncolors)
        } else {
            make_uniform_colormap(self.ncolors)
        };
        self.ncolors = self.colors.len().max(1);
        self.fg_index = 0;

        if !self.blob {
            // The window's background becomes the first colour, so what the
            // band has not covered yet is not simply black.
            let c = self.colors[0].pixel;
            self.gc.set_foreground(c);
            let (w, h) = (self.width, self.height);
            d.win().fill_rectangle(&self.gc, 0, 0, w, h);
        }

        self.fish = self.make_window_starfish();
    }

    fn make_window_starfish(&self) -> Starfish {
        let (w, h) = (self.width, self.height);
        let mut size = w.min(h);
        if self.blob {
            size /= 2;
        } else {
            size = (size as f64 * 1.3) as i32;
        }
        if w < 100 || h < 100 {
            // Tiny window.
            size = w.max(h).max(100);
        }
        Starfish::make(self, w, h, size)
    }
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.fish.throb();
        self.fish.spin();

        self.fish.spline.compute_closed();
        if !self.fish.prev.is_empty() {
            // The band between this outline and the last, filled even-odd so
            // the two together bound a ring.
            let mut points = self.fish.spline.points.clone();
            points.extend_from_slice(&self.fish.prev);
            if self.fish.blob {
                d.clear_window();
            }
            d.win().fill_polygon(&self.gc, &points);
        }
        self.fish.prev.clear();
        self.fish.prev.extend_from_slice(&self.fish.spline.points);

        self.fg_index = (self.fg_index + 1) % self.ncolors;
        let c = self.colors[self.fg_index].pixel;
        self.gc.set_foreground(c);

        if self.duration > 0.0 {
            if self.start_time == 0.0 {
                self.start_time = d.time;
            }
            if self.start_time + self.duration < d.time {
                self.start_time = d.time;
                // Every now and then pick new colours; otherwise just build a
                // new starfish with the ones we have.
                if random().is_multiple_of(10) {
                    self.reset(d);
                } else {
                    self.fish = self.make_window_starfish();
                }
            }
        }

        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.reset(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*delay: 10000",
    "*thickness: 0",
    "*rotation: -1",
    "*colors: 200",
    "*duration: 30",
    "*delay2: 5",
    "*mode: random",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random",
    },
    SelectItem {
        value: "zoom",
        label: "Color gradients",
    },
    SelectItem {
        value: "blob",
        label: "Pulsating blob",
    },
];

const OPTS: &[Opt] = &[
    Opt::select("mode", "Mode", MODES, "random"),
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("duration", "Duration", 1.0, 60.0, 1.0, 0, "30"),
    Opt::slider("thickness", "Lines", 0.0, 150.0, 1.0, 0, "0"),
    Opt::slider("colors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "starfish",
    label: "Starfish",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=atwc7IJHuLo"),
        blurb: "Undulating, throbbing, star-like patterns.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
