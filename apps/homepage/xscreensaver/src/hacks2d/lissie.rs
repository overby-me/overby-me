//! Port of `hacks/lissie.c`.
//!
//! ```text
//! lissie.c - The Lissajous worm for xlock, the X Window System
//!               lockscreen.
//!
//! Copyright (c) 1996 by Alexander Jolk <ub9x@rz.uni-karlsruhe.de>
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
//! 01-Nov-2000: Allocation checks
//! 10-May-1997: Compatible with xscreensaver
//! 18-Aug-1996: added refresh-hook.
//! 01-May-1996: written.
//! ```
//!
//! A worm crawling a Lissajous figure. Its head is a circle whose two
//! coordinates run on independent sine waves, and a ring buffer of past
//! positions is erased from the tail as fast as it is drawn at the head, so a
//! fixed length of worm chases itself around the curve. Both frequencies drift
//! by up to one percent per frame, so the figure never closes on itself twice.
//!
//! Upstream's redraw-from-the-tail path is left out: it exists to service
//! `refresh_lissie`, which is compiled only for xlock, so nothing ever sets
//! `redrawing` in the xscreensaver build either.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint, frand};

const MIN_SIZE: i32 = 1;
const MIN_DT: f64 = 0.01;
const MAX_DT: f64 = 0.15;
const MAX_LISSIE_LEN: usize = 100;
const MIN_LISSIE_LEN: i32 = 10;
const MIN_LISSIES: i32 = 1;

/// `FLOATRAND(min, max)`.
fn floatrand(min: f64, max: f64) -> f64 {
    min + frand(max - min)
}

/// `INTRAND(min, max)`, inclusive at both ends. An inverted range yields `min`,
/// which is how a window too small to hold a worm stays out of trouble.
fn intrand(min: i32, max: i32) -> i32 {
    min + nrand(max - min + 1)
}

struct Worm {
    tx: f64,
    ty: f64,
    dtx: f64,
    dty: f64,
    /// Centre of the figure.
    xi: i32,
    yi: i32,
    /// Radius of the worm itself.
    ri: i32,
    /// Half-extent of the figure, per axis.
    rx: i32,
    ry: i32,
    len: i32,
    pos: i32,
    color: usize,
    loc: [XPoint; MAX_LISSIE_LEN],
}

impl Default for Worm {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            dtx: 0.0,
            dty: 0.0,
            xi: 0,
            yi: 0,
            ri: 0,
            rx: 0,
            ry: 0,
            len: 0,
            pos: 0,
            color: 0,
            loc: [XPoint::default(); MAX_LISSIE_LEN],
        }
    }
}

struct Lissie {
    mi: ModeInfo,
    width: i32,
    height: i32,
    worms: Vec<Worm>,
    loopcount: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut st = Lissie {
        mi,
        width: d.width(),
        height: d.height(),
        worms: Vec::new(),
        loopcount: 0,
    };
    st.restart(d);
    Box::new(st)
}

impl Lissie {
    /// The `Lissie(n)` macro: draw whichever ring slot `n` holds, if it is on
    /// screen, in whatever colour the GC currently carries.
    fn plot(&mut self, d: &mut Dpy, w: usize, n: usize) {
        let p = self.worms[w].loc[n];
        let ri = self.worms[w].ri;
        if p.x > 0 && p.y > 0 && p.x <= self.width && p.y <= self.height {
            if ri < 2 {
                d.win().draw_point(&self.mi.gc, p.x, p.y);
            } else {
                d.win().draw_arc(
                    &self.mi.gc,
                    p.x - ri / 2,
                    p.y - ri / 2,
                    ri,
                    ri,
                    0,
                    FULL_CIRCLE,
                );
            }
        }
    }

    fn init_worm(&mut self, w: usize) {
        let size = self.mi.size;
        let npixels = self.mi.npixels();
        let (width, height) = (self.width, self.height);
        // The biggest worm the window has room for.
        let room = (width.min(height) / 4).max(MIN_SIZE);

        let worm = &mut self.worms[w];
        // In mono upstream stores a pixel value here rather than an index, but
        // never reads it back: the mono branch draws in white directly.
        worm.color = if npixels > 2 {
            nrand(npixels) as usize
        } else {
            0
        };

        worm.ri = if size < -MIN_SIZE {
            nrand((-size).min(room) - MIN_SIZE + 1) + MIN_SIZE
        } else if size < MIN_SIZE {
            if size == 0 { room } else { MIN_SIZE }
        } else {
            size.min(room)
        };

        worm.xi = intrand(width / 4 + worm.ri, width * 3 / 4 - worm.ri);
        worm.yi = intrand(height / 4 + worm.ri, height * 3 / 4 - worm.ri);
        worm.rx = intrand(width / 4, (width - worm.xi).min(worm.xi)) - 2 * worm.ri;
        worm.ry = intrand(height / 4, (height - worm.yi).min(worm.yi)) - 2 * worm.ri;
        worm.len = intrand(MIN_LISSIE_LEN, MAX_LISSIE_LEN as i32 - 1);
        worm.pos = 0;

        worm.tx = floatrand(0.0, std::f64::consts::TAU);
        worm.ty = floatrand(0.0, std::f64::consts::TAU);
        worm.dtx = floatrand(MIN_DT, MAX_DT);
        worm.dty = floatrand(MIN_DT, MAX_DT);

        worm.loc = [XPoint::default(); MAX_LISSIE_LEN];
    }

    fn restart(&mut self, d: &mut Dpy) {
        self.width = d.width();
        self.height = d.height();

        let mut nlissies = self.mi.count;
        if nlissies < -MIN_LISSIES {
            nlissies = nrand(-nlissies - MIN_LISSIES + 1) + MIN_LISSIES;
        } else if nlissies < MIN_LISSIES {
            nlissies = MIN_LISSIES;
        }

        self.loopcount = 0;
        self.worms = (0..nlissies).map(|_| Worm::default()).collect();

        d.clear_window();
        for w in 0..self.worms.len() {
            self.init_worm(w);
        }
    }

    fn draw_worm(&mut self, d: &mut Dpy, w: usize) {
        self.worms[w].pos += 1;
        let p = (self.worms[w].pos as usize) % MAX_LISSIE_LEN;
        let oldp =
            (self.worms[w].pos - self.worms[w].len).rem_euclid(MAX_LISSIE_LEN as i32) as usize;

        {
            let worm = &mut self.worms[w];

            // Let time go by ...
            worm.tx += worm.dtx;
            worm.ty += worm.dty;
            if worm.tx > std::f64::consts::TAU {
                worm.tx -= std::f64::consts::TAU;
            }
            if worm.ty > std::f64::consts::TAU {
                worm.ty -= std::f64::consts::TAU;
            }

            // Vary both (x/y) speeds by max. 1%.
            worm.dtx *= floatrand(0.99, 1.01);
            worm.dty *= floatrand(0.99, 1.01);
            worm.dtx = worm.dtx.clamp(MIN_DT, MAX_DT);
            worm.dty = worm.dty.clamp(MIN_DT, MAX_DT);

            worm.loc[p].x = worm.xi + (worm.tx.sin() * worm.rx as f64) as i32;
            worm.loc[p].y = worm.yi + (worm.ty.sin() * worm.ry as f64) as i32;
        }

        // Erase the tail.
        let black = self.mi.black;
        self.mi.gc.set_foreground(black);
        self.plot(d, w, oldp);

        // Draw the head.
        if self.mi.npixels() > 2 {
            let c = self.mi.pixel(self.worms[w].color);
            self.mi.gc.set_foreground(c);
            self.worms[w].color += 1;
            if self.worms[w].color >= self.mi.npixels() as usize {
                self.worms[w].color = 0;
            }
        } else {
            let white = self.mi.white;
            self.mi.gc.set_foreground(white);
        }
        self.plot(d, w, p);
    }
}

impl Screenhack for Lissie {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.loopcount += 1;
        if self.loopcount > self.mi.cycles {
            self.restart(d);
        } else {
            for w in 0..self.worms.len() {
                self.draw_worm(d, w);
            }
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
    "*delay: 10000",
    "*count: 1",
    "*cycles: 20000",
    "*size: -200",
    "*ncolors: 200",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("cycles", "Timeout", 0.0, 80000.0, 1000.0, 0, "20000"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
    Opt::slider("count", "Count", 0.0, 20.0, 1.0, 0, "1"),
    Opt::spin("size", "Size", -500.0, 500.0, "-200"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "lissie",
    label: "Lissie",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Alexander Jolk",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=6EBNCXcD9f0"),
        blurb: "A worm crawling a Lissajous figure.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
