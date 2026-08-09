//! Port of `hacks/thornbird.c`.
//!
//! ```text
//! Copyright (c) 1997 by Tim Auckland <Tim.Auckland@Procket.com>
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
//! 04-Jun-1999: 3D tumble added by Tim Auckland
//! 31-Jul-1997: Adapted from discrete.c Copyright (c) 1996 by Tim Auckland
//! ```
//!
//! The "Bird in a Thornbush" fractal map, with its three free parameters
//! varying continuously on two incommensurate Lissajous frequencies and the
//! whole thing tumbling in three dimensions. Points are kept in a ring of
//! buffers so an old frame can be erased exactly rather than by clearing, which
//! is what gives the figure its persistent, smoky trail.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XRectangle};

/// `balance_rand(v)`: random around zero.
fn balance_rand(v: f64) -> f64 {
    (lrand() as f64 / u32::MAX as f64 * v) - (v / 2.0)
}

struct Thornbird {
    mi: ModeInfo,
    maxx: i32,
    maxy: i32,
    a: f64,
    b: f64,
    c: f64,
    i: f64,
    j: f64,
    /// The two frequencies the parameters vary on.
    f1: f64,
    f2: f64,
    theta: f64,
    dtheta: f64,
    phi: f64,
    dphi: f64,
    inc: i32,
    pix: usize,
    nbuffers: usize,
    scale: i32,
    /// A ring of frames, so the oldest can be erased point by point.
    point_buffer: Vec<Option<Vec<XRectangle>>>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // BRIGHT_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Bright);
    let nbuffers = mi.cycles.max(1) as usize;
    let mut st = Thornbird {
        maxx: mi.width,
        maxy: mi.height,
        scale: if mi.width > 2560 || mi.height > 2560 {
            2 // Retina displays
        } else {
            1
        },
        mi,
        a: 0.0,
        b: 0.1,
        c: 0.0,
        i: 0.1,
        j: 0.1,
        f1: 0.0,
        f2: 0.0,
        theta: 0.0,
        dtheta: 0.0,
        phi: 0.0,
        dphi: 0.0,
        inc: 0,
        pix: 0,
        nbuffers,
        point_buffer: (0..nbuffers).map(|_| None).collect(),
    };
    st.restart(d);
    Box::new(st)
}

impl Thornbird {
    fn restart(&mut self, d: &mut Dpy) {
        self.maxx = d.width();
        self.maxy = d.height();
        self.b = 0.1;
        self.i = 0.1;
        self.j = 0.1;
        self.pix = 0;
        self.inc = 0;

        // Frequencies for the parameter variation.
        self.f1 = (lrand() % 5000) as f64;
        self.f2 = (lrand() % 2000) as f64;

        // Random 3D tumbling.
        self.theta = 0.0;
        self.phi = 0.0;
        self.dtheta = balance_rand(0.001);
        self.dphi = balance_rand(0.005);

        d.clear_window();
    }
}

impl Screenhack for Thornbird {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let batchcount = self.mi.count.max(1) as usize;
        let erase = ((self.inc + 1) as usize) % self.nbuffers;
        let current = (self.inc as usize) % self.nbuffers;

        // Vary the parameters along two incommensurate frequencies.
        let inc = self.inc as f64;
        self.a =
            1.99 + (0.4 * (inc / self.f1.max(1.0)).sin() + 0.05 * (inc / self.f2.max(1.0)).cos());
        self.c =
            0.80 + (0.15 * (inc / self.f1.max(1.0)).cos() + 0.05 * (inc / self.f2.max(1.0)).sin());

        // Vary the view.
        self.theta += self.dtheta;
        self.phi += self.dphi;
        let (sint, cost) = (self.theta.sin(), self.theta.cos());
        let (sinp, cosp) = (self.phi.sin(), self.phi.cos());

        let mut points = Vec::with_capacity(batchcount);
        for _ in 0..batchcount {
            let oldj = self.j;
            let oldi = self.i;

            self.j = oldi;
            self.i =
                (1.0 - self.c) * (std::f64::consts::PI * self.a * oldj).cos() + self.c * self.b;
            self.b = oldj;

            points.push(XRectangle {
                x: (self.maxx as f64 / 2.0
                    * (1.0 + sint * self.j + cost * cosp * self.i - cost * sinp * self.b))
                    as i16 as i32,
                y: (self.maxy as f64 / 2.0
                    * (1.0 - cost * self.j + sint * cosp * self.i - sint * sinp * self.b))
                    as i16 as i32,
                width: self.scale,
                height: self.scale,
            });
        }

        // Erase the frame this buffer held before, if it held one.
        match self.point_buffer[erase].as_ref() {
            None => {}
            Some(old) => {
                let black = self.mi.black;
                self.mi.gc.set_foreground(black);
                let old = old.clone();
                d.win().fill_rectangles(&self.mi.gc, &old);
            }
        }

        if self.mi.npixels() > 2 {
            let c = self.mi.pixel(self.pix);
            self.mi.gc.set_foreground(c);
            // jwz: change colours sooner than once per full ring.
            if (self.inc + 1) % (1 + (self.mi.cycles / 3)).max(1) == 0 {
                self.pix += 1;
                if self.pix >= self.mi.npixels() as usize {
                    self.pix = 0;
                }
            }
        } else {
            let w = self.mi.white;
            self.mi.gc.set_foreground(w);
        }

        d.win().fill_rectangles(&self.mi.gc, &points);
        self.point_buffer[current] = Some(points);
        self.inc += 1;
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.restart(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay:    10000",
    "*count:    100",
    "*cycles:  400",
    "*ncolors: 64",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Points", 10.0, 1000.0, 10.0, 0, "100"),
    Opt::slider("cycles", "Thickness", 2.0, 1000.0, 10.0, 0, "400"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "thornbird",
    label: "Thornbird",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tim Auckland",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=rfGfPezVnac"),
        blurb: "The Bird in a Thornbush fractal, tumbling in three dimensions.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
