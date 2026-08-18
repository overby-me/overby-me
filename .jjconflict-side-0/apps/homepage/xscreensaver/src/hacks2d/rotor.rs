//! Port of `hacks/rotor.c`.
//!
//! ```text
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
//! 01-Nov-2000: Allocation checks
//! 10-May-1997: Compatible with xscreensaver
//! 08-Mar-1995: CAT stuff for ## was tripping up some C compilers.  Removed.
//! 01-Dec-1993: added patch for AIXV3 from Tom McConnell
//!              <tmcconne@sedona.intel.com>
//! 11-Nov-1990: put into xlock by Steve Zellers <zellers@sun.com>
//! 16-Oct-1990: Received from Tom Lawrence (tcl@cs.brown.edu: 'flight'
//!               simulator)
//! ```
//!
//! Tom's Roto-Rooter. A chain of arms, each spinning at its own rate about the
//! tip of the one before it, with the pen on the end. Every arm's length and
//! rate drift slowly and independently towards new random targets, so the curve
//! never repeats. A ring of past positions is kept so the oldest segment can be
//! erased exactly as the newest is drawn, which is what gives the line its
//! fixed length.
//!
//! Upstream's iconified-view path is left out: it rescales the drawing for
//! xlock's little preview window, which has no counterpart here.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

/// The angle the drive winds up to before it starts unwinding.
const MAX_ANGLE: f32 = 3000.0;

#[derive(Clone, Copy, Default)]
struct Elem {
    angle: f32,
    radius: f32,
    start_radius: f32,
    end_radius: f32,
    radius_drift_max: f32,
    radius_drift_now: f32,

    ratio: f32,
    start_ratio: f32,
    end_ratio: f32,
    ratio_drift_max: f32,
    ratio_drift_now: f32,
}

struct Rotor {
    mi: ModeInfo,
    pix: usize,
    lastx: i32,
    lasty: i32,
    num: i32,
    /// Write and erase positions in the ring of past pen positions.
    rotor: usize,
    prev: usize,
    nsave: usize,
    angle: f32,
    centerx: i32,
    centery: i32,
    firsttime: bool,
    forward: bool,
    elements: Vec<Elem>,
    save: Vec<XPoint>,
    linewidth: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut st = Rotor {
        mi,
        pix: 0,
        lastx: 0,
        lasty: 0,
        num: 0,
        rotor: 0,
        prev: 1,
        nsave: 2,
        angle: 0.0,
        centerx: 0,
        centery: 0,
        firsttime: true,
        forward: true,
        elements: Vec::new(),
        save: Vec::new(),
        linewidth: 1,
    };
    st.restart(d);
    Box::new(st)
}

impl Rotor {
    fn restart(&mut self, d: &mut Dpy) {
        self.centerx = self.mi.width / 2;
        self.centery = self.mi.height / 2;

        self.num = self.mi.count;
        if self.num < 0 {
            self.num = nrand(-self.num) + 1;
        }
        self.elements = vec![Elem::default(); self.num.max(0) as usize];

        self.nsave = self.mi.cycles.max(2) as usize;
        self.save = vec![
            XPoint {
                x: self.centerx,
                y: self.centery,
            };
            self.nsave
        ];

        for e in self.elements.iter_mut() {
            e.radius_drift_max = 1.0;
            e.radius_drift_now = 1.0;
            e.end_radius = 100.0;

            e.ratio_drift_max = 1.0;
            e.ratio_drift_now = 1.0;
            e.end_ratio = 10.0;
        }

        if self.mi.npixels() > 2 {
            self.pix = nrand(self.mi.npixels()) as usize;
        }

        self.rotor = 0;
        self.prev = 1;
        self.lastx = self.centerx;
        self.lasty = self.centery;
        self.angle = nrand(MAX_ANGLE as i32) as f32 / 3.0;
        self.forward = true;
        self.firsttime = true;

        self.linewidth = self.mi.size;
        if self.linewidth == 0 {
            self.linewidth = 1;
        }
        if self.linewidth < 0 {
            self.linewidth = nrand(-self.linewidth) + 1;
        }
        if self.mi.width > 2560 || self.mi.height > 2560 {
            self.linewidth *= 2; // Retina displays
        }

        d.clear_window();
    }
}

impl Screenhack for Rotor {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut penx = self.centerx;
        let mut peny = self.centery;

        self.mi.gc.set_line_width(self.linewidth);

        for e in self.elements.iter_mut() {
            // Each arm's length and rate creep towards a fresh target, and pick
            // a new one whenever they arrive.
            if e.radius_drift_max <= e.radius_drift_now {
                e.start_radius = e.end_radius;
                e.end_radius = nrand(40000) as f32 / 100.0 - 200.0;
                e.radius_drift_max = nrand(100_000) as f32 + 10000.0;
                e.radius_drift_now = 0.0;
            }
            if e.ratio_drift_max <= e.ratio_drift_now {
                e.start_ratio = e.end_ratio;
                e.end_ratio = nrand(2000) as f32 / 100.0 - 10.0;
                e.ratio_drift_max = nrand(100_000) as f32 + 10000.0;
                e.ratio_drift_now = 0.0;
            }
            e.ratio = e.start_ratio
                + (e.end_ratio - e.start_ratio) / e.ratio_drift_max * e.ratio_drift_now;
            e.angle = self.angle * e.ratio;
            e.radius = e.start_radius
                + (e.end_radius - e.start_radius) / e.radius_drift_max * e.radius_drift_now;

            penx += (e.angle.cos() * e.radius) as i32;
            peny += (e.angle.sin() * e.radius) as i32;

            e.ratio_drift_now += 1.0;
            e.radius_drift_now += 1.0;
        }

        if self.firsttime {
            self.firsttime = false;
        } else {
            let black = self.mi.black;
            self.mi.gc.set_foreground(black);
            let (a, b) = (self.save[self.rotor], self.save[self.prev]);
            d.win().draw_line(&self.mi.gc, a.x, a.y, b.x, b.y);

            if self.mi.npixels() > 2 {
                let c = self.mi.pixel(self.pix);
                self.mi.gc.set_foreground(c);
                self.pix += 1;
                if self.pix >= self.mi.npixels() as usize {
                    self.pix = 0;
                }
            } else {
                let white = self.mi.white;
                self.mi.gc.set_foreground(white);
            }

            let (x1, y1) = (self.lastx, self.lasty);
            d.win().draw_line(&self.mi.gc, x1, y1, penx, peny);
        }

        self.save[self.rotor] = XPoint { x: penx, y: peny };
        self.lastx = penx;
        self.lasty = peny;

        self.rotor = (self.rotor + 1) % self.nsave;
        self.prev = (self.prev + 1) % self.nsave;

        // The drive winds up slowly and unwinds ten times faster.
        if self.forward {
            self.angle += 0.01;
            if self.angle >= MAX_ANGLE {
                self.angle = MAX_ANGLE;
                self.forward = false;
            }
        } else {
            self.angle -= 0.1;
            if self.angle <= 0.0 {
                self.angle = 0.0;
                self.forward = true;
            }
        }

        self.mi.gc.set_line_width(1);
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
    "*count: 4",
    "*cycles: 20",
    "*size: -6",
    "*ncolors: 200",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("cycles", "Length", 2.0, 100.0, 1.0, 0, "20"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
    Opt::spin("count", "Count", 0.0, 20.0, "4"),
    Opt::spin("size", "Size", -50.0, 50.0, "-6"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "rotor",
    label: "Rotor",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tom Lawrence",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=HWcEvT1keDA"),
        blurb: "A line segment moving along a complex spiraling curve.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
