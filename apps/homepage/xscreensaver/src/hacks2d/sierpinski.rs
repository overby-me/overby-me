//! Port of `hacks/sierpinski.c`.
//!
//! ```text
//! Copyright (c) 1996 by Desmond Daignault
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
//! 10-May-1997: Jamie Zawinski <jwz@jwz.org> compatible with xscreensaver
//! 05-Sep-1996: Desmond Daignault Datatimes Incorporated
//! ```
//!
//! The chaos game: pick three or four corners, then repeatedly step halfway
//! from where you are to a randomly chosen corner and plot the point. The
//! Sierpinski gasket falls out. Points are coloured by which corner drew them,
//! which is what makes the structure legible.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

/// Upstream's `MAXCORNERS`. Three or four; the rest of the code is written for
/// exactly those two cases.
const MAX_CORNERS: usize = 4;

struct Sierpinski {
    mi: ModeInfo,
    time: i32,
    px: i32,
    py: i32,
    total_npoints: usize,
    corners: usize,
    colors: [usize; MAX_CORNERS],
    /// One point buffer per corner, drawn in that corner's colour.
    points: [Vec<XPoint>; MAX_CORNERS],
    vertex: [XPoint; MAX_CORNERS],
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // BRIGHT_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Bright);

    let total_npoints = mi.count.max(1) as usize;
    let corners = match mi.size {
        3 | 4 => mi.size as usize,
        _ => (lrand() & 1) as usize + 3,
    };

    let mut st = Sierpinski {
        mi,
        time: 0,
        px: 0,
        py: 0,
        total_npoints,
        corners,
        colors: [0; MAX_CORNERS],
        points: Default::default(),
        vertex: [XPoint::default(); MAX_CORNERS],
    };
    st.startover(d);
    Box::new(st)
}

impl Sierpinski {
    fn startover(&mut self, d: &mut Dpy) {
        let n = self.mi.npixels();
        if n > 2 {
            let n = n as usize;
            // Spread the corners' colours around the map so the three or four
            // point clouds stay distinguishable.
            if self.corners == 3 {
                self.colors[0] = nrand(n as i32) as usize;
                self.colors[1] =
                    (self.colors[0] + n / 7 + nrand(2 * n as i32 / 7 + 1) as usize) % n;
                self.colors[2] =
                    (self.colors[0] + 4 * n / 7 + nrand(2 * n as i32 / 7 + 1) as usize) % n;
            } else {
                self.colors[0] = nrand(n as i32) as usize;
                self.colors[1] = (self.colors[0] + n / 7 + nrand(n as i32 / 7 + 1) as usize) % n;
                self.colors[2] =
                    (self.colors[0] + 3 * n / 7 + nrand(n as i32 / 7 + 1) as usize) % n;
                self.colors[3] =
                    (self.colors[0] + 5 * n / 7 + nrand(n as i32 / 7 + 1) as usize) % n;
            }
        }
        for j in 0..self.corners {
            self.vertex[j] = XPoint {
                x: nrand(self.mi.width),
                y: nrand(self.mi.height),
            };
        }
        self.px = nrand(self.mi.width);
        self.py = nrand(self.mi.height);
        self.time = 0;

        d.clear_window();
    }
}

impl Screenhack for Sierpinski {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.mi.npixels() <= 2 {
            self.mi.gc.set_foreground(self.mi.white);
        }
        for buf in self.points.iter_mut() {
            buf.clear();
        }

        for _ in 0..self.total_npoints {
            let v = nrand(self.corners as i32) as usize;
            self.px = (self.px + self.vertex[v].x) / 2;
            self.py = (self.py + self.vertex[v].y) / 2;
            self.points[v].push(XPoint {
                x: self.px,
                y: self.py,
            });
        }

        for i in 0..self.corners {
            if self.mi.npixels() > 2 {
                let color = self.mi.pixel(self.colors[i]);
                self.mi.gc.set_foreground(color);
            }
            d.win().draw_points(&self.mi.gc, &self.points[i]);
        }

        self.time += 1;
        if self.time >= self.mi.cycles {
            self.startover(d);
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
    "*delay: 400000",
    "*count: 2000",
    "*cycles: 100",
    "*ncolors: 64",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "delay",
        "Frame rate",
        0.0,
        1_000_000.0,
        10_000.0,
        0,
        "400000",
    )
    .inverted(),
    Opt::slider("count", "Points", 10.0, 10000.0, 10.0, 0, "2000"),
    Opt::slider("cycles", "Timeout", 0.0, 1000.0, 10.0, 0, "100"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "sierpinski",
    label: "Sierpinski",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Desmond Daignault",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=m0zdPWuFhjA"),
        blurb: "Sierpinski's triangle, drawn by the chaos game.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
