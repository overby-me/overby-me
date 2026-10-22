//! Port of `hacks/worm.c`.
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
//! 10-May-97: Compatible with xscreensaver
//! 03-Sep-96: fixed bug in allocation of space for worms, added 3d support
//!            Henrik Theiling <theiling@coli.uni-sb.de>
//! 27-Sep-95: put back malloc
//! 23-Sep-93: got rid of "rint". (David Bagley)
//! 27-Sep-91: got rid of all malloc calls since there were no calls to free().
//! 25-Sep-91: Integrated into X11R5 contrib xlock.
//!
//! Adapted from a concept in the Dec 87 issue of Scientific American p. 142.
//!
//! SunView version: Brad Taylor <brad@sun.com>
//! X11 version: Dave Lemke <lemke@ncd.com>
//! xlock version: Boris Putanec <bp@cs.brown.edu>
//! ```
//!
//! Worms that crawl. Each keeps a heading out of thirty-six and turns one step
//! either way at random every frame, so it wanders without ever doubling back
//! sharply. A ring of past positions is kept, and the cell at the tail is
//! cleared as the head is drawn, which is what gives every worm the same
//! length. Positions wrap at the edges, so a worm that leaves one side crawls
//! in at the other.
//!
//! Upstream's red-and-blue anaglyph mode is left out: it needs `use3d`, which
//! is off by default and has no control in the settings.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{
    About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint, XRectangle,
};

const MIN_SIZE: i32 = 1;
/// Headings a worm can take.
const SEGMENTS: usize = 36;
const MIN_WORMS: i32 = 1;
/// The largest colour count the rectangle table is sized for.
const NUM_COLORS: i32 = 256;

struct WormStuff {
    circ: Vec<XPoint>,
    dir: usize,
    tail: usize,
    x: i32,
    y: i32,
}

struct Worm {
    mi: ModeInfo,
    xsize: i32,
    ysize: i32,
    wormlength: usize,
    /// How many colours are in play, and how many worms.
    nc: usize,
    nw: usize,
    circsize: i32,
    worms: Vec<WormStuff>,
    /// One bucket of rectangles per colour, drawn in one go at the end of the
    /// frame so the colour is only set once each.
    rects: Vec<Vec<XRectangle>>,
    chromo: usize,
    sintab: [f64; SEGMENTS],
    costab: [f64; SEGMENTS],
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut sintab = [0.0; SEGMENTS];
    let mut costab = [0.0; SEGMENTS];
    for i in 0..SEGMENTS {
        let a = i as f64 * 2.0 * std::f64::consts::PI / SEGMENTS as f64;
        sintab[i] = a.sin();
        costab[i] = a.cos();
    }

    let mut st = Worm {
        mi,
        xsize: 0,
        ysize: 0,
        wormlength: 1,
        nc: 1,
        nw: 1,
        circsize: 1,
        worms: Vec::new(),
        rects: Vec::new(),
        chromo: 0,
        sintab,
        costab,
    };
    st.restart(d);
    Box::new(st)
}

/// `IRINT(x)`: round away from zero, which is what upstream uses in place of
/// `rint`.
fn irint(x: f64) -> i32 {
    (if x > 0.0 { x + 0.5 } else { x - 0.5 }) as i32
}

impl Worm {
    fn restart(&mut self, d: &mut Dpy) {
        let npixels = self.mi.npixels();
        self.nc = if npixels <= 2 {
            2
        } else {
            npixels.min(NUM_COLORS) as usize
        };

        let mut nw = self.mi.count;
        if nw < -MIN_WORMS {
            nw = nrand(-nw - MIN_WORMS + 1) + MIN_WORMS;
        } else if nw < MIN_WORMS {
            nw = MIN_WORMS;
        }
        self.nw = nw as usize;

        self.xsize = self.mi.width;
        self.ysize = self.mi.height;
        if npixels > 2 {
            self.chromo = nrand(npixels) as usize;
        }

        let size = self.mi.size;
        self.circsize = if size < -MIN_SIZE {
            nrand(-size - MIN_SIZE + 1) + MIN_SIZE
        } else if size < MIN_SIZE {
            MIN_SIZE
        } else {
            size
        };

        // Fudged to something reasonable, as upstream puts it.
        self.wormlength =
            (((self.xsize + self.ysize) as f64).sqrt() as i32 * self.mi.cycles / 8).max(1) as usize;

        self.rects = (0..self.nc).map(|_| Vec::new()).collect();
        self.worms = (0..self.nw)
            .map(|_| {
                let dir = nrand(SEGMENTS as i32) as usize;
                // Upstream rolls a second heading here, for its 3D mode.
                let _dir2 = nrand(SEGMENTS as i32);
                WormStuff {
                    circ: vec![
                        XPoint {
                            x: self.xsize / 2,
                            y: self.ysize / 2,
                        };
                        self.wormlength
                    ],
                    dir,
                    tail: 0,
                    x: self.xsize / 2,
                    y: self.ysize / 2,
                }
            })
            .collect();

        d.clear_window();
    }

    fn step(&mut self, d: &mut Dpy, which: usize, color: usize) {
        let (xsize, ysize, circsize) = (self.xsize, self.ysize, self.circsize);
        let wormlength = self.wormlength;

        let mut tail = self.worms[which].tail + 1;
        if tail == wormlength {
            tail = 0;
        }
        self.worms[which].tail = tail;

        // The cell the tail is leaving, to be cleared below.
        let old = self.worms[which].circ[tail];

        let dir = self.worms[which].dir;
        let dir = if lrand() & 1 == 1 {
            (dir + 1) % SEGMENTS
        } else {
            (dir + SEGMENTS - 1) % SEGMENTS
        };
        self.worms[which].dir = dir;

        let x = (self.worms[which].x + irint(circsize as f64 * self.costab[dir]) + xsize)
            .rem_euclid(xsize);
        let y = (self.worms[which].y + irint(circsize as f64 * self.sintab[dir]) + ysize)
            .rem_euclid(ysize);

        self.worms[which].circ[tail] = XPoint { x, y };
        self.worms[which].x = x;
        self.worms[which].y = y;

        d.win()
            .clear_area(self.mi.black, old.x, old.y, circsize, circsize);

        self.rects[color].push(XRectangle {
            x,
            y,
            width: circsize,
            height: circsize,
        });
    }
}

impl Screenhack for Worm {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for r in self.rects.iter_mut() {
            r.clear();
        }

        let npixels = self.mi.npixels();
        for i in 0..self.nw {
            let color = if npixels > 2 {
                (i + self.chromo) % self.nc
            } else {
                0
            };
            self.step(d, i, color);
        }

        if npixels > 2 {
            for i in 0..self.nc {
                let c = self.mi.pixel(i);
                self.mi.gc.set_foreground(c);
                d.win().fill_rectangles(&self.mi.gc, &self.rects[i]);
            }
        } else {
            let white = self.mi.white;
            self.mi.gc.set_foreground(white);
            d.win().fill_rectangles(&self.mi.gc, &self.rects[0]);
        }

        self.chromo += 1;
        if self.chromo == self.nc {
            self.chromo = 0;
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
    "*delay: 17000",
    "*count: -20",
    "*cycles: 10",
    "*size: -3",
    "*ncolors: 150",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "17000").inverted(),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "150"),
    Opt::spin("count", "Count", -100.0, 100.0, "-20"),
    Opt::spin("size", "Size", -20.0, 20.0, "-3"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "worm",
    label: "Worm",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Brad Taylor, Dave Lemke, Boris Putanec, and Henrik Theiling",
        year: "1991",
        video: Some("https://www.youtube.com/watch?v=-S26J2Ja11g"),
        blurb: "Multicoloured worms that crawl around the screen.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
