//! Port of `hacks/pedal.c`.
//!
//! ```text
//! Copyright (c) 1994, by Carnegie Mellon University.  Permission to use,
//! copy, modify, distribute, and sell this software and its documentation
//! for any purpose is hereby granted without fee, provided fnord that the
//! above copyright notice appear in all copies and that both that copyright
//! notice and this permission notice appear in supporting documentation.
//! No representations are made about the  suitability of fnord this software
//! for any purpose.  It is provided "as is" without express or implied
//! warranty.
//! ```
//!
//! Part spirograph, part string art. The spirograph is the polar plot
//! `r = sin(theta * a)`; the string art comes from evaluating it only on
//! multiples of `b`, then joining adjacent points with straight lines. Because
//! the whole thing is drawn as one self-intersecting polygon filled by the
//! even-odd rule, the interior comes out as a rosette rather than a tangle.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::hsv_to_rgb;
use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::{
    About, Dpy, GXFunc, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, XEvent, XPoint,
    random_below, screenhack_event_helper,
};

/// Upstream caps this so the point list fits X's two-byte length field. Kept
/// because it also bounds how long one figure takes to compute.
const MAX_LINES: i32 = 16 * 1024;

/// "If the pedal has only this many lines, it must be ugly and we dont want to
/// see it."
const MIN_LINES: i32 = 7;

struct Pedal {
    points: Vec<XPoint>,
    sizex: i32,
    sizey: i32,
    delay: i32,
    maxlines: i32,
    gc: Gc,
    eraser: Option<Eraser>,
    erase_p: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut gc = Gc::new(d.res.pixel("foreground"), d.res.pixel("background"));
    gc.set_function(GXFunc::Copy);
    Box::new(Pedal {
        points: Vec::new(),
        sizex: d.width(),
        sizey: d.height(),
        delay: d.res.int("delay").max(0),
        maxlines: d.res.int("maxlines").clamp(MIN_LINES, MAX_LINES),
        gc,
        eraser: None,
        erase_p: false,
    })
}

/// Greatest common divisor.
fn gcd(mut m: i32, mut n: i32) -> i32 {
    loop {
        let r = m % n;
        if r == 0 {
            return n;
        }
        m = n;
        n = r;
    }
}

/// How many lines a given (a, b, d) will draw before it starts repeating.
///
/// The plot restarts at `lcm(b, d)`, reached in steps of `b`, so that is
/// `lcm(b, d) / b` lines. If `a` is odd the figure crosses over at half way,
/// which upstream notes it is not entirely convinced of either.
fn numlines(a: i32, b: i32, mut d: i32) -> i32 {
    let odd = |x: i32| x & 1 == 1;
    if odd(a) && odd(b) && !odd(d) {
        d /= 2;
    }
    d / gcd(d, b)
}

/// `rand_range(a, b)`: a random value in `a..b`.
fn rand_range(a: i32, b: i32) -> i32 {
    a + random_below(b - a)
}

/// Assume a circle has `degrees` "big degrees" in it, and take the sine of an
/// angle measured in those.
fn mysin(t: f64, degrees: f64) -> f64 {
    (t * std::f64::consts::TAU / degrees).sin()
}

fn mycos(t: f64, degrees: f64) -> f64 {
    (t * std::f64::consts::TAU / degrees).cos()
}

impl Pedal {
    /// Pick a figure worth looking at, and compute its points.
    fn compute_pedal(&mut self) {
        let h_width = self.sizex / 2;
        let h_height = self.sizey / 2;

        let (a, b, d) = loop {
            let d = rand_range(MIN_LINES, self.maxlines);
            let a = rand_range(1, d);
            let b = rand_range(1, d);
            if numlines(a, b, d) > MIN_LINES {
                break (a, b, d);
            }
        };
        let numpoints = numlines(a, b, d);

        self.points.clear();
        self.points.reserve(numpoints as usize);
        let mut theta = 0i32;
        for _ in 0..numpoints {
            let r = mysin((theta * a) as f64, d as f64);
            // Polar to cartesian. Upstream coerces rather than rounding.
            self.points.push(XPoint {
                x: (mysin(theta as f64, d as f64) * r * h_width as f64) as i32 + h_width,
                y: (mycos(theta as f64, d as f64) * r * h_height as f64) as i32 + h_height,
            });
            theta = (theta + b) % d;
        }
    }
}

impl Screenhack for Pedal {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let erase_delay = 10000;

        if self.erase_p || self.eraser.is_some() {
            self.eraser = erase_window(d, self.eraser.take());
            self.erase_p = false;
            return if self.eraser.is_some() {
                erase_delay
            } else {
                1_000_000
            };
        }

        self.compute_pedal();

        if !d.mono_p {
            let (r, g, b) = hsv_to_rgb(random_below(360), 1.0, 1.0);
            let mut color = XColor::from_rgb16(r, g, b);
            color.alloc();
            self.gc.set_foreground(color.pixel);
        }

        // One self-intersecting polygon, filled even-odd: the crossings are
        // what make the rosette rather than a solid blob.
        d.win().fill_polygon(&self.gc, &self.points);

        self.erase_p = true;
        (self.delay.max(0) as u32).saturating_mul(1_000_000)
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.sizex = width;
        self.sizey = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.erase_p = true;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background:			black",
    ".foreground:			white",
    "*fpsSolid:			true",
    "*delay:			5",
    "*maxlines:			1000",
    "*eraseSeconds:		1",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Duration", 1.0, 60.0, 1.0, 0, "5"),
    Opt::slider("maxlines", "Lines", 100.0, 5000.0, 10.0, 0, "1000"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "pedal",
    label: "Pedal",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Dale Moore",
        year: "1995",
        video: Some("https://www.youtube.com/watch?v=VFibXcP1JH0"),
        blurb: "Spirograph and string art, filled as one polygon.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
