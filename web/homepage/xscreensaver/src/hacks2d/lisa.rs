//! Port of `hacks/lisa.c`.
//!
//! ```text
//! lisa --- animated full-loop lissajous figures
//!
//! Copyright (c) 1997, 2006 by Caleb Cullen.
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
//! 23-Feb-2006: fixed color-cycling issues
//! 01-Nov-2000: Allocation checks
//! 10-May-1997: Compatible with xscreensaver
//!
//! The inspiration for this program, Lasp, was written by Adam B. Roach
//! in 1990, assisted by me, Caleb Cullen.  It was written first in C, then
//! in assembly, and used pre-calculated data tables to graph lissajous
//! figures on 386 machines and lower.  This version bears only superficial
//! resemblances to the original Lasp.
//! ```
//!
//! A lissajous figure: x and y each a product of two sines whose frequencies
//! are small whole numbers, so the curve closes on itself and the whole loop
//! is redrawn every frame with its phase advanced. Twenty-eight frequency
//! pairs are on the menu, the last three chosen only rarely, and moving from
//! one to the next is a melt rather than a cut: for one full cycle both
//! figures are evaluated and crossfaded point by point.
//!
//! The colour comes out of the loop itself. The curve is cut into runs of
//! `cstep` points, each run drawn as its own polyline in the next colour of
//! the map, and the segment between one run and the next is simply never
//! drawn. That gap is deliberate upstream: it separates the colours cleanly
//! instead of letting them collide at the joins.
//!
//! Only the chunked drawing path is here. Upstream has a second one for when
//! a run would not fit in a single X request, which needs a run of some thirty
//! thousand points; the largest the panel can ask for is a thousand.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{
    About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, XPoint, random,
};

/// Maximum velocities.
const XVMAX: i32 = 10;
const YVMAX: i32 = 10;
const NUMSTDFUNCS: usize = 28;
/// Functions from here on are "rare" and will not show up as often.
const RAREFUNCMIN: usize = 25;
/// One in n chance a rare function will be re-randomized.
const RAREFUNCODDS: i32 = 4;
const MAXCYCLES: i32 = 3;
const STARTCOLOR: usize = 0;
const STARTFUNC: usize = 24;
/// Negative, so it is an upper bound to randomize within.
const LINEWIDTH: i32 = -8;

/// One figure: `x = sin(a s) sin(b s)`, `y = sin(c t) sin(d t)`.
///
/// Upstream also stores each entry's own index; here that is its position in
/// the table, which is the same number.
struct LisaFunc {
    xcoeff: [f64; 2],
    ycoeff: [f64; 2],
    nx: usize,
    ny: usize,
}

const fn f(xcoeff: [f64; 2], ycoeff: [f64; 2]) -> LisaFunc {
    LisaFunc {
        xcoeff,
        ycoeff,
        nx: 2,
        ny: 2,
    }
}

static FUNCTION: [LisaFunc; NUMSTDFUNCS] = [
    f([1.0, 2.0], [1.0, 2.0]),
    f([1.0, 2.0], [1.0, 1.0]),
    f([1.0, 3.0], [1.0, 2.0]),
    f([1.0, 3.0], [1.0, 3.0]),
    f([2.0, 4.0], [1.0, 2.0]),
    f([1.0, 4.0], [1.0, 3.0]),
    f([1.0, 4.0], [1.0, 4.0]),
    f([1.0, 5.0], [1.0, 5.0]),
    f([2.0, 5.0], [2.0, 5.0]),
    f([1.0, 2.0], [2.0, 5.0]),
    f([1.0, 2.0], [3.0, 5.0]),
    f([1.0, 2.0], [2.0, 3.0]),
    f([1.0, 3.0], [2.0, 3.0]),
    f([2.0, 3.0], [1.0, 3.0]),
    f([2.0, 4.0], [1.0, 3.0]),
    f([1.0, 4.0], [2.0, 3.0]),
    f([2.0, 4.0], [2.0, 3.0]),
    f([1.0, 5.0], [2.0, 3.0]),
    f([2.0, 5.0], [2.0, 3.0]),
    f([1.0, 5.0], [2.0, 5.0]),
    f([1.0, 3.0], [2.0, 7.0]),
    f([2.0, 3.0], [5.0, 7.0]),
    f([1.0, 2.0], [3.0, 7.0]),
    f([2.0, 5.0], [5.0, 7.0]),
    f([5.0, 7.0], [5.0, 7.0]),
    // Functions past here are rare.
    f([2.0, 7.0], [1.0, 7.0]),
    f([2.0, 9.0], [1.0, 7.0]),
    f([5.0, 11.0], [2.0, 9.0]),
];

struct Lisas {
    /// An index into the colormap. In mono upstream keeps a pixel value here
    /// instead and never reads it back, because that branch draws in white.
    color: usize,
    radius: i32,
    dx: i32,
    dy: i32,
    nsteps: i32,
    nfuncs: i32,
    /// Frames left in the crossfade from the old figure to the new one.
    melting: i32,
    /// Points per colour, and so per separately drawn polyline.
    cstep: i32,
    pistep: f64,
    center: XPoint,
    /// The last frame's points, kept so they can be painted out again. The
    /// array is padded so the final run can be taken as a slice like the rest.
    lastpoint: Vec<XPoint>,
    /// `[0]` is the figure being drawn, `[1]` the one it is melting into.
    function: [usize; 2],
    linewidth: i32,
}

struct State {
    mi: ModeInfo,
    width: i32,
    height: i32,
    lissajous: Vec<Lisas>,
    loopcount: i32,
    maxcycles: i32,
    additive: bool,
}

impl State {
    /// `CHECK_RADIUS`: take the size resource if the window is comfortably
    /// bigger than it, and fall back to a fraction of the window otherwise.
    fn check_radius(&mut self, li: usize) {
        let size = self.mi.size;
        let (w, h) = (self.width, self.height);
        let lp = &mut self.lissajous[li];
        if h / 2 > size && w / 2 > size {
            lp.radius = size;
        }
        if lp.radius < 0 || lp.radius > lp.center.x || lp.radius > lp.center.y {
            lp.radius = w.min(h) * 3 / 8;
        }
    }

    /// The points of one frame of one figure.
    fn points(&self, li: usize, phase: i32) -> Vec<XPoint> {
        let lp = &self.lissajous[li];
        let extra_points = lp.cstep - (lp.nsteps % lp.cstep);
        let mut np = vec![XPoint::default(); (lp.nsteps + extra_points) as usize];

        for (pctr, slot) in np.iter_mut().enumerate().take(lp.nsteps as usize) {
            let pctr = pctr as i32;
            let phi = (pctr - phase) as f64 * lp.pistep;
            let theta = (pctr + phase) as f64 * lp.pistep;
            let (mut xsum, mut ysum) = (0.0, 0.0);

            // Counting down, so the figure being melted into is evaluated
            // first and the one being melted away from last.
            let mut fctr = lp.nfuncs;
            while fctr > 0 {
                fctr -= 1;
                let fun = &FUNCTION[lp.function[fctr as usize]];
                let (mut xprod, mut yprod);

                if self.additive {
                    xprod = 0.0;
                    yprod = 0.0;
                    for c in fun.xcoeff.iter().take(fun.nx) {
                        xprod += (c * theta).sin();
                    }
                    for c in fun.ycoeff.iter().take(fun.ny) {
                        yprod += (c * phi).sin();
                    }
                    if lp.melting != 0 {
                        let w = if fctr != 0 {
                            (lp.nsteps - lp.melting) as f64 / lp.nsteps as f64
                        } else {
                            lp.melting as f64 / lp.nsteps as f64
                        };
                        xsum += xprod * w;
                        ysum += yprod * w;
                    } else {
                        xsum = xprod;
                        ysum = yprod;
                    }
                    if fctr == 0 {
                        xsum = xsum * lp.radius as f64 / fun.nx as f64;
                        ysum = ysum * lp.radius as f64 / fun.ny as f64;
                    }
                } else {
                    if lp.melting != 0 {
                        let m = if fctr != 0 {
                            (lp.nsteps - lp.melting) as f64
                        } else {
                            lp.melting as f64
                        };
                        xprod = lp.radius as f64 * m / lp.nsteps as f64;
                    } else {
                        xprod = lp.radius as f64;
                    }
                    yprod = xprod;
                    for c in fun.xcoeff.iter().take(fun.nx) {
                        xprod *= (c * theta).sin();
                    }
                    for c in fun.ycoeff.iter().take(fun.ny) {
                        yprod *= (c * phi).sin();
                    }
                    xsum += xprod;
                    ysum += yprod;
                }
            }

            if lp.nfuncs > 1 && lp.melting == 0 {
                xsum /= lp.nfuncs as f64;
                ysum /= lp.nfuncs as f64;
            }
            xsum += lp.center.x as f64;
            ysum += lp.center.y as f64;
            slot.x = xsum.ceil() as i32;
            slot.y = ysum.ceil() as i32;
        }

        // Fill in the extra points, so the last run can be drawn like the rest.
        for pctr in lp.nsteps as usize..np.len() {
            np[pctr] = np[pctr - lp.nsteps as usize];
        }
        np
    }

    /// Paint out last frame's runs and lay down this frame's, one colour at a
    /// time. The segment joining one run to the next is never drawn, which is
    /// what keeps the colours from colliding.
    fn stroke(&mut self, d: &mut Dpy, li: usize, old: Option<&[XPoint]>, np: &[XPoint]) {
        let (nsteps, cstep, lw) = {
            let lp = &self.lissajous[li];
            (lp.nsteps, lp.cstep as usize, lp.linewidth)
        };
        let npixels = self.mi.npixels();
        let (black, white) = (self.mi.black, self.mi.white);
        let mut color = self.lissajous[li].color;

        let mut pctr = 0usize;
        while (pctr as i32) < nsteps {
            self.mi.gc.line_width = lw;
            if let Some(old) = old {
                self.mi.gc.set_foreground(black);
                d.win().draw_lines(&self.mi.gc, &old[pctr..pctr + cstep]);
            }

            // SET_COLOR: draw in the colour we are on, then step to the next.
            if npixels > 2 {
                let p = self.mi.pixel(color);
                self.mi.gc.set_foreground(p);
                if cstep != 0 {
                    color += 1;
                    if color >= npixels as usize {
                        color = STARTCOLOR;
                    }
                }
            } else {
                self.mi.gc.set_foreground(white);
            }
            d.win().draw_lines(&self.mi.gc, &np[pctr..pctr + cstep]);
            self.mi.gc.line_width = 1;

            pctr += cstep;
        }
        self.lissajous[li].color = color;
    }

    fn drawlisa(&mut self, d: &mut Dpy, li: usize) {
        let phase = self.loopcount % self.lissajous[li].nsteps;

        // Update the centre, then check for overlaps: where the figure might
        // go off the screen, it bounces off with a fresh random speed.
        {
            let lp = &mut self.lissajous[li];
            lp.center.x += lp.dx;
            lp.center.y += lp.dy;
        }
        self.check_radius(li);
        {
            let (w, h) = (self.width, self.height);
            let lp = &mut self.lissajous[li];
            if lp.center.x - lp.radius <= 0 {
                lp.center.x = lp.radius;
                lp.dx = nrand(XVMAX);
            } else if lp.center.x + lp.radius >= w {
                lp.center.x = w - lp.radius;
                lp.dx = -nrand(XVMAX);
            }
            if lp.center.y - lp.radius <= 0 {
                lp.center.y = lp.radius;
                lp.dy = nrand(YVMAX);
            } else if lp.center.y + lp.radius >= h {
                lp.center.y = h - lp.radius;
                lp.dy = -nrand(YVMAX);
            }
        }

        let np = self.points(li, phase);

        {
            let lp = &mut self.lissajous[li];
            if lp.melting != 0 {
                lp.melting -= 1;
                if lp.melting == 0 {
                    lp.nfuncs = 1;
                    lp.function[0] = lp.function[1];
                }
            }
            // Reset the starting colour each time, so the loop's colouring
            // looks solid rather than flickering.
            lp.color = STARTCOLOR;
        }

        let old = std::mem::take(&mut self.lissajous[li].lastpoint);
        self.stroke(d, li, Some(&old), &np);
        self.lissajous[li].lastpoint = np;
    }

    /// Pick a new figure for every loop and start the melt into it.
    fn change(&mut self) {
        self.loopcount = 0;
        for li in 0..self.lissajous.len() {
            let current = self.lissajous[li].function[0];
            let mut newfunc = nrand(NUMSTDFUNCS as i32) as usize;
            if newfunc == current {
                // Take the next if we got the one we have.
                newfunc = (newfunc + 1) % NUMSTDFUNCS;
            }
            if newfunc >= RAREFUNCMIN && random().is_multiple_of(RAREFUNCODDS as u32) && {
                newfunc = nrand(NUMSTDFUNCS as i32) as usize;
                newfunc == current
            } {
                newfunc = (newfunc + 1) % NUMSTDFUNCS;
            }
            let lp = &mut self.lissajous[li];
            lp.function[1] = newfunc;
            // Melt the two functions together for a full cycle.
            lp.melting = lp.nsteps - 1;
            lp.nfuncs = 2;
        }
    }

    /// The first figure, drawn straight rather than over an erased one.
    ///
    /// Upstream evaluates this one multiplicatively whether or not `additive`
    /// is on, and only the frames after it follow the resource.
    fn initlisa(&mut self, d: &mut Dpy) {
        let nsteps = self.mi.cycles.max(1);
        let npixels = self.mi.npixels();
        // Upstream divides by both `cstep` and `npixels` here without
        // guarding either, so a mode with no colormap divides by zero. One
        // colour per point is what the expression is reaching for.
        let cstep = if nsteps > npixels {
            nsteps / npixels.max(1)
        } else {
            1
        };
        self.maxcycles = (MAXCYCLES * nsteps) - 1;

        let lp = Lisas {
            color: STARTCOLOR,
            radius: self.mi.size,
            dx: 0,
            dy: 0,
            nsteps,
            nfuncs: 1,
            melting: 0,
            cstep,
            pistep: 2.0 * std::f64::consts::PI / nsteps as f64,
            center: XPoint {
                x: self.width / 2,
                y: self.height / 2,
            },
            lastpoint: Vec::new(),
            function: [STARTFUNC, STARTFUNC],
            linewidth: 1,
        };

        let li = self.lissajous.len();
        self.lissajous.push(lp);
        self.check_radius(li);

        // Speed first, then the line width: that is the order the random
        // numbers come out upstream, and so what a seed reproduces.
        self.lissajous[li].dx = nrand(XVMAX) + 1;
        self.lissajous[li].dy = nrand(YVMAX) + 1;

        let mut linewidth = LINEWIDTH;
        if linewidth == 0 {
            linewidth = 1;
        }
        if linewidth < 0 {
            linewidth = nrand(-linewidth) + 1;
        }
        if self.width > 2560 || self.height > 2560 {
            linewidth *= 2; // Retina displays.
        }
        self.lissajous[li].linewidth = linewidth;

        // The opening frame is always the multiplicative form.
        let additive = std::mem::replace(&mut self.additive, false);
        let phase = self.loopcount % nsteps;
        let lp = self.points(li, phase);
        self.additive = additive;

        // Nothing to erase yet.
        self.stroke(d, li, None, &lp);
        self.lissajous[li].lastpoint = lp;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // UNIFORM_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Uniform);
    let nlissajous = mi.count.max(1);
    let mut st = State {
        mi,
        width: d.width(),
        height: d.height(),
        lissajous: Vec::new(),
        loopcount: 0,
        maxcycles: 0,
        additive: d.res.bool("additive"),
    };
    d.clear_window();
    for _ in 0..nlissajous {
        st.initlisa(d);
        st.loopcount += 1;
    }
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.loopcount += 1;
        if self.loopcount > self.maxcycles {
            self.change();
        }
        for li in 0..self.lissajous.len() {
            self.drawlisa(d, li);
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.width = width;
        self.height = height;
        // Upstream has no reshape at all, which leaves the figures walking a
        // box the window no longer has. Start them over at the new size.
        self.lissajous.clear();
        self.loopcount = 0;
        d.clear_window();
        let nlissajous = self.mi.count.max(1);
        for _ in 0..nlissajous {
            self.initlisa(d);
            self.loopcount += 1;
        }
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

const DEFAULTS: &[&str] = &[
    "*delay: 17000",
    "*count: 1",
    "*cycles: 768",
    "*size: 500",
    "*ncolors: 64",
    "*fpsSolid: true",
    "*additive: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 50_000.0, 1000.0, 0, "17000").inverted(),
    Opt::slider("cycles", "Steps", 1.0, 1000.0, 1.0, 0, "768"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
    Opt::slider("size", "Size", 10.0, 500.0, 10.0, 0, "500"),
    Opt::slider("count", "Count", 0.0, 20.0, 1.0, 0, "1"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "lisa",
    label: "Lisa",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Caleb Cullen",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=AUbAuARmlnE"),
        blurb: "Lissajous loops.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
