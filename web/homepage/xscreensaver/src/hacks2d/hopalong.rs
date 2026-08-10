//! Port of `hacks/hopalong.c`.
//!
//! ```text
//! hop --- real plane fractals
//!
//! Copyright (c) 1991 by Patrick J. Naughton.
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
//! Revision History:
//! Changes in xlockmore distribution
//! 01-Nov-2000: Allocation checks
//! 24-Jun-1997: EJK and RR functions stolen from xmartin2.2
//!              Ed Kubaitis <ejk@ux2.cso.uiuc.edu> ejk functions and xmartin
//!              Renaldo Recuerdo rr function, generalized exponent version
//!              of the Barry Martin's square root function
//! 10-May-1997: Compatible with xscreensaver
//! 27-Jul-1995: added Peter de Jong's hop from Scientific American
//!              July 87 p. 111.  Sometimes they are amazing but there are a
//!              few duds (I did not see a pattern in the parameters).
//! 29-Mar-1995: changed name from hopalong to hop
//! 09-Dec-1994: added Barry Martin's sine hop
//! Changes in original xlock
//! 29-Oct-1990: fix bad (int) cast.
//! 29-Jul-1990: support for multiple screens.
//! 08-Jul-1990: new timing and colors and new algorithm for fractals.
//! 15-Dec-1989: Fix for proper skipping of {White,Black}Pixel() in colors.
//! 08-Oct-1989: Fixed long standing typo bug in RandomInitHop();
//!              Fixed bug in memory allocation in init_hop();
//!              Moved seconds() to an extern.
//!              Got rid of the % mod since .mod is slow on a sparc.
//! 20-Sep-1989: Lint.
//! 31-Aug-1988: Forked from xlock.c for modularity.
//! 23-Mar-1988: Coded HOPALONG routines from Scientific American Sept. 86 p. 14.
//!              Hopalong was attributed to Barry Martin of Aston University
//!              (Birmingham, England)
//! ```
//!
//! Barry Martin's "hopalong" and its descendants: a two-line recurrence whose
//! orbit, plotted a thousand points at a time, lacy-fills the plane. Eleven
//! variants, differing only in the function applied to the running value, from
//! the original square root through Pickover's popcorn to de Jong's attractor.
//! The colour advances one step per frame, so the picture is laid down in
//! bands as the orbit revisits the same region.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, MAXRAND, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XRectangle};

/// `LRAND() / MAXRAND`: a fraction of the way through the generator's range.
fn unit() -> f64 {
    lrand() as f64 / MAXRAND
}

/// `LRAND() & 1`.
fn coin() -> bool {
    lrand() & 1 == 1
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Martin,
    Ejk1,
    Ejk2,
    Ejk4,
    Ejk5,
    Rr,
    Jong,
    Popcorn,
    Sine,
    Ejk3,
    Ejk6,
}

/// The eleven variants, in the numeric order upstream `#define`s them, which
/// is the order `NRAND(OPS)` picks from.
const OPS: [Op; 11] = [
    Op::Martin,
    Op::Ejk1,
    Op::Ejk2,
    Op::Ejk4,
    Op::Ejk5,
    Op::Rr,
    Op::Jong,
    Op::Popcorn,
    Op::Sine,
    Op::Ejk3,
    Op::Ejk6,
];

/// Which resource name turns each variant on. Upstream reads these too, but
/// then ignores them: xscreensaver's `xlockmore.c` forces `fullrandom` on, so
/// the mode is always chosen at random. The knobs are in the config file, so
/// they are honoured here when one is set.
const OP_NAMES: [(&str, Op); 11] = [
    ("martin", Op::Martin),
    ("popcorn", Op::Popcorn),
    ("ejk1", Op::Ejk1),
    ("ejk2", Op::Ejk2),
    ("ejk3", Op::Ejk3),
    ("ejk4", Op::Ejk4),
    ("ejk5", Op::Ejk5),
    ("ejk6", Op::Ejk6),
    ("rr", Op::Rr),
    ("jong", Op::Jong),
    ("sine", Op::Sine),
];

const HVAL: f64 = 0.05;
const INCVAL: f64 = 50.0;

struct Hop {
    mi: ModeInfo,
    /// Centre of the screen.
    centerx: i32,
    centery: i32,
    /// The variant's free parameters.
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    /// The current point.
    i: f64,
    j: f64,
    inc: i32,
    pix: usize,
    op: Op,
    count: i32,
    scale: i32,
    points: Vec<XRectangle>,
}

impl Hop {
    /// `init_hop`, which upstream also calls again every `cycles` frames to
    /// start a fresh figure.
    fn reset(&mut self, d: &mut Dpy) {
        self.scale = 1;
        if self.mi.width > 2560 || self.mi.height > 2560 {
            self.scale *= 3; // Retina displays.
        }
        self.centerx = self.mi.width / 2;
        self.centery = self.mi.height / 2;

        self.op = match OP_NAMES.iter().find(|(name, _)| d.res.bool(name)) {
            Some((_, op)) => *op,
            None => OPS[nrand(OPS.len() as i32) as usize],
        };

        let range =
            ((self.centerx as f64).powi(2) + (self.centery as f64).powi(2)).sqrt() / (1.0 + unit());
        self.i = 0.0;
        self.j = 0.0;
        self.inc = (unit() * 200.0) as i32 - 100;

        match self.op {
            Op::Martin => {
                self.a = (unit() * 2.0 - 1.0) * range / 20.0;
                self.b = (unit() * 2.0 - 1.0) * range / 20.0;
                self.c = if coin() {
                    (unit() * 2.0 - 1.0) * range / 20.0
                } else {
                    0.0
                };
            }
            Op::Ejk1 => {
                self.a = (unit() * 2.0 - 1.0) * range / 30.0;
                self.c = (unit() * 2.0 - 1.0) * range / 40.0;
                self.b = unit() * 0.4;
            }
            Op::Ejk2 => {
                self.a = (unit() * 2.0 - 1.0) * range / 30.0;
                self.b = 10f64.powf(6.0 + unit() * 24.0);
                if coin() {
                    self.b = -self.b;
                }
                self.c = 10f64.powf(unit() * 9.0);
                if coin() {
                    self.c = -self.c;
                }
            }
            Op::Ejk3 => {
                self.a = (unit() * 2.0 - 1.0) * range / 30.0;
                self.c = (unit() * 2.0 - 1.0) * range / 70.0;
                self.b = unit() * 0.35 + 0.5;
            }
            Op::Ejk4 => {
                self.a = (unit() * 2.0 - 1.0) * range / 2.0;
                self.c = (unit() * 2.0 - 1.0) * range / 200.0;
                self.b = unit() * 9.0 + 1.0;
            }
            Op::Ejk5 => {
                self.a = (unit() * 2.0 - 1.0) * range / 2.0;
                self.c = (unit() * 2.0 - 1.0) * range / 200.0;
                self.b = unit() * 0.3 + 0.1;
            }
            Op::Ejk6 => {
                self.a = (unit() * 2.0 - 1.0) * range / 30.0;
                self.b = unit() + 0.5;
            }
            Op::Rr => {
                self.a = (unit() * 2.0 - 1.0) * range / 40.0;
                self.b = (unit() * 2.0 - 1.0) * range / 200.0;
                self.c = (unit() * 2.0 - 1.0) * range / 20.0;
                self.d = unit() * 0.9;
            }
            Op::Popcorn => {
                self.a = 0.0;
                self.b = 0.0;
                self.c = (unit() * 2.0 - 1.0) * 0.24 + 0.25;
                self.inc = 100;
            }
            Op::Jong => {
                self.a = (unit() * 2.0 - 1.0) * std::f64::consts::PI;
                self.b = (unit() * 2.0 - 1.0) * std::f64::consts::PI;
                self.c = (unit() * 2.0 - 1.0) * std::f64::consts::PI;
                self.d = (unit() * 2.0 - 1.0) * std::f64::consts::PI;
            }
            Op::Sine => {
                self.a = std::f64::consts::PI + (unit() * 2.0 - 1.0) * 0.7;
            }
        }

        if self.mi.npixels() > 2 {
            self.pix = nrand(self.mi.npixels()) as usize;
        }
        let bufsize = self.mi.count.clamp(1, 100_000) as usize;
        self.points = vec![XRectangle::default(); bufsize];

        d.clear_window();
        self.mi.gc.set_foreground(self.mi.white);
        self.count = 0;
    }
}

/// An `XRectangle`'s coordinates go over the wire as 16-bit values, so a point
/// that runs away wraps rather than vanishing, and the wrapped copies are part
/// of what these figures look like.
fn wrap(v: f64) -> i32 {
    let n = if v.is_nan() {
        0
    } else {
        v.clamp(-1.0e9, 1.0e9) as i32
    };
    n as i16 as i32
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut st = Hop {
        mi,
        centerx: 0,
        centery: 0,
        a: 0.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        i: 0.0,
        j: 0.0,
        inc: 0,
        pix: 0,
        op: Op::Martin,
        count: 0,
        scale: 1,
        points: Vec::new(),
    };
    st.reset(d);
    Box::new(st)
}

impl Screenhack for Hop {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.inc += 1;
        if self.mi.npixels() > 2 {
            let p = self.mi.pixel(self.pix);
            self.mi.gc.set_foreground(p);
            self.pix += 1;
            if self.pix >= self.mi.npixels() as usize {
                self.pix = 0;
            }
        }

        for n in 0..self.points.len() {
            let oldj = self.j;
            let (x, y);
            match self.op {
                Op::Martin => {
                    // SQRT, MARTIN1.
                    let oldi = self.i + self.inc as f64;
                    self.j = self.a - self.i;
                    let s = (self.b * oldi - self.c).abs().sqrt();
                    self.i = oldj + if self.i < 0.0 { s } else { -s };
                    x = self.centerx as f64 + (self.i + self.j);
                    y = self.centery as f64 - (self.i - self.j);
                }
                Op::Ejk1 => {
                    let oldi = self.i + self.inc as f64;
                    self.j = self.a - self.i;
                    let s = self.b * oldi - self.c;
                    self.i = oldj - if self.i > 0.0 { s } else { -s };
                    x = self.centerx as f64 + (self.i + self.j);
                    y = self.centery as f64 - (self.i - self.j);
                }
                Op::Ejk2 => {
                    let oldi = self.i + self.inc as f64;
                    self.j = self.a - self.i;
                    let s = (self.b * oldi - self.c).abs().ln();
                    self.i = oldj - if self.i < 0.0 { s } else { -s };
                    x = self.centerx as f64 + (self.i + self.j);
                    y = self.centery as f64 - (self.i - self.j);
                }
                Op::Ejk3 => {
                    let oldi = self.i + self.inc as f64;
                    self.j = self.a - self.i;
                    self.i = oldj
                        - if self.i > 0.0 {
                            (self.b * oldi).sin() - self.c
                        } else {
                            -(self.b * oldi).sin() - self.c
                        };
                    x = self.centerx as f64 + (self.i + self.j);
                    y = self.centery as f64 - (self.i - self.j);
                }
                Op::Ejk4 => {
                    let oldi = self.i + self.inc as f64;
                    self.j = self.a - self.i;
                    self.i = oldj
                        - if self.i > 0.0 {
                            (self.b * oldi).sin() - self.c
                        } else {
                            -(self.b * oldi - self.c).abs().sqrt()
                        };
                    x = self.centerx as f64 + (self.i + self.j);
                    y = self.centery as f64 - (self.i - self.j);
                }
                Op::Ejk5 => {
                    let oldi = self.i + self.inc as f64;
                    self.j = self.a - self.i;
                    self.i = oldj
                        - if self.i > 0.0 {
                            (self.b * oldi).sin() - self.c
                        } else {
                            -(self.b * oldi - self.c)
                        };
                    x = self.centerx as f64 + (self.i + self.j);
                    y = self.centery as f64 - (self.i - self.j);
                }
                Op::Ejk6 => {
                    let oldi = self.i + self.inc as f64;
                    self.j = self.a - self.i;
                    let t = self.b * oldi;
                    self.i = oldj - (t - t.trunc()).asin();
                    x = self.centerx as f64 + (self.i + self.j);
                    y = self.centery as f64 - (self.i - self.j);
                }
                Op::Rr => {
                    // RR1.
                    let oldi = self.i + self.inc as f64;
                    self.j = self.a - self.i;
                    let s = (self.b * oldi - self.c).abs().powf(self.d);
                    self.i = oldj - if self.i < 0.0 { -s } else { s };
                    x = self.centerx as f64 + (self.i + self.j);
                    y = self.centery as f64 - (self.i - self.j);
                }
                Op::Popcorn => {
                    if self.inc >= 100 {
                        self.inc = 0;
                    }
                    // Upstream tests this inside the point loop, so the frame
                    // that wraps the counter re-seeds once per point rather
                    // than once per frame. That is what draws popcorn's grid.
                    if self.inc == 0 {
                        let olda = self.a;
                        self.a += 1.0;
                        if olda >= INCVAL {
                            self.a = 0.0;
                            let oldb = self.b;
                            self.b += 1.0;
                            if oldb >= INCVAL {
                                self.b = 0.0;
                            }
                        }
                        self.i = (-self.c * INCVAL / 2.0 + self.c * self.a) * std::f64::consts::PI
                            / 180.0;
                        self.j = (-self.c * INCVAL / 2.0 + self.c * self.b) * std::f64::consts::PI
                            / 180.0;
                    }
                    let tempi = self.i - HVAL * (self.j + (3.0 * self.j).tan()).sin();
                    let tempj = self.j - HVAL * (self.i + (3.0 * self.i).tan()).sin();
                    x = self.centerx as f64 + (self.mi.width / 40) as f64 * tempi;
                    y = self.centery as f64 + (self.mi.height / 40) as f64 * tempj;
                    self.i = tempi;
                    self.j = tempj;
                }
                Op::Jong => {
                    let oldi = if self.centerx > 0 {
                        self.i + 4.0 * self.inc as f64 / self.centerx as f64
                    } else {
                        self.i
                    };
                    self.j = (self.c * self.i).sin() - (self.d * self.j).cos();
                    self.i = (self.a * oldj).sin() - (self.b * oldi).cos();
                    x = self.centerx as f64 + self.centerx as f64 * (self.i + self.j) / 4.0;
                    y = self.centery as f64 - self.centery as f64 * (self.i - self.j) / 4.0;
                }
                Op::Sine => {
                    // MARTIN2.
                    let oldi = self.i + self.inc as f64;
                    self.j = self.a - self.i;
                    self.i = oldj - oldi.sin();
                    x = self.centerx as f64 + (self.i + self.j);
                    y = self.centery as f64 - (self.i - self.j);
                }
            }
            self.points[n] = XRectangle {
                x: wrap(x),
                y: wrap(y),
                width: self.scale,
                height: self.scale,
            };
        }

        d.win().fill_rectangles(&self.mi.gc, &self.points);

        self.count += 1;
        if self.count > self.mi.cycles {
            self.reset(d);
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.reset(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 10000",
    "*count: 1000",
    "*cycles: 2500",
    "*ncolors: 200",
    "*fpsSolid: true",
    "*martin: False",
    "*popcorn: False",
    "*ejk1: False",
    "*ejk2: False",
    "*ejk3: False",
    "*ejk4: False",
    "*ejk5: False",
    "*ejk6: False",
    "*rr: False",
    "*jong: False",
    "*sine: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("cycles", "Duration", 0.0, 800_000.0, 100.0, 0, "2500"),
    Opt::slider("count", "Color contrast", 100.0, 10_000.0, 100.0, 0, "1000"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
    Opt::boolean("sine", "Sine", "False"),
    Opt::boolean("martin", "Martin", "False"),
    Opt::boolean("popcorn", "Popcorn", "False"),
    Opt::boolean("jong", "Jong", "False"),
    Opt::boolean("rr", "RR", "False"),
    Opt::boolean("ejk1", "EJK1", "False"),
    Opt::boolean("ejk2", "EJK2", "False"),
    Opt::boolean("ejk3", "EJK3", "False"),
    Opt::boolean("ejk4", "EJK4", "False"),
    Opt::boolean("ejk5", "EJK5", "False"),
    Opt::boolean("ejk6", "EJK6", "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hopalong",
    label: "Hopalong",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Patrick Naughton",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=Ck0pKMflau0"),
        blurb: "Lacy fractal patterns based on iteration in the imaginary plane.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
