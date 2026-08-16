//! Port of `hacks/vines.c`.
//!
//! ```text
//! Copyright (c) 1997 by Tracy Camp campt@hurrah.com
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
//! If you make a modification I would of course appreciate a copy.
//!
//! 11-Jul-1997: David Hansen <dhansen@metapath.com>
//!              Changed names to vines and modified draw loop
//!              to honor batchcount so vines can be grown or plotted.
//! 10-May-1997: Compatible with xscreensaver
//! 21-Mar-1997: David Hansen <dhansen@metapath.com>
//!              Updated mode to draw complete patterns on every
//!              iteration instead of growing the vine.
//! ```
//!
//! From the header: adapted from a screen saver the author and a friend wrote
//! on their TI-8x calculators in high school physics one day. A geometric
//! pattern generator whose claim to fame is a pseudo-fractal vine-like pattern
//! of whorls and loops.
//!
//! The first port of an xlockmore-style hack, so it goes through
//! [`crate::runtime::xlockmore`] rather than taking a display and a window.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs};

struct Vines {
    mi: ModeInfo,
    a: i64,
    x1: i64,
    y1: i64,
    x2: i64,
    y2: i64,
    i: i64,
    length: i64,
    iterations: i32,
    constant: i64,
    ang: i64,
    centerx: i32,
    centery: i32,
    /// Retina displays: draw longer vines so the pattern still fills the frame.
    pscale: i64,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mi = ModeInfo::new(d, ColorScheme::Random);
    let mut st = Vines {
        mi,
        a: 0,
        x1: 0,
        y1: 0,
        x2: 1,
        y2: 0,
        i: 0,
        length: 0,
        iterations: 0,
        constant: 0,
        ang: 0,
        centerx: 0,
        centery: 0,
        pscale: 1,
    };
    st.restart(d);
    Box::new(st)
}

impl Vines {
    /// `init_vines`, which the hack also calls on itself when it runs out of
    /// iterations.
    fn restart(&mut self, d: &mut Dpy) {
        self.i = 0;
        self.length = 0;
        self.iterations = 30 + nrand(100);

        self.pscale = 1;
        if self.mi.width > 1280 || self.mi.height > 1280 {
            self.pscale *= 3;
        }
        if self.mi.width > 2560 || self.mi.height > 2560 {
            self.pscale *= 2;
        }

        d.clear_window();
    }
}

impl Screenhack for Vines {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.i >= self.length {
            self.iterations -= 1;
            if self.iterations == 0 {
                self.restart(d);
                return self.mi.delay;
            }
            self.centerx = nrand(self.mi.width);
            self.centery = nrand(self.mi.height);

            self.ang = (60 + nrand(720)) as i64;
            self.length = (100 + nrand(3000)) as i64 * self.pscale;
            self.constant = self.length * (10 + nrand(10)) as i64;

            self.i = 0;
            self.a = 0;
            self.x1 = 0;
            self.y1 = 0;
            self.x2 = 1;
            self.y2 = 0;

            let color = if self.mi.npixels() > 2 {
                self.mi.pixel(nrand(self.mi.npixels()) as usize)
            } else {
                self.mi.white
            };
            self.mi.gc.set_foreground(color);
        }

        // `count` is the batch size: zero means draw the whole vine at once.
        let mut count = self.i + self.mi.count as i64;
        if count <= self.i || count > self.length {
            count = self.length;
        }

        while self.i < count {
            if self.constant != 0 {
                d.win().draw_line(
                    &self.mi.gc,
                    self.centerx + (self.x1 / self.constant) as i32,
                    self.centery - (self.y1 / self.constant) as i32,
                    self.centerx + (self.x2 / self.constant) as i32,
                    self.centery - (self.y2 / self.constant) as i32,
                );
            }

            self.a += self.ang * self.i;

            self.x1 = self.x2;
            self.y1 = self.y2;

            let a = self.a as f64;
            self.x2 += (self.i as f64 * (a.cos() * 360.0) / std::f64::consts::TAU) as i64;
            self.y2 += (self.i as f64 * (a.sin() * 360.0) / std::f64::consts::TAU) as i64;
            self.i += 1;
        }

        self.mi.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 200000",
    "*count: 0",
    "*ncolors: 64",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 250_000.0, 1000.0, 0, "200000").inverted(),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "vines",
    label: "Vines",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tracy Camp and David Hansen",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=IaVfFCIAUn8"),
        blurb: "A pseudo-fractal vine of whorls and loops.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
