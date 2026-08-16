//! Port of `hacks/discrete.c`.
//!
//! ```text
//! Copyright (c) 1996 by Tim Auckland <tda10.geo@yahoo.com>
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! "discrete" shows a number of fractals based on the "discrete map"
//! type of dynamical systems.  They include a different way of looking
//! at the HOPALONG system, an inverse julia-set iteration, the "Standard
//! Map" and the "Bird in a Thornbush" fractal.
//!
//! Revision History:
//! 01-Nov-2000: Allocation checks
//! 31-Jul-1997: Ported to xlockmore-4
//! 08-Aug-1996: Adapted from hop.c Copyright (c) 1991 by Patrick J. Naughton.
//! ```
//!
//! Eight chaotic maps, picked from a weighted table so the prettier ones come
//! up more often. Each frame iterates the chosen map a few thousand times from
//! the point it left off, plotting every step, so the attractor fills in from a
//! thin sketch to a dense figure over a few hundred frames before a new map and
//! new parameters are rolled.
//!
//! Upstream carries two more maps, HSHOE and DELOG, that its weighted table
//! never selects: they are reachable only by editing a `#define TEST` into the
//! source, so they are left out here.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, MAXRAND, ModeInfo, lrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

/// `LRAND() / MAXRAND`: a fraction of the way through the generator's range.
fn unit() -> f64 {
    lrand() as f64 / MAXRAND
}

/// A coin flip, spelled the way upstream spells it.
fn coin() -> bool {
    (lrand() as f64) < MAXRAND / 2.0
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ftype {
    Sqrt,
    Birdie,
    Standard,
    Trig,
    Cubic,
    Henon,
    Ailuj,
}

/// The weighted table the map is drawn from.
const BIAS: [Ftype; 18] = [
    Ftype::Standard,
    Ftype::Standard,
    Ftype::Standard,
    Ftype::Standard,
    Ftype::Sqrt,
    Ftype::Sqrt,
    Ftype::Sqrt,
    Ftype::Sqrt,
    Ftype::Birdie,
    Ftype::Birdie,
    Ftype::Birdie,
    Ftype::Ailuj,
    Ftype::Ailuj,
    Ftype::Ailuj,
    Ftype::Trig,
    Ftype::Trig,
    Ftype::Cubic,
    Ftype::Henon,
];

const MAX_ITER: i32 = 10;

struct Discrete {
    mi: ModeInfo,
    maxx: i32,
    maxy: i32,
    /// The map's free parameters.
    a: f64,
    b: f64,
    c: f64,
    /// The current point.
    i: f64,
    j: f64,
    /// Centre and scale of the projection.
    ic: f64,
    jc: f64,
    is: f64,
    js: f64,
    inc: i32,
    pix: usize,
    op: Ftype,
    count: i32,
    points: Vec<XPoint>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut st = Discrete {
        maxx: mi.width,
        maxy: mi.height,
        mi,
        a: 0.0,
        b: 0.0,
        c: 0.0,
        i: 0.0,
        j: 0.0,
        ic: 0.0,
        jc: 0.0,
        is: 1.0,
        js: 1.0,
        inc: 0,
        pix: 0,
        op: Ftype::Standard,
        count: 0,
        points: Vec::new(),
    };
    st.restart(d);
    Box::new(st)
}

impl Discrete {
    fn restart(&mut self, d: &mut Dpy) {
        self.maxx = self.mi.width;
        self.maxy = self.mi.height;
        self.op = BIAS[(lrand() % BIAS.len() as u32) as usize];

        match self.op {
            Ftype::Henon => {
                self.jc = (unit() * 2.0 - 1.0) * 0.4;
                self.ic = 1.3 * (1.0 - (self.jc * self.jc) / (0.4 * 0.4));
                self.is = self.maxx as f64;
                self.js = self.maxy as f64 * 1.5;
                self.a = 1.0;
                self.b = 1.4;
                self.c = 0.3;
                self.i = 0.0;
                self.j = 0.0;
            }
            Ftype::Sqrt => {
                self.ic = 0.0;
                self.jc = 0.0;
                self.is = 1.0;
                self.js = 1.0;
                let range = ((self.maxx as f64 * 2.0 * (self.maxx as f64 * 2.0))
                    + (self.maxy as f64 * 2.0 * (self.maxy as f64 * 2.0)))
                    .sqrt()
                    / (10.0 + (lrand() % 10) as f64);

                self.a = unit() * range - range / 2.0;
                self.b = unit() * range - range / 2.0;
                self.c = unit() * range - range / 2.0;
                if lrand().is_multiple_of(2) {
                    self.c = 0.0;
                }
                self.i = 0.0;
                self.j = 0.0;
            }
            Ftype::Standard => {
                self.ic = std::f64::consts::PI;
                self.jc = std::f64::consts::PI;
                self.is = self.maxx as f64 / (std::f64::consts::PI * 2.0);
                self.js = self.maxy as f64 / (std::f64::consts::PI * 2.0);
                self.a = 0.0; // decay
                self.b = unit() * 2.0;
                self.c = 0.0;
                self.i = std::f64::consts::PI;
                self.j = std::f64::consts::PI;
            }
            Ftype::Birdie => {
                self.ic = 0.0;
                self.jc = 0.0;
                self.is = self.maxx as f64 / 2.0;
                self.js = self.maxy as f64 / 2.0;
                self.a = 1.99 + (unit() * 2.0 - 1.0) * 0.2;
                self.b = 0.0;
                self.c = 0.8 + (unit() * 2.0 - 1.0) * 0.1;
                self.i = 0.0;
                self.j = 0.0;
            }
            Ftype::Trig => {
                self.a = 5.0;
                self.b = 0.5 + (unit() * 2.0 - 1.0) * 0.3;
                self.ic = self.a;
                self.jc = 0.0;
                self.is = self.maxx as f64 / (self.b * 20.0);
                self.js = self.maxy as f64 / (self.b * 20.0);
                self.i = 0.0;
                self.j = 0.0;
            }
            Ftype::Cubic => {
                self.a = 2.77;
                self.b = 0.1 + (unit() * 2.0 - 1.0) * 0.1;
                self.ic = 0.0;
                self.jc = 0.0;
                self.is = self.maxx as f64 / 4.0;
                self.js = self.maxy as f64 / 4.0;
                self.i = 0.1;
                self.j = 0.1;
            }
            Ftype::Ailuj => {
                self.ic = 0.0;
                self.jc = 0.0;
                self.is = self.maxx as f64 / 4.0;
                // Upstream scales both axes by the width here.
                self.js = self.maxx as f64 / 4.0;
                loop {
                    self.a = (unit() * 2.0 - 1.0) * 1.5 - 0.5;
                    self.b = (unit() * 2.0 - 1.0) * 1.5;
                    let (mut x, mut y) = (0.0f64, 0.0f64);
                    let mut n = 0;
                    // Wait for a connected set: one whose orbit has not escaped
                    // after ten steps.
                    while n < MAX_ITER && x * x + y * y < 13.0 {
                        let xn = x * x - y * y + self.a;
                        let yn = 2.0 * x * y + self.b;
                        x = xn;
                        y = yn;
                        n += 1;
                    }
                    if n >= MAX_ITER {
                        break;
                    }
                }
                self.i = 0.1;
                self.j = 0.1;
            }
        }

        self.pix = 0;
        self.inc = 0;

        let count = self.mi.count.max(1) as usize;
        self.points = vec![XPoint::default(); count];

        d.clear_window();

        let white = self.mi.white;
        self.mi.gc.set_foreground(white);
        self.count = 0;
    }

    fn iterate(&mut self, d: &mut Dpy) {
        let count = self.mi.count.max(1) as usize;
        let cycles = self.mi.cycles;

        self.inc += 1;

        if self.mi.npixels() > 2 {
            let c = self.mi.pixel(self.pix);
            self.mi.gc.set_foreground(c);
            self.pix += 1;
            if self.pix >= self.mi.npixels() as usize {
                self.pix = 0;
            }
        }

        // `while (k--)` runs from count-1 down to zero, and several maps use
        // the last step to jump back to a seed point.
        for slot in 0..count {
            let k = count - 1 - slot;
            let oldj = self.j;
            let oldi = self.i;

            match self.op {
                Ftype::Henon => {
                    self.i = oldj + self.a - self.b * oldi * oldi;
                    self.j = self.c * oldi;
                }
                Ftype::Sqrt => {
                    if k != 0 {
                        self.j = self.a + self.i;
                        self.i = -oldj
                            + if self.i < 0.0 {
                                (self.b * (self.i - self.c)).abs().sqrt()
                            } else {
                                -(self.b * (self.i - self.c)).abs().sqrt()
                            };
                    } else {
                        // The seed alternates sides and walks outwards, which
                        // is what fills the figure in over time.
                        let sign = if self.inc % 2 == 1 { 1.0 } else { -1.0 };
                        self.i =
                            sign * self.inc as f64 * self.maxx as f64 / cycles.max(1) as f64 / 2.0;
                        self.j = self.a + self.i;
                    }
                }
                Ftype::Standard => {
                    if k != 0 {
                        self.j = (1.0 - self.a) * oldj + self.b * oldi.sin() + self.a * self.c;
                        self.j =
                            (self.j + 2.0 * std::f64::consts::PI) % (2.0 * std::f64::consts::PI);
                        self.i = oldi + self.j;
                        self.i =
                            (self.i + 2.0 * std::f64::consts::PI) % (2.0 * std::f64::consts::PI);
                    } else {
                        let sign = if self.inc % 2 == 1 { 1.0 } else { -1.0 };
                        self.j = std::f64::consts::PI
                            + (sign * self.inc as f64 * 2.0 * std::f64::consts::PI
                                / (cycles as f64 - 0.5))
                                % std::f64::consts::PI;
                        self.i = std::f64::consts::PI;
                    }
                }
                Ftype::Birdie => {
                    self.j = oldi;
                    self.i = (1.0 - self.c) * (std::f64::consts::PI * self.a * oldj).cos()
                        + self.c * self.b;
                    // Upstream keeps the previous point in `b`.
                    self.b = oldj;
                }
                Ftype::Trig => {
                    let r2 = oldi * oldi + oldj * oldj;
                    self.i = self.a + self.b * (oldi * r2.cos() - oldj * r2.sin());
                    self.j = self.b * (oldj * r2.cos() + oldi * r2.sin());
                }
                Ftype::Cubic => {
                    self.i = oldj;
                    self.j = self.a * oldj - oldj * oldj * oldj - self.b * oldi;
                }
                Ftype::Ailuj => {
                    let dx = oldi - self.a;
                    let dy = oldj - self.b;
                    self.i = if coin() { -1.0 } else { 1.0 }
                        * ((dx + (dx * dx + dy * dy).sqrt()) / 2.0).sqrt();
                    if self.i < 0.00000001 && self.i > -0.00000001 {
                        self.i = if self.i > 0.0 {
                            0.00000001
                        } else {
                            -0.00000001
                        };
                    }
                    self.j = dy / (2.0 * self.i);
                }
            }

            // X truncates a point to a short, and several of these maps rely on
            // the wrap to keep a diverging orbit on screen.
            self.points[slot] = XPoint {
                x: (self.maxx / 2).wrapping_add(((self.i - self.ic) * self.is) as i32) as i16
                    as i32,
                y: (self.maxy / 2).wrapping_sub(((self.j - self.jc) * self.js) as i32) as i16
                    as i32,
            };
        }

        let points = std::mem::take(&mut self.points);
        d.win().draw_points(&self.mi.gc, &points);
        self.points = points;
    }
}

impl Screenhack for Discrete {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for _ in 0..10 {
            self.iterate(d);
            self.count += 1;
        }

        if self.count > self.mi.cycles {
            self.restart(d);
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.maxx = width;
        self.maxy = height;
        d.clear_window();
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 20000",
    "*count: 4096",
    "*cycles: 2500",
    "*ncolors: 100",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("cycles", "Timeout", 100.0, 10000.0, 100.0, 0, "2500"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "100"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "discrete",
    label: "Discrete",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tim Auckland",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=l-yIY8vRlHA"),
        blurb: "Discrete map fractals: Hopalong, Julia and others.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
