//! Port of `hacks/spiral.c`.
//!
//! ```text
//! Copyright (c) 1994 by Darrick Brown.
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
//! 01-Nov-2000: Allocation checks
//! 10-May-1997: jwz@jwz.org: turned into a standalone program.
//! 24-Jul-1995: Fix to allow cycles not to have an arbitrary value by
//!              Peter Schmitzberger (schmitz@coma.sbg.ac.at).
//! 06-Mar-1995: Finished cleaning up and final testing.
//! 03-Mar-1995: Cleaned up code.
//! 12-Jul-1994: Written.
//!
//! Low CPU usage mode.
//! Idea based on a graphics demo I saw a *LONG* time ago.
//! ```
//!
//! A ring of dots wandering the screen, leaving a trail of every ring it has
//! been. The trail is a ring buffer of positions, so the oldest ring is erased
//! exactly as the newest is drawn and a fixed length of history hangs in the
//! air. Centre, radius and spin all drift, and each is randomly kicked onto a
//! new course now and then, so the overlapping rings beat against each other
//! into moiré.
//!
//! Upstream's redraw-from-the-trail path is left out: it services
//! `refresh_spiral`, which is compiled only for xlock.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs};

/// Fewest dots in a ring.
const MIN_DOTS: i32 = 1;
/// How wild the course changes are. Upstream likes 4.
const JAGGINESS: i32 = 4;
const SPEED: f64 = 2.0;

#[derive(Clone, Copy, Default)]
struct TrailDot {
    hx: f64,
    hy: f64,
    ha: f64,
    hr: f64,
}

struct Spiral {
    mi: ModeInfo,
    trail: Vec<TrailDot>,
    /// Centre of the ring, in the hack's own 10000-tall coordinate space.
    cx: f64,
    cy: f64,
    angle: f64,
    radius: f64,
    /// Rates of change: radius, angle, centre.
    dr: f64,
    da: f64,
    dx: f64,
    dy: f64,
    /// Set once the trail has wrapped, so there is something to erase.
    erase: bool,
    inc: usize,
    /// Colour index, kept fractional so it advances slower than one per frame.
    colors: f64,
    top: f64,
    right: f64,
    dots: i32,
    nlength: usize,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut st = Spiral {
        mi,
        trail: Vec::new(),
        cx: 0.0,
        cy: 0.0,
        angle: 0.0,
        radius: 0.0,
        dr: 0.0,
        da: 0.0,
        dx: 0.0,
        dy: 0.0,
        erase: false,
        inc: 0,
        colors: 0.0,
        top: 10000.0,
        right: 10000.0,
        dots: MIN_DOTS,
        nlength: 2,
    };
    st.restart(d);
    Box::new(st)
}

impl Spiral {
    /// `TFX`: the hack's own x coordinate, in pixels.
    fn tfx(&self, x: f64) -> i32 {
        ((x / self.right) * self.mi.width as f64) as i32
    }

    /// `TFY`.
    fn tfy(&self, y: f64) -> i32 {
        ((y / self.top) * self.mi.height as f64) as i32
    }

    fn draw_dots(&mut self, d: &mut Dpy, at: usize) {
        let dot = self.trail[at];
        let step = std::f64::consts::TAU / self.dots as f64;
        let mut i = 0.0;
        while i < std::f64::consts::TAU {
            let x = dot.hx + (i + dot.ha).cos() * dot.hr;
            let y = dot.hy + (i + dot.ha).sin() * dot.hr;
            let (px, py) = (self.tfx(x), self.tfy(y));
            d.win().draw_point(&self.mi.gc, px, py);
            i += step;
        }
    }

    fn restart(&mut self, d: &mut Dpy) {
        self.mi.width = d.width();
        self.mi.height = d.height();
        d.clear_window();

        // Two is the real floor, not one: upstream seeds slot zero and then
        // leaves `inc` pointing at slot one.
        self.nlength = self.mi.cycles.max(2) as usize;
        self.trail = vec![TrailDot::default(); self.nlength];

        // Keep the window parameters proportional.
        self.top = 10000.0;
        self.right = self.mi.width as f64 / self.mi.height as f64 * 10000.0;

        self.cx = (5000.0 - nrand(2000) as f64) / 10000.0 * self.right;
        self.cy = 5000.0 - nrand(2000) as f64;
        self.radius = (nrand(200) + 200) as f64;
        self.angle = 0.0;
        self.dx = (10 - nrand(20)) as f64 * SPEED;
        self.dy = (10 - nrand(20)) as f64 * SPEED;
        self.dr = ((nrand(10) + 4) * (1 - (lrand() & 1) as i32 * 2)) as f64;
        self.da = nrand(360) as f64 / 7200.0 + 0.01;
        if self.mi.npixels() > 2 {
            self.colors = nrand(self.mi.npixels()) as f64;
        }
        self.erase = false;

        self.inc = 0;
        self.trail[self.inc] = TrailDot {
            hx: self.cx,
            hy: self.cy,
            ha: self.angle,
            hr: self.radius,
        };
        self.inc += 1;

        self.dots = self.mi.count;
        if self.dots < -MIN_DOTS {
            self.dots = nrand(self.dots - MIN_DOTS + 1) + MIN_DOTS;
        }
        // Absolute minimum.
        if self.dots < MIN_DOTS {
            self.dots = MIN_DOTS;
        }
    }
}

impl Screenhack for Spiral {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.erase {
            let black = self.mi.black;
            self.mi.gc.set_foreground(black);
            let inc = self.inc;
            self.draw_dots(d, inc);
        }

        self.cx += self.dx;
        self.trail[self.inc].hx = self.cx;
        if self.cx > 9000.0 || self.cx < 1000.0 {
            self.dx *= -1.0;
        }

        self.cy += self.dy;
        self.trail[self.inc].hy = self.cy;
        if self.cy > 9000.0 || self.cy < 1000.0 {
            self.dy *= -1.0;
        }

        self.radius += self.dr;
        self.trail[self.inc].hr = self.radius;
        if self.radius > 2500.0 && self.dr > 0.0 {
            self.dr *= -1.0;
        } else if self.radius < 0.0 {
            // Upstream writes `radius < 50.0 && radius < 0.0`, where the second
            // test plainly meant `dr`. Both spellings only fire once the radius
            // has gone negative, so this keeps its behaviour without the
            // redundant half.
            self.dr *= -1.0;
        }

        // Randomly give some variations to:

        // Spiral direction (if it is within the boundaries).
        if nrand(3000) < JAGGINESS
            && (self.cx > 2000.0 && self.cx < 8000.0)
            && (self.cy > 2000.0 && self.cy < 8000.0)
        {
            self.dx = (10 - nrand(20)) as f64 * SPEED;
            self.dy = (10 - nrand(20)) as f64 * SPEED;
        }

        // The speed of the change in size of the spiral.
        if nrand(3000) < JAGGINESS {
            if lrand() & 1 == 1 {
                self.dr += (nrand(3) + 1) as f64;
            } else {
                self.dr -= (nrand(3) + 1) as f64;
            }

            // Don't let it get too wild. Note this floor is what stops the
            // radius ever shrinking again once it has been kicked.
            self.dr = self.dr.clamp(4.0, 18.0);
        }

        // The speed of rotation.
        if nrand(3000) < JAGGINESS {
            self.da = nrand(360) as f64 / 7200.0 + 0.01;
        }

        // Reverse rotation.
        if nrand(3000) < JAGGINESS {
            self.da *= -1.0;
        }

        self.angle += self.da;
        self.trail[self.inc].ha = self.angle;
        if self.angle > std::f64::consts::TAU {
            self.angle -= std::f64::consts::TAU;
        } else if self.angle < 0.0 {
            self.angle += std::f64::consts::TAU;
        }

        let npixels = self.mi.npixels();
        self.colors += npixels as f64 / (2 * self.nlength) as f64;
        if self.colors >= npixels as f64 {
            self.colors = 0.0;
        }

        let c = if npixels > 2 {
            self.mi.pixel(self.colors as usize)
        } else {
            self.mi.white
        };
        self.mi.gc.set_foreground(c);

        let inc = self.inc;
        self.draw_dots(d, inc);

        self.inc += 1;
        if self.inc > self.nlength - 1 {
            self.inc -= self.nlength;
            self.erase = true;
        }

        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.restart(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 50000",
    "*count: 40",
    "*cycles: 350",
    "*ncolors: 64",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "50000").inverted(),
    Opt::slider("count", "Count", 0.0, 100.0, 1.0, 0, "40"),
    Opt::slider("cycles", "Cycles", 10.0, 800.0, 10.0, 0, "350"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "spiral",
    label: "Spiral",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Peter Schmitzberger",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=8Ov2SxnO_Kg"),
        blurb: "Moving circular moiré patterns.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
