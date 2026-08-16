//! Port of `hacks/apollonian.c`.
//!
//! ```text
//! apollonian --- Apollonian Circles
//!
//! Copyright (c) 2000, 2001 by Allan R. Wilks <allan@research.att.com>.
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
//! radius r = 1 / c (curvature)
//!
//! Descartes Circle Theorem: (a, b, c, d are curvatures of tangential circles)
//! Let a, b, c, d be the curvatures of for mutually (externally) tangent
//! circles in the plane.  Then
//! a^2 + b^2 + c^2 + d^2 = (a + b + c + d)^2 / 2
//!
//! Revision History:
//! 25-Jun-2001: Converted from C and Postscript code by David Bagley
//!              Original code by Allan R. Wilks <allan@research.att.com>.
//! ```
//!
//! Three circles that all touch each other leave two gaps, and each gap has
//! exactly one circle that fills it touching all three. Do that to the three
//! new gaps, and to their gaps, forever. Descartes worked out in 1643 that the
//! curvatures satisfy a quadratic, and its second root is the arithmetic that
//! generates the whole packing: given four mutually tangent circles, twice the
//! sum of three of the curvatures minus the fourth is the curvature of the
//! circle filling their gap, and the same holds for the curvature times the
//! centre. So one line of arithmetic and a recursion produce the entire gasket,
//! with no geometry anywhere.
//!
//! Starting configurations come from two places. Four are hand-written,
//! including two that are unbounded, where a circle of curvature zero is a
//! straight line and is drawn as one. The rest are found by search: integer
//! curvature quadruples that satisfy Descartes's equation and cannot be reduced
//! to a smaller one, which is what makes a packing where every circle in it has
//! an integer curvature.
//!
//! Upstream can also write each circle's curvature inside it, and offers
//! spherical and hyperbolic labellings alongside the ordinary one. Neither is
//! here, because both need a font. That is not a gap in the picture: upstream
//! ties the alternate geometries to the labels, so with labels off it draws
//! the euclidean packing too.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, XPoint};

/// 1 + 2/sqrt(3).
const K: f64 = 2.154_700_538_379_251_5;
/// Not configurable by the user: raising it makes the search take too long.
const MAXBEND: i32 = 100;
const BIG: f64 = 7.0;

/// A circle: curvature, and curvature times centre. Upstream also carries a
/// spherical and a hyperbolic curvature for its labels, which are not here.
#[derive(Clone, Copy, Default, Debug)]
struct Circle {
    /// Euclidean bend. The radius is one over this.
    e: f64,
    x: f64,
    y: f64,
}

const fn c(e: f64, x: f64, y: f64) -> Circle {
    Circle { e, x, y }
}

/// ((3 + 2*sqrt(3)) / 3).
const DELTA: f64 = 2.154700538;
/// ((3 + sqrt(5)) / 2).
const ALPHA: f64 = 2.618033989;
/// (phi + sqrt(phi)), where phi is the golden ratio.
const BETA: f64 = 2.890053638;

/// The hand-written starting configurations. The x and y of the last three
/// are computed later from the curvatures.
static EXAMPLES: [[Circle; 4]; 4] = [
    // Double semi-bounded: two of these are straight lines.
    [
        c(0.0, 0.0, 1.0),
        c(0.0, 0.0, -1.0),
        c(1.0, -1.0, 0.0),
        c(1.0, 1.0, 0.0),
    ],
    // Three-fold symmetric bounded.
    [
        c(-1.0, 0.0, 0.0),
        c(DELTA, 1.0, 0.0),
        c(DELTA, 1.0, -1.0),
        c(DELTA, -1.0, 1.0),
    ],
    // Semi-bounded.
    [
        c(1.0, 0.0, 0.0),
        c(0.0, 0.0, -1.0),
        c(1.0 / (ALPHA * ALPHA), -1.0, 0.0),
        c(1.0 / ALPHA, -1.0, 0.0),
    ],
    // Unbounded.
    [
        c(1.0, 0.0, 0.0),
        c(1.0 / (BETA * BETA * BETA), 1.0, 0.0),
        c(1.0 / (BETA * BETA), 1.0, 0.0),
        c(1.0 / BETA, 1.0, 0.0),
    ],
];

const PREDEF_CIRCLE_GAMES: usize = EXAMPLES.len();

#[derive(Clone, Copy, Default)]
struct Quadruple {
    a: i32,
    b: i32,
    c: i32,
    d: i32,
}

fn gcd(mut a: i32, mut b: i32) -> i32 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

/// The integer square root, or minus one if `n` is not a perfect square.
fn isqrt(n: i32) -> i32 {
    if n < 0 {
        return -1;
    }
    let y = ((n as f64).sqrt() + 0.5) as i32;
    if n == y * y { y } else { -1 }
}

/// Search for integer curvature quadruples that satisfy Descartes's equation
/// and cannot be reduced to a smaller one.
fn dquad(n: usize) -> Vec<Quadruple> {
    let mut quad: Vec<Quadruple> = Vec::with_capacity(n);
    'outer: for a in 0..MAXBEND {
        let b_hi = (K * a as f64) as i32;
        for b in (a + 1)..=b_hi {
            let c_hi = (((a + b) * (a + b)) as f64 / (4.0 * (b - a) as f64)) as i32;
            for cc in b..=c_hi {
                let d = isqrt(b * cc - a * (b + cc));
                if d >= 0 && gcd(a, gcd(b, cc)) <= 1 {
                    quad.push(Quadruple {
                        a: -a,
                        b,
                        c: cc,
                        d: -a + b + cc - 2 * d,
                    });
                    if quad.len() >= n {
                        break 'outer;
                    }
                }
            }
        }
    }
    // Found only this many below the maximum bend; pad with the standard one.
    while quad.len() < n {
        quad.push(Quadruple {
            a: -1,
            b: 2,
            c: 2,
            d: 3,
        });
    }
    quad
}

struct State {
    mi: ModeInfo,
    size: i32,
    offset: XPoint,
    c1: Circle,
    c2: Circle,
    c3: Circle,
    c4: Circle,
    color_offset: i32,
    count: usize,
    quad: Vec<Quadruple>,
    time: i32,
    game: usize,
    delay: u32,
    cycles: i32,
}

impl State {
    fn color_for(&self, g: f64) -> crate::runtime::Pixel {
        if self.mi.npixels() <= 2 {
            return self.mi.white;
        }
        let i = ((g + self.color_offset as f64) * g) as i32;
        self.mi.pixel(i.rem_euclid(self.mi.npixels()) as usize)
    }

    /// Draw one circle. A curvature of zero is a straight line, and a negative
    /// one is the circle everything else is packed inside, drawn as an
    /// outline rather than filled.
    fn p(&mut self, d: &mut Dpy, circ: Circle) {
        let g = circ.e;

        if circ.e < 0.0 {
            let g = g.abs();
            let p = self.color_for(g);
            self.mi.gc.set_foreground(p);
            let s = self.size as f64;
            d.win().draw_arc(
                &self.mi.gc,
                (s * (-self.c1.e) * (circ.x - 1.0) / (-2.0 * circ.e)
                    + s / 2.0
                    + self.offset.x as f64) as i32,
                (s * (-self.c1.e) * (circ.y - 1.0) / (-2.0 * circ.e)
                    + s / 2.0
                    + self.offset.y as f64) as i32,
                (self.c1.e * s / circ.e) as i32,
                (self.c1.e * s / circ.e) as i32,
                0,
                FULL_CIRCLE,
            );
            return;
        }

        let p = self.color_for(g);
        self.mi.gc.set_foreground(p);

        if circ.e == 0.0 {
            let s = self.size as f64;
            let (w, h) = (self.mi.width, self.mi.height);
            if circ.x == 0.0 && circ.y != 0.0 {
                let y = ((circ.y + 1.0) * s / 2.0 + self.offset.y as f64) as i32;
                d.win().draw_line(&self.mi.gc, 0, y, w, y);
            } else if circ.y == 0.0 && circ.x != 0.0 {
                let x = ((circ.x + 1.0) * s / 2.0 + self.offset.x as f64) as i32;
                d.win().draw_line(&self.mi.gc, x, 0, x, h);
            }
            return;
        }

        let e = if self.c1.e >= 0.0 { 1.0 } else { -self.c1.e };
        let s = self.size as f64;
        d.win().fill_arc(
            &self.mi.gc,
            (s * e * (circ.x - 1.0) / (2.0 * circ.e) + s / 2.0 + self.offset.x as f64) as i32,
            (s * e * (circ.y - 1.0) / (2.0 * circ.e) + s / 2.0 + self.offset.y as f64) as i32,
            (e * s / circ.e) as i32,
            (e * s / circ.e) as i32,
            0,
            FULL_CIRCLE,
        );
    }

    /// Descartes's second root: twice the sum of three, minus the fourth, for
    /// the curvature and for the curvature times the centre alike. Then do the
    /// same in each of the three new gaps.
    fn f(&mut self, d: &mut Dpy, c1: Circle, c2: Circle, c3: Circle, c4: Circle) {
        let e = if self.c1.e >= 0.0 { 1.0 } else { -self.c1.e };
        let c = Circle {
            e: 2.0 * (c1.e + c2.e + c3.e) - c4.e,
            x: 2.0 * (c1.x + c2.x + c3.x) - c4.x,
            y: 2.0 * (c1.y + c2.y + c3.y) - c4.y,
        };
        if c.e == 0.0
            || c.e > self.size as f64 * e
            || c.x / c.e > BIG
            || c.y / c.e > BIG
            || c.x / c.e < -BIG
            || c.y / c.e < -BIG
        {
            return;
        }
        self.p(d, c);
        self.f(d, c2, c3, c, c1);
        self.f(d, c1, c3, c, c2);
        self.f(d, c1, c2, c, c3);
    }

    /// Reflect a starting configuration, so the same packing is not always
    /// drawn the same way up.
    fn randomize_c(randomize: i32, c: &mut Circle) {
        if randomize / 2 != 0 {
            std::mem::swap(&mut c.x, &mut c.y);
        }
        if randomize % 2 != 0 {
            c.x = -c.x;
            c.y = -c.y;
        }
    }

    fn restart(&mut self, d: &mut Dpy) {
        let (w, h) = (self.mi.width, self.mi.height);
        self.size = (w.min(h) - 1).max(1);
        self.offset = XPoint {
            x: (w - self.size) / 2,
            y: (h - self.size) / 2,
        };
        self.color_offset = nrand(self.mi.npixels());

        self.game = nrand((PREDEF_CIRCLE_GAMES + self.count) as i32) as usize;

        if self.game < PREDEF_CIRCLE_GAMES {
            let g = &EXAMPLES[self.game];
            self.c1 = g[0];
            self.c2 = g[1];
            self.c3 = g[2];
            self.c4 = g[3];
        } else {
            let q = self.quad[self.game - PREDEF_CIRCLE_GAMES];
            self.c1 = c(q.a as f64, 0.0, 0.0);
            self.c2 = c(q.b as f64, 0.0, 0.0);
            self.c3 = c(q.c as f64, 0.0, 0.0);
            self.c4 = c(q.d as f64, 0.0, 0.0);
        }
        self.time = 0;
        d.clear_window();

        if self.game != 0 {
            if self.c1.e == 0.0 || self.c1.e == -self.c2.e {
                return;
            }
            self.c1.x = 0.0;
            self.c1.y = 0.0;
            self.c2.x = -(self.c1.e + self.c2.e) / self.c1.e;
            self.c2.y = 0.0;
            let mut q123 =
                (self.c1.e * self.c2.e + self.c1.e * self.c3.e + self.c2.e * self.c3.e).sqrt();
            self.c3.x =
                (self.c1.e * self.c1.e - q123 * q123) / (self.c1.e * (self.c1.e + self.c2.e));
            self.c3.y = -2.0 * q123 / (self.c1.e + self.c2.e);
            q123 += -self.c1.e - self.c2.e;
            self.c4.x =
                (self.c1.e * self.c1.e - q123 * q123) / (self.c1.e * (self.c1.e + self.c2.e));
            self.c4.y = -2.0 * q123 / (self.c1.e + self.c2.e);
        }

        if lrand() & 1 != 0 {
            self.c3.y = -self.c3.y;
            self.c4.y = -self.c4.y;
        }
        let i = nrand(4);
        Self::randomize_c(i, &mut self.c1);
        Self::randomize_c(i, &mut self.c2);
        Self::randomize_c(i, &mut self.c3);
        Self::randomize_c(i, &mut self.c4);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mi = ModeInfo::new(d, ColorScheme::Random);
    let count = mi.count.unsigned_abs().max(1) as usize;
    let cycles = mi.cycles;
    let delay = mi.delay;
    let mut st = State {
        mi,
        size: 1,
        offset: XPoint::default(),
        c1: Circle::default(),
        c2: Circle::default(),
        c3: Circle::default(),
        c4: Circle::default(),
        color_offset: 0,
        count,
        quad: dquad(count),
        time: 0,
        game: 0,
        delay,
        cycles,
    };
    st.restart(d);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // The four starting circles first, then the three gaps of each of the
        // four ways of choosing three of them.
        if self.time < 5 {
            let (c1, c2, c3, c4) = (self.c1, self.c2, self.c3, self.c4);
            match self.time {
                0 => {
                    self.p(d, c1);
                    self.p(d, c2);
                    self.p(d, c3);
                    self.p(d, c4);
                }
                1 => self.f(d, c1, c2, c3, c4),
                2 => self.f(d, c1, c2, c4, c3),
                3 => self.f(d, c1, c3, c4, c2),
                _ => self.f(d, c2, c3, c4, c1),
            }
        }
        self.time += 1;
        if self.time > self.cycles {
            self.restart(d);
        }
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.restart(d);
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

const DEFAULTS: &[&str] = &[
    "*delay: 1000000",
    "*count: 64",
    "*cycles: 20",
    "*ncolors: 64",
    "*fpsTop: true",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("cycles", "Depth", 1.0, 20.0, 1.0, 0, "20"),
    Opt::slider("ncolors", "Number of colors", 2.0, 255.0, 1.0, 0, "64"),
    Opt::slider("delay", "Speed", 0.0, 1_000_000.0, 10000.0, 0, "1000000").inverted(),
];

pub static DEF: SaverDef = SaverDef {
    slug: "apollonian",
    label: "Apollonian",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Allan R. Wilks and David Bagley",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=aeWnjSROR8U"),
        blurb: "A fractal packing of circles with smaller circles, demonstrating Descartes's theorem.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
