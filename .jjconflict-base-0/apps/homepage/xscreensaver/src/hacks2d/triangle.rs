//! Port of `hacks/triangle.c`.
//!
//! ```text
//! Copyright (c) 1995 by Tobias Gloth
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
//! 10-Mar-96: re-arranged and re-formatted the code for appearance and
//!            to make common subroutines.  Simplified.
//!            Ron Hitchens <ron@idiom.com>
//! 07-Mar-96: Removed internal delay code, set MI_PAUSE(mi) for inter-scene
//!            delays.  No other delays are needed here.
//!            Made pause time sensitive to value of cycles (in 10ths of a
//!            second).  Removed (hopefully) all references to globals.
//!            Ron Hitchens <ron@idiom.com>
//! 27-Feb-96: Undid the changes listed below.  Added ModeInfo argument.
//!            Implemented delay between scenes using the MI_PAUSE(mi)
//!            scheme.  Ron Hitchens <ron@idiom.com>
//! 27-Dec-95: Ron Hitchens <ron@idiom.com>
//!            Modified logic of draw_triangle() to provide a delay
//!            (sensitive to the value of cycles) between each iteration.
//! 03-Nov-95: Many changes (hopefully some good ones) by David Bagley
//! 01-Oct-95: Written by Tobias Gloth
//! ```
//!
//! Random mountain ranges by midpoint displacement on a triangular grid. Each
//! pass halves the spacing and halves the size of the random offset added to
//! each new midpoint, so the terrain gains detail without gaining relief; the
//! whole mesh is redrawn after each pass, so you watch the range sharpen. A
//! face's colour comes from how steep it is, which is what reads as shading.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint, frand};

const MAX_STEPS: usize = 8;
const MAX_SIZE: i32 = 1 << MAX_STEPS;
const MAX_LEVELS: usize = 1000;

const DELTA: f64 = 0.4;
const LEFT: f64 = -0.25;
const RIGHT: f64 = 1.25;
const TOP: f64 = 0.3;
const BOTTOM: f64 = 1.0;
/// Just the right shade of blue, for the sea.
const BLUE: usize = 45;

/// `DISPLACE(h, d)`: the average of two heights, nudged by up to `d` either
/// way.
fn displace(h: i32, d: i32) -> i32 {
    // Upstream keeps the random offset fractional and truncates only at the
    // assignment, which rounds negative results the other way.
    ((h / 2) as f64 + frand((2 * d + 1) as f64) - d as f64) as i32
}

struct Triangle {
    mi: ModeInfo,
    size: i32,
    steps: usize,
    stage: i32,
    init_now: bool,
    i: i32,
    j: i32,
    d: i32,
    level: [i32; MAX_LEVELS],
    xpos: Vec<i32>,
    ypos: Vec<i32>,
    /// The height field, a triangular array flattened row by row.
    h: Vec<i32>,
    row: Vec<usize>,
    delta: [i32; MAX_STEPS],
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut st = Triangle {
        mi,
        size: 1,
        steps: 1,
        stage: -1,
        init_now: true,
        i: 0,
        j: 0,
        d: 2,
        level: [0; MAX_LEVELS],
        xpos: Vec::new(),
        ypos: Vec::new(),
        h: Vec::new(),
        row: Vec::new(),
        delta: [0; MAX_STEPS],
    };
    st.restart(d);
    Box::new(st)
}

impl Triangle {
    fn get_h(&self, i: i32, j: i32) -> i32 {
        self.h[self.row[i as usize] + j as usize]
    }

    fn set_h(&mut self, i: i32, j: i32, v: i32) {
        let at = self.row[i as usize] + j as usize;
        self.h[at] = v;
    }

    /// `level[MAX(h, 0)]`. Upstream indexes the table directly, which runs off
    /// the end on a screen tall enough to push the height field past a
    /// thousand; clamping is the same picture without the overrun.
    fn level_of(&self, h: i32) -> i32 {
        self.level[h.clamp(0, MAX_LEVELS as i32 - 1) as usize]
    }

    fn restart(&mut self, d: &mut Dpy) {
        self.mi.width = d.width();
        self.mi.height = d.height();
        self.init_now = true;

        d.clear_window();

        // The largest grid whose triangles are still wider than a fifth of the
        // window. The floor at one step is ours: upstream would shift by a
        // negative amount on a window under five pixels wide.
        self.steps = MAX_STEPS;
        loop {
            self.steps -= 1;
            self.size = 1 << self.steps;
            if self.size * 5 <= self.mi.width || self.steps == 0 {
                break;
            }
        }

        // Row i of the height field holds size + 1 - i entries.
        self.row = Vec::with_capacity(self.size as usize + 1);
        let mut at = 0usize;
        for i in 0..=self.size as usize {
            self.row.push(at);
            at += self.size as usize + 1 - i;
        }
        self.h = vec![0; at];

        self.stage = -1;
        let dim = self.mi.width.min(self.mi.height);

        self.xpos = (0..2 * self.size + 1)
            .map(|i| {
                ((i as f64 / (2 * self.size) as f64 * (RIGHT - LEFT) + LEFT) * dim as f64) as i32
                    + (self.mi.width - dim) / 2
            })
            .collect();

        self.ypos = (0..self.size + 1)
            .map(|i| {
                ((i as f64 / self.size as f64 * (BOTTOM - TOP) + TOP) * dim as f64) as i32
                    + (self.mi.height - dim) / 2
            })
            .collect();

        for i in 0..self.steps {
            self.delta[i] = ((DELTA * dim as f64) as i32) >> i;
        }

        let one = self.delta[0];
        if one > 0 {
            for i in 0..MAX_LEVELS {
                self.level[i] = (i * i) as i32 / one;
            }
        }
    }

    fn draw_atriangle(
        &mut self,
        d: &mut Dpy,
        p: &[XPoint; 3],
        y0: i32,
        y1: i32,
        y2: i32,
        dinv: f64,
    ) {
        let ncolors = self.mi.npixels();
        if ncolors > 2 {
            let dmin = y0.min(y1).min(y2);
            let dmax = y0.max(y1).max(y2);

            // Flat and at sea level is water; otherwise the steeper the face,
            // the further along the map its colour sits.
            let color = if dmax == 0 {
                BLUE
            } else {
                let steep = (dinv * (dmax - dmin) as f64).atan();
                (ncolors as f64 - (ncolors as f64 / std::f64::consts::FRAC_PI_2 * steep)) as usize
            };

            let c = self.mi.pixel(color);
            self.mi.gc.set_foreground(c);
            d.win().fill_polygon(&self.mi.gc, p);
        } else {
            // Mono: fill with black first, so the face behind is hidden.
            let black = self.mi.black;
            self.mi.gc.set_foreground(black);
            d.win().fill_polygon(&self.mi.gc, p);
            let white = self.mi.white;
            self.mi.gc.set_foreground(white);
            d.win()
                .draw_line(&self.mi.gc, p[0].x, p[0].y, p[1].x, p[1].y);
            d.win()
                .draw_line(&self.mi.gc, p[1].x, p[1].y, p[2].x, p[2].y);
            d.win()
                .draw_line(&self.mi.gc, p[2].x, p[2].y, p[0].x, p[0].y);
        }
    }

    /// The upward-pointing face at the current cell.
    fn calc_points1(&self, d: i32) -> (i32, i32, i32, [XPoint; 3]) {
        let (i, j) = (self.i, self.j);
        let y0 = self.level_of(self.get_h(i, j));
        let y1 = self.level_of(self.get_h(i + d, j));
        let y2 = self.level_of(self.get_h(i, j + d));
        let p = [
            XPoint {
                x: self.xpos[(2 * i + j) as usize],
                y: self.ypos[j as usize] - y0,
            },
            XPoint {
                x: self.xpos[(2 * (i + d) + j) as usize],
                y: self.ypos[j as usize] - y1,
            },
            XPoint {
                x: self.xpos[(2 * i + (j + d)) as usize],
                y: self.ypos[(j + d) as usize] - y2,
            },
        ];
        (y0, y1, y2, p)
    }

    /// The downward-pointing face that fills the gap beside it.
    fn calc_points2(&self, d: i32) -> (i32, i32, i32, [XPoint; 3]) {
        let (i, j) = (self.i, self.j);
        let y0 = self.level_of(self.get_h(i + d, j));
        let y1 = self.level_of(self.get_h(i + d, j + d));
        let y2 = self.level_of(self.get_h(i, j + d));
        let p = [
            XPoint {
                x: self.xpos[(2 * (i + d) + j) as usize],
                y: self.ypos[j as usize] - y0,
            },
            XPoint {
                x: self.xpos[(2 * (i + d) + (j + d)) as usize],
                y: self.ypos[(j + d) as usize] - y1,
            },
            XPoint {
                x: self.xpos[(2 * i + (j + d)) as usize],
                y: self.ypos[(j + d) as usize] - y2,
            },
        ];
        (y0, y1, y2, p)
    }

    fn draw_mesh(&mut self, dpy: &mut Dpy, d: i32, mut count: i32) {
        let mut first = true;
        let dinv = 0.2 / d as f64;

        if self.j == 0 && self.i == 0 {
            // Sky: everything above the top of the range.
            let (w, y2) = (self.mi.width, self.ypos[0]);
            let black = self.mi.black;
            self.mi.gc.set_foreground(black);
            dpy.win().fill_rectangle(&self.mi.gc, 0, 0, w, y2);
        }

        while self.j < self.size && count > 0 {
            if !first {
                self.i = 0;
            }
            first = false;

            while self.i < MAX_SIZE - self.j && count > 0 {
                if self.i + self.j < self.size {
                    let (y0, y1, y2, p) = self.calc_points1(d);
                    self.draw_atriangle(dpy, &p, y0, y1, y2, dinv);
                }
                if self.i + self.j + d < self.size {
                    let (y0, y1, y2, p) = self.calc_points2(d);
                    self.draw_atriangle(dpy, &p, y0, y1, y2, dinv);
                }
                self.i += d;
                count -= 1;
            }

            if count != 0 {
                self.j += d;
            }
        }

        if self.j == self.size {
            self.init_now = true;
        }
    }
}

impl Screenhack for Triangle {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if !self.init_now {
            let (step, count) = (self.d / 2, MAX_SIZE / self.d);
            self.draw_mesh(d, step, count);

            // The scene is complete: start the next one from a clean screen
            // and a fresh palette.
            if self.init_now && self.stage == -1 {
                d.clear_window();
                if !self.mi.is_mono() {
                    let n = d.res.int("ncolors").max(1) as usize;
                    self.mi.remake_colors(ColorScheme::Smooth, n);
                }
            }
            return self.mi.delay;
        }

        if self.delta[0] > 0 {
            self.stage += 1;
            if self.stage == 0 {
                let one = self.delta[0];
                let (size, v) = (self.size, displace(0, one).max(0));
                self.set_h(0, 0, v);
                let v = displace(0, one).max(0);
                self.set_h(size, 0, v);
                let v = displace(0, one).max(0);
                self.set_h(0, size, v);
            } else {
                let step = 2 << (self.steps - self.stage as usize);
                let half = step / 2;
                let delta = self.delta[self.stage as usize - 1];

                let mut i = 0;
                while i < self.size {
                    let mut j = 0;
                    while j < self.size - i {
                        let v = displace(self.get_h(i, j) + self.get_h(i + step, j), delta);
                        self.set_h(i + half, j, v);
                        let v = displace(self.get_h(i, j) + self.get_h(i, j + step), delta);
                        self.set_h(i, j + half, v);
                        let v = displace(self.get_h(i + step, j) + self.get_h(i, j + step), delta);
                        self.set_h(i + half, j + half, v);
                        j += step;
                    }

                    self.init_now = false;
                    self.i = 0;
                    self.j = 0;
                    self.d = step;
                    i += step;
                }
            }
        }

        if self.stage == self.steps as i32 {
            self.stage = -1;
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
    "*ncolors: 128",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "128"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "triangle",
    label: "Triangle",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tobias Gloth",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=GXrzjY-Flro"),
        blurb: "Random mountain ranges by iterative subdivision of triangles.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
