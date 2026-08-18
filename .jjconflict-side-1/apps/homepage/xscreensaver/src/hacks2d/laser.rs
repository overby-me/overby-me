//! Port of `hacks/laser.c`.
//!
//! ```text
//! Copyright (c) 1995 Pascal Pensa <pensa@aurora.unice.fr>
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
//! 1995: Written.
//! ```
//!
//! Beams from a common origin out to points that crawl around the edge of the
//! screen. Each beam keeps a short stack of where it has been, and once that
//! stack is full the oldest line is redrawn in black before the newest is
//! drawn, so every beam trails a fan of itself. Several steps are taken per
//! frame, which is what makes the fan wide enough to read as a sweep.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs};

/// Number of steps taken on each frame.
const MIN_REDRAW: i32 = 3;
const MAX_REDRAW: i32 = 8;

const MIN_LASER: i32 = 1;

/// Laser ray width range: how many past positions a beam keeps.
const MIN_WIDTH: i32 = 2;
const MAX_WIDTH: usize = 40;

const MIN_SPEED: i32 = 2;
const MAX_SPEED: i32 = 17;

/// Minimal distance from edges.
const MIN_DIST: i32 = 10;

/// Laser color step.
const COLOR_STEP: usize = 2;

/// `RANGE_RAND(min, max)`, half open. An empty range yields `min`, which keeps
/// a window too small to hold the margin out of trouble.
fn range_rand(min: i32, max: i32) -> i32 {
    min + nrand(max - min)
}

/// Which edge a beam's far end is currently crawling along.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Border {
    Top,
    Right,
    Bottom,
    Left,
}

struct Beam {
    /// The far end of the beam, always on an edge.
    bx: i32,
    by: i32,
    bn: Border,
    /// Which way around the edge it crawls.
    dir: bool,
    speed: i32,
    /// Past positions, oldest first once the stack has filled.
    sx: [i32; MAX_WIDTH],
    sy: [i32; MAX_WIDTH],
    color: Pixel,
}

struct Lasers {
    mi: ModeInfo,
    /// Upstream's `stippledGC`, which it keeps apart from `MI_GC`.
    gc: Gc,
    width: i32,
    height: i32,
    /// Where all the beams start.
    cx: i32,
    cy: i32,
    /// Stack depth, fill and write position.
    lw: usize,
    sw: usize,
    so: usize,
    /// Steps per frame.
    lr: i32,
    time: i32,
    beams: Vec<Beam>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // BRIGHT_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Bright);
    let gc = Gc::new(mi.white, mi.black);
    let mut st = Lasers {
        mi,
        gc,
        width: 0,
        height: 0,
        cx: 0,
        cy: 0,
        lw: MIN_WIDTH as usize,
        sw: 0,
        so: 0,
        lr: MIN_REDRAW,
        time: 0,
        beams: Vec::new(),
    };
    st.restart(d);
    Box::new(st)
}

impl Lasers {
    fn restart(&mut self, d: &mut Dpy) {
        self.width = d.width();
        self.height = d.height();
        self.time = 0;

        let mut ln = self.mi.count;
        if ln < -MIN_LASER {
            ln = nrand(-ln - MIN_LASER + 1) + MIN_LASER;
        } else if ln < MIN_LASER {
            ln = MIN_LASER;
        }

        d.clear_window();

        self.cx = if MIN_DIST < self.width - MIN_DIST {
            range_rand(MIN_DIST, self.width - MIN_DIST)
        } else {
            range_rand(0, self.width)
        };
        self.cy = if MIN_DIST < self.height - MIN_DIST {
            range_rand(MIN_DIST, self.height - MIN_DIST)
        } else {
            range_rand(0, self.height)
        };
        self.lw = range_rand(MIN_WIDTH, MAX_WIDTH as i32) as usize;
        self.lr = range_rand(MIN_REDRAW, MAX_REDRAW);
        self.sw = 0;
        self.so = 0;

        let npixels = self.mi.npixels();
        let mut c = if npixels > 2 {
            nrand(npixels) as usize
        } else {
            0
        };

        self.beams = (0..ln)
            .map(|_| {
                let bn = match nrand(4) {
                    0 => Border::Top,
                    1 => Border::Right,
                    2 => Border::Bottom,
                    _ => Border::Left,
                };
                let (bx, by) = match bn {
                    Border::Top => (nrand(self.width), 0),
                    Border::Right => (self.width, nrand(self.height)),
                    Border::Bottom => (nrand(self.width), self.height),
                    Border::Left => (0, nrand(self.height)),
                };
                let color = if npixels > 2 {
                    let p = self.mi.pixel(c);
                    c = (c + COLOR_STEP) % npixels as usize;
                    p
                } else {
                    self.mi.white
                };
                Beam {
                    bx,
                    by,
                    bn,
                    dir: lrand() & 1 == 1,
                    speed: ((range_rand(MIN_SPEED, MAX_SPEED) * self.width) / 1000) + 1,
                    sx: [0; MAX_WIDTH],
                    sy: [0; MAX_WIDTH],
                    color,
                }
            })
            .collect();
    }

    /// Crawl one beam's far end along the edges by its own speed.
    fn advance(beam: &mut Beam, width: i32, height: i32) {
        if beam.dir {
            match beam.bn {
                Border::Top => {
                    beam.bx -= beam.speed;
                    if beam.bx < 0 {
                        beam.by = -beam.bx;
                        beam.bx = 0;
                        beam.bn = Border::Left;
                    }
                }
                Border::Right => {
                    beam.by -= beam.speed;
                    if beam.by < 0 {
                        beam.bx = width + beam.by;
                        beam.by = 0;
                        beam.bn = Border::Top;
                    }
                }
                Border::Bottom => {
                    beam.bx += beam.speed;
                    if beam.bx >= width {
                        beam.by = height - beam.bx % width;
                        beam.bx = width;
                        beam.bn = Border::Right;
                    }
                }
                Border::Left => {
                    beam.by += beam.speed;
                    if beam.by >= height {
                        beam.bx = beam.by % height;
                        beam.by = height;
                        beam.bn = Border::Bottom;
                    }
                }
            }
        } else {
            match beam.bn {
                Border::Top => {
                    beam.bx += beam.speed;
                    if beam.bx >= width {
                        beam.by = beam.bx % width;
                        beam.bx = width;
                        beam.bn = Border::Right;
                    }
                }
                Border::Right => {
                    beam.by += beam.speed;
                    if beam.by >= height {
                        beam.bx = width - beam.by % height;
                        beam.by = height;
                        beam.bn = Border::Bottom;
                    }
                }
                Border::Bottom => {
                    beam.bx -= beam.speed;
                    if beam.bx < 0 {
                        beam.by = height + beam.bx;
                        beam.bx = 0;
                        beam.bn = Border::Left;
                    }
                }
                Border::Left => {
                    beam.by -= beam.speed;
                    if beam.by < 0 {
                        beam.bx = -beam.bx;
                        beam.by = 0;
                        beam.bn = Border::Top;
                    }
                }
            }
        }
    }

    fn step(&mut self, d: &mut Dpy) {
        let (width, height, cx, cy) = (self.width, self.height, self.cx, self.cy);
        let (lw, sw, so) = (self.lw, self.sw, self.so);
        let black = self.mi.black;

        for i in 0..self.beams.len() {
            if sw >= lw {
                let (sx, sy) = (self.beams[i].sx[so], self.beams[i].sy[so]);
                self.gc.set_foreground(black);
                d.win().draw_line(&self.gc, cx, cy, sx, sy);
            }

            Self::advance(&mut self.beams[i], width, height);

            let beam = &self.beams[i];
            let (bx, by, color) = (beam.bx, beam.by, beam.color);
            self.gc.set_foreground(color);
            d.win().draw_line(&self.gc, cx, cy, bx, by);

            self.beams[i].sx[so] = bx;
            self.beams[i].sy[so] = by;
        }

        if self.sw < self.lw {
            self.sw += 1;
        }
        self.so = (self.so + 1) % self.lw;
    }
}

impl Screenhack for Lasers {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for _ in 0..self.lr {
            self.step(d);
        }

        self.time += 1;
        if self.time > self.mi.cycles {
            self.restart(d);
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
    "*delay: 40000",
    "*count: 10",
    "*cycles: 200",
    "*ncolors: 64",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "40000").inverted(),
    Opt::spin("count", "Count", 0.0, 20.0, "10"),
    Opt::slider("cycles", "Duration", 0.0, 2000.0, 10.0, 0, "200"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "laser",
    label: "Laser",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Pascal Pensa",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=QjPEa3KDlsw"),
        blurb: "Radiating lines, sweeping like scanning laser beams.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
