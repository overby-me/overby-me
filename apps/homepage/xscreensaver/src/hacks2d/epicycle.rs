//! Port of `hacks/epicycle.c`.
//!
//! ```text
//! epicycle --- The motion of a body with epicycles, as in the pre-Copernican
//! cosmologies.
//!
//! Copyright (c) 1998  James Youngman <jay@gnu.org>
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
//! A wheel turning on the rim of a wheel turning on the rim of a wheel, and a
//! pen on the last one. Hipparchus perfected the geometry around 125 B.C. and
//! Ptolemy used it to explain the whole known universe; it survived until
//! Kepler noticed the orbits were ellipses.
//!
//! The figure closes because the speeds are not free. Each wheel turns at the
//! fundamental rate divided by a small whole number, so the whole system
//! returns to its starting position after the lowest common multiple of those
//! divisors, and that multiple is exactly how long the hack draws before
//! starting a new figure. Some of the divisors are negative, which is a wheel
//! turning backwards, and that is where the cusps and the flower shapes come
//! from.
//!
//! Before drawing anything, the figure is traced once to find its extent and
//! the radii are scaled so it fits the window. On the very first figure that
//! trace runs over a period of zero, because the period is only known after
//! the divisors have been chosen; upstream leaves a question mark in the
//! source about it, and the first figure of a run is the one that may not fit.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, frand, random,
    screenhack_event_helper,
};

/// Smallest allowable circle radius.
const MIN_RADIUS: i64 = 5;
/// Proportion of screen to fill by scaling.
const FILL_PROPORTION: f64 = 0.9;
/// Radians in a circle.
const FULLCIRCLE: f64 = std::f64::consts::TAU;

/// Each circle is centred on a point on the rim of another circle.
#[derive(Clone, Copy, Default)]
struct Circle {
    radius: i64,
    /// Position, radians anticlockwise from the x-axis.
    w: f64,
    initial_w: f64,
    /// Rotation rate: the change in `w` per iteration.
    wdot: f64,
    divisor: i32,
}

/// A body that moves on a system of circles.
#[derive(Default)]
struct Body {
    x_origin: i32,
    y_origin: i32,
    x: i32,
    y: i32,
    old_x: i32,
    old_y: i32,
    /// An index into the colormap.
    current_color: usize,
    /// The system of circles the body moves on, outermost first.
    epicycles: Vec<Circle>,
}

/// Determine the GCD of two numbers using Euclid's method. See Knuth,
/// section 4.5.2.
fn gcd(u: i64, v: i64) -> i64 {
    let (mut u, mut v) = (u.abs(), v.abs());
    while v != 0 {
        let r = u % v;
        u = v;
        v = r;
    }
    u
}

/// The lowest common multiple, using Euclid's Proposition 34.
fn lcm(u: i64, v: i64) -> i64 {
    let g = gcd(u, v);
    if g == 0 { 0 } else { u / g * v }
}

struct State {
    gc: Gc,
    width: i32,
    height: i32,
    x_offset: i32,
    y_offset: i32,
    unit_pixels: i32,
    restart: bool,
    wdot_max: f64,
    colors: Vec<XColor>,
    ncolors: usize,
    mono: bool,
    color_shift_pos: usize,
    colour_cycle_rate: f64,
    harmonics: i32,
    divisor_poisson: f64,
    size_factor_min: f64,
    size_factor_max: f64,
    min_circles: i32,
    max_circles: i32,

    l: i64,
    t: f64,
    timestep: f64,
    timestep_coarse: f64,
    delay: u32,
    uncleared: bool,
    holdtime: u32,
    body: Body,
    xtime: f64,
    eraser: Option<Eraser>,
}

impl State {
    fn random_radius(&self, scale: f64) -> i64 {
        let r = (frand(scale) * self.unit_pixels as f64 / 2.0) as i64;
        r.max(MIN_RADIUS)
    }

    /// A small whole number, sometimes negative. Bigger ones get rarer, which
    /// is what keeps the figures from being uniformly frantic.
    fn random_divisor(&self) -> i32 {
        let mut divisor = 1;
        while frand(1.0) < self.divisor_poisson && divisor <= self.harmonics {
            divisor += 1;
        }
        let sign = if frand(1.0) < 0.5 { 1 } else { -1 };
        sign * divisor
    }

    fn new_circle(&self, scale: f64) -> Circle {
        let divisor = self.random_divisor();
        Circle {
            radius: self.random_radius(scale),
            w: 0.0,
            initial_w: 0.0,
            divisor,
            wdot: self.wdot_max / divisor as f64,
        }
    }

    fn new_circle_chain(&self) -> Vec<Circle> {
        // Parent circles are larger than their children by a factor of at
        // least the minimum and at most the maximum.
        let factor = self.size_factor_min + frand(self.size_factor_max - self.size_factor_min);

        let n = if self.max_circles == self.min_circles {
            self.min_circles // Avoid division by zero.
        } else {
            self.min_circles + (random() % (self.max_circles - self.min_circles) as u32) as i32
        };

        let mut scale = 1.0;
        let mut head: Vec<Circle> = Vec::new();
        for _ in 0..n.max(0) {
            head.insert(0, self.new_circle(scale));
            scale /= factor;
        }
        head
    }

    fn new_body(&self) -> Body {
        let mut b = Body {
            epicycles: self.new_circle_chain(),
            ..Body::default()
        };
        // Start all the epicycles at the same w value to make it easier to
        // figure out at what T value the cycle is closed. We do not just fix
        // the initial W value because that makes all the patterns tend to be
        // symmetrical about the X axis.
        let w_common = frand(FULLCIRCLE);
        for c in &mut b.epicycles {
            c.initial_w = w_common;
        }
        b
    }

    /// Calculate the position for the body at time `t`. We work in floating
    /// point rather than integers to avoid the cumulative errors that would be
    /// caused by the rounding implicit in an assignment to int.
    fn move_body(&mut self, t: f64) {
        self.body.old_x = self.body.x;
        self.body.old_y = self.body.y;

        let mut x = self.body.x_origin as f64;
        let mut y = self.body.y_origin as f64;

        for p in &mut self.body.epicycles {
            // Angular position is the initial position plus time times
            // angular speed, modulo a full circle.
            p.w = (p.initial_w + t * p.wdot) % FULLCIRCLE;
            x += p.radius as f64 * p.w.cos();
            y += p.radius as f64 * p.w.sin();
        }

        self.body.x = x as i32;
        self.body.y = y as i32;
    }

    fn compute_divisor_lcm(&self) -> i64 {
        let mut l = 1;
        for p in &self.body.epicycles {
            l = lcm(l, p.divisor as i64);
        }
        l
    }

    /// Trace the whole figure once to find out how big it is.
    fn precalculate_figure(&mut self, this_xtime: f64, step: f64) -> (i32, i32, i32, i32) {
        // Move once to avoid an initial line from the origin.
        self.move_body(0.0);
        let (mut x_min, mut x_max) = (self.body.x, self.body.x);
        let (mut y_min, mut y_max) = (self.body.y, self.body.y);

        let mut t = 0.0;
        while t < this_xtime {
            self.move_body(t);
            x_max = x_max.max(self.body.x);
            x_min = x_min.min(self.body.x);
            y_max = y_max.max(self.body.y);
            y_min = y_min.min(self.body.y);
            t += step;
        }
        (x_max, y_max, x_min, y_min)
    }

    fn rescale_circles(&mut self, x_max: i32, y_max: i32, x_min: i32, y_min: i32) {
        let x_max = (x_max - self.x_offset).max(-(x_min - self.x_offset));
        let y_max = (y_max - self.y_offset).max(-(y_min - self.y_offset));

        let xm = self.width as f64 / 2.0;
        let ym = self.height as f64 / 2.0;
        let xscale = if x_max as f64 > xm {
            xm / x_max as f64
        } else {
            1.0
        };
        let yscale = if y_max as f64 > ym {
            ym / y_max as f64
        } else {
            1.0
        };

        // Whichever axis is tighter is the one that has to fit.
        let mut scale = xscale.min(yscale);

        // Only fill a proportion of the screen, and only reduce, never
        // enlarge.
        scale *= FILL_PROPORTION;
        if scale < 1.0 {
            for p in &mut self.body.epicycles {
                p.radius = (p.radius as f64 * scale) as i64;
            }
        }

        // Window has a weird aspect.
        if self.width > self.height * 5 || self.height > self.width * 5 {
            let r = if self.width > self.height {
                self.width as f64 / self.height as f64
            } else {
                self.height as f64 / self.width as f64
            };
            for p in &mut self.body.epicycles {
                p.radius = (p.radius as f64 * r) as i64;
            }
        }
    }

    /// Angular speeds of the circles are harmonics of a fundamental value.
    /// That should please the Pythagoreans among you.
    fn random_wdot_max(&self, minspeed: f64, maxspeed: f64) -> f64 {
        self.harmonics as f64 * (minspeed + FULLCIRCLE * frand(maxspeed - minspeed))
    }

    fn color_step(&mut self, frac: f64) {
        if self.mono || self.colors.is_empty() {
            return;
        }
        let newshift =
            (self.ncolors as f64 * (frac * self.colour_cycle_rate).rem_euclid(1.0)) as usize;
        if newshift != self.color_shift_pos {
            self.body.current_color = newshift.min(self.colors.len() - 1);
            self.gc
                .set_foreground(self.colors[self.body.current_color].pixel);
            self.color_shift_pos = newshift;
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.width(), d.height());
    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");

    let mut line_width = d.res.int("lineWidth").max(1);
    if width > 2560 || height > 2560 {
        line_width *= 3; // Retina displays.
    }
    let mut gc = Gc::new(fg, bg);
    gc.line_width = line_width;

    let mut ncolors = d.res.int("colors").max(2) as usize;
    let mut mono = d.mono_p || ncolors <= 2;
    let colors = if mono {
        Vec::new()
    } else {
        let c = make_smooth_colormap(ncolors);
        if c.len() <= 2 {
            mono = true;
            ncolors = 0;
            Vec::new()
        } else {
            ncolors = c.len();
            c
        }
    };

    let timestep = d.res.float("timestep");
    let mut st = State {
        gc,
        width,
        height,
        x_offset: width / 2,
        y_offset: height / 2,
        unit_pixels: width.min(height),
        restart: true,
        wdot_max: 0.0,
        colors,
        ncolors,
        mono,
        color_shift_pos: 0,
        colour_cycle_rate: 0.0,
        harmonics: d.res.int("harmonics").max(1),
        divisor_poisson: d.res.float("divisorPoisson"),
        size_factor_min: d.res.float("sizeFactorMin"),
        size_factor_max: d.res.float("sizeFactorMax"),
        min_circles: d.res.int("minCircles").max(1),
        max_circles: d.res.int("maxCircles").max(1),
        l: 1,
        t: 0.0,
        timestep,
        timestep_coarse: timestep * d.res.float("timestepCoarseFactor"),
        delay: d.res.int("delay").max(0) as u32,
        uncleared: false,
        holdtime: d.res.int("holdtime").max(0) as u32,
        body: Body::default(),
        // Upstream leaves a question mark next to this: the first figure is
        // measured over a period of zero, so it is the one that may not fit.
        xtime: 0.0,
        eraser: None,
    };
    if st.max_circles < st.min_circles {
        st.max_circles = st.min_circles;
    }
    d.clear_window();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut this_delay = self.delay;

        if self.eraser.is_some() {
            self.eraser = erase_window(d, self.eraser.take());
            return 10000;
        }

        if self.restart {
            self.width = d.width();
            self.height = d.height();
            self.x_offset = self.width / 2;
            self.y_offset = self.height / 2;
            self.unit_pixels = self.width.min(self.height);
            self.restart = false;

            self.wdot_max = self.random_wdot_max(d.res.float("minSpeed"), d.res.float("maxSpeed"));

            let body = self.new_body();
            self.body = body;
            self.body.x_origin = self.x_offset;
            self.body.x = self.x_offset;
            self.body.y_origin = self.y_offset;
            self.body.y = self.y_offset;

            if self.uncleared {
                self.eraser = erase_window(d, self.eraser.take());
                self.uncleared = false;
            }

            let (xmax, ymax, xmin, ymin) =
                self.precalculate_figure(self.xtime, self.timestep_coarse);
            self.rescale_circles(xmax, ymax, xmin, ymin);

            // Move twice to avoid an initial line from the origin.
            self.move_body(0.0);
            self.move_body(0.0);

            self.t = 0.0;
            self.l = self.compute_divisor_lcm();
            self.colour_cycle_rate = self.l.abs() as f64;
            self.xtime = (self.l as f64 * FULLCIRCLE / self.wdot_max).abs();

            if !self.colors.is_empty() {
                let p = self.colors[self.body.current_color.min(self.colors.len() - 1)].pixel;
                self.gc.set_foreground(p);
            }
        }

        let frac = self.t / self.xtime;
        self.color_step(frac);
        let (ox, oy, x, y) = (self.body.old_x, self.body.old_y, self.body.x, self.body.y);
        d.win().draw_line(&self.gc, ox, oy, x, y);
        self.uncleared = true;

        // Check if the figure is complete.
        if self.t > self.xtime {
            this_delay = self.holdtime * 1_000_000;
            self.restart = true; // Begin a new figure.
        }

        self.t += self.timestep;
        let t = self.t;
        self.move_body(t);

        this_delay
    }

    fn reshape(&mut self, _d: &mut Dpy, _width: i32, _height: i32) {
        self.restart = true;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.restart = true;
            return true;
        }
        false
    }
}

/// Some of these resource values are hand-tuned to give a pleasing variety of
/// interesting shapes. These are not the only good settings, but you may find
/// you need to change some as a group to get pleasing figures.
const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*colors: 100",
    "*color0: red",
    "*delay: 20000",
    "*holdtime: 2",
    "*lineWidth: 4",
    "*minCircles: 2",
    "*maxCircles: 10",
    "*minSpeed: 0.003",
    "*maxSpeed: 0.005",
    "*harmonics: 8",
    "*timestep: 1.0",
    // No option for this resource.
    "*timestepCoarseFactor: 1.0",
    "*divisorPoisson: 0.4",
    "*sizeFactorMin: 1.05",
    "*sizeFactorMax: 2.05",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("colors", "Number of colors", 1.0, 255.0, 1.0, 0, "100"),
    Opt::slider("holdtime", "Linger", 1.0, 30.0, 1.0, 0, "2"),
    Opt::spin("lineWidth", "Line thickness", 1.0, 50.0, "4"),
    Opt::spin("harmonics", "Harmonics", 1.0, 20.0, "8"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "epicycle",
    label: "Epicycle",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "James Youngman",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=rpk3zxQxaR8"),
        blurb: "A pre-heliocentric model of planetary motion: the path traced by a point on a circle rolling on a circle.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
