//! Port of `hacks/demon.c`.
//!
//! ```text
//! demon --- David Griffeath's cellular automata
//!
//! Copyright (c) 1995 by David Bagley.
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
//! 16-Apr-1997: -neighbors 3, 9 (not sound mathematically), 12, and 8 added
//! 30-May-1996: Ron Hitchens <ron@idiom.com>
//!            Fixed memory management that caused leaks
//! 14-Apr-1996: -neighbors 6 runtime-time option added
//! 21-Aug-1995: Coded from A.K. Dewdney's "Computer Recreations", Scientific
//!              American Magazine" Aug 1989 pp 102-105.  Also very similar
//!              to hodgepodge machine described in A.K. Dewdney's "Computer
//!              Recreations", Scientific American Magazine" Aug 1988
//!              pp 104-107.  Also used life.c as a guide.
//! ```
//!
//! Every cell holds one of a dozen or two states, numbered in a ring. The whole
//! rule is: if any neighbour is holding the state one past yours, take it. That
//! is all. A cell can only ever move forward, and only by one, and only when
//! something next to it has already got there.
//!
//! From a random field, that turns into spirals. A cell in state 5 next to a 6
//! becomes 6, and now it is the thing its own 4-neighbours are waiting for, so
//! the change walks backwards through the ring as a wave. Where two waves meet
//! at an angle, one end of the front gets stuck and the front winds around it,
//! and a stuck end that keeps winding is a spiral. The random start is mostly
//! noise, then curls, then the curls take over the screen.
//!
//! The lattice is not fixed. Cells can be squares with four or eight
//! neighbours, hexagons with six, or triangles with three, nine or twelve, and
//! upstream rolls one of the six at startup. Squares with four make blocky
//! square-cornered spirals; hexagons make round ones.
//!
//! Drawing is spread out. A generation is computed in one frame, which lists
//! only the cells that changed, and then one frame per state draws that state's
//! cells in one colour. Cells that did not change are never touched.
//!
//! Two upstream paths are not here, and neither is reachable in an xscreensaver
//! build. The stipple patterns for a display with fewer than eleven colours are
//! `#ifndef HAVE_JWXYZ`, so upstream's own modern builds already draw those
//! cells solid white, which is what this does. The redraw-on-expose path is
//! `#ifndef STANDALONE`, so nothing in the screensaver build ever switches it
//! on.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{
    About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint, XRectangle,
};

/// Below this many colours upstream reaches for stipple patterns instead.
const NUMSTIPPLES: i32 = 11;
const MINSTATES: i32 = 2;
const MINGRIDSIZE: i32 = 5;
const MINSIZE: i32 = 4;

/// The lattices, and the number of states each one gets by default.
const NEIGHBORHOODS: [i32; 6] = [3, 4, 6, 8, 9, 12];
const STATE_COUNTS: [i32; 6] = [12, 16, 18, 20, 22, 24];

/// `hexagonUnit` from `automata.h`, in `CoordModePrevious` form: the first
/// point is placed absolutely and the rest are steps from the one before.
const HEXAGON_UNIT: [XPoint; 6] = [
    XPoint { x: 0, y: 0 },
    XPoint { x: 1, y: 1 },
    XPoint { x: 0, y: 2 },
    XPoint { x: -1, y: 1 },
    XPoint { x: -1, y: -1 },
    XPoint { x: 0, y: -2 },
];

/// `triangleUnit`, likewise. The two rows are the left-pointing and the
/// right-pointing triangle.
const TRIANGLE_UNIT: [[XPoint; 3]; 2] = [
    [
        XPoint { x: 0, y: 0 },
        XPoint { x: 1, y: -1 },
        XPoint { x: 0, y: 2 },
    ],
    [
        XPoint { x: 0, y: 0 },
        XPoint { x: -1, y: 1 },
        XPoint { x: 0, y: -2 },
    ],
];

/// Turn a `CoordModePrevious` polygon into absolute points.
fn absolute(shape: &[XPoint], x: i32, y: i32) -> Vec<XPoint> {
    let mut out = Vec::with_capacity(shape.len());
    let (mut cx, mut cy) = (x, y);
    out.push(XPoint { x: cx, y: cy });
    for p in &shape[1..] {
        cx += p.x;
        cy += p.y;
        out.push(XPoint { x: cx, y: cy });
    }
    out
}

struct Demon {
    mi: ModeInfo,
    generation: i32,
    /// The size of one cell, and where the grid starts.
    xs: i32,
    ys: i32,
    xb: i32,
    yb: i32,
    nrows: i32,
    ncols: i32,
    width: i32,
    height: i32,
    states: i32,
    /// Which state is being drawn this frame. Once it reaches `states` the next
    /// frame computes a generation instead.
    state: i32,
    /// The cells that changed into each state, waiting to be drawn.
    cell_list: Vec<Vec<XPoint>>,
    oldcell: Vec<u8>,
    newcell: Vec<u8>,
    /// The lattice in use, and the one asked for. Upstream's default of zero
    /// matches nothing in the table, which is how it comes to roll a random
    /// lattice at every restart.
    neighbors: i32,
    neighbors_requested: i32,
    hexagon: [XPoint; 6],
    triangle: [[XPoint; 3]; 2],
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // UNIFORM_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Uniform);
    let mut st = Demon {
        mi,
        generation: 0,
        xs: 1,
        ys: 1,
        xb: 0,
        yb: 0,
        nrows: 2,
        ncols: 2,
        width: d.width(),
        height: d.height(),
        states: MINSTATES,
        state: 0,
        cell_list: Vec::new(),
        oldcell: Vec::new(),
        newcell: Vec::new(),
        neighbors: 4,
        neighbors_requested: d.res.int("neighbors"),
        hexagon: HEXAGON_UNIT,
        triangle: TRIANGLE_UNIT,
    };
    st.restart(d);
    Box::new(st)
}

impl Demon {
    /// `init_demon`, which is also how a field that has run its course is
    /// replaced.
    fn restart(&mut self, d: &mut Dpy) {
        let mut size = self.mi.size;
        if self.mi.width < 100 || self.mi.height < 100 {
            size = self.mi.width.min(self.mi.height); // Tiny window.
        }

        self.generation = 0;

        // Upstream walks the table looking for the requested neighbourhood and
        // rolls a random one when it falls off the end, which is what a
        // neighbours setting of zero does.
        let mut nk = NEIGHBORHOODS.len() - 1;
        for (i, n) in NEIGHBORHOODS.iter().enumerate() {
            if self.neighbors_requested == *n {
                nk = i;
                break;
            }
            if i == NEIGHBORHOODS.len() - 1 {
                nk = nrand(NEIGHBORHOODS.len() as i32) as usize;
                break;
            }
        }
        self.neighbors = NEIGHBORHOODS[nk];

        self.states = self.mi.count;
        if self.states < -MINSTATES {
            self.states = nrand(-self.states - MINSTATES + 1) + MINSTATES;
        } else if self.states < MINSTATES {
            self.states = STATE_COUNTS[nk];
        }
        self.cell_list = vec![Vec::new(); self.states as usize];

        self.state = 0;

        self.width = self.mi.width;
        self.height = self.mi.height;

        if self.neighbors == 6 {
            self.width = self.width.max(8);
            self.height = self.height.max(8);
            self.ys = self.cell_size(size);
            self.xs = self.ys;
            let nccols = (self.width / self.xs - 2).max(2);
            let ncrows = (self.height / self.ys - 1).max(4);
            self.ncols = nccols / 2;
            self.nrows = 2 * (ncrows / 4);
            self.xb = (self.width - self.xs * nccols) / 2 + self.xs / 2;
            self.yb = (self.height - self.ys * (ncrows / 2) * 2) / 2 + self.ys - 2;
            let (xs, ys) = (self.xs, self.ys);
            for (out, unit) in self.hexagon.iter_mut().zip(HEXAGON_UNIT) {
                out.x = (xs - 1) * unit.x;
                out.y = ((ys - 1) * unit.y / 2) * 4 / 3;
            }
        } else if self.neighbors == 4 || self.neighbors == 8 {
            self.ys = self.cell_size(size);
            self.xs = self.ys;
            self.ncols = (self.width / self.xs).max(2);
            self.nrows = (self.height / self.ys).max(2);
            self.xb = (self.width - self.xs * self.ncols) / 2;
            self.yb = (self.height - self.ys * self.nrows) / 2;
        } else {
            // Triangles.
            self.width = self.width.max(2);
            self.height = self.height.max(2);
            self.ys = self.cell_size(size);
            self.xs = (1.52 * self.ys as f64) as i32;
            self.ncols = ((self.width / self.xs - 1).max(2) / 2) * 2;
            self.nrows = ((self.height / self.ys - 1).max(2) / 2) * 2;
            self.xb = (self.width - self.xs * self.ncols) / 2 + self.xs / 2;
            self.yb = (self.height - self.ys * self.nrows) / 2 + self.ys / 2;
            let (xs, ys) = (self.xs, self.ys);
            for (row, unit_row) in self.triangle.iter_mut().zip(TRIANGLE_UNIT) {
                for (out, unit) in row.iter_mut().zip(unit_row) {
                    out.x = (xs - 2) * unit.x;
                    out.y = (ys - 2) * unit.y;
                }
            }
        }

        self.mi.clear_window(d);

        let cells = (self.ncols * self.nrows) as usize;
        self.oldcell = vec![0; cells];
        self.newcell = vec![0; cells];

        self.random_soup();
    }

    /// How big one cell is, from `size`: a negative value means pick at random
    /// up to that many pixels, zero means fill the window, positive means that
    /// many pixels. Never bigger than a fifth of the shorter side.
    fn cell_size(&self, size: i32) -> i32 {
        let fit = MINSIZE.max(self.width.min(self.height) / MINGRIDSIZE);
        if size < -MINSIZE {
            nrand((-size).min(fit) - MINSIZE + 1) + MINSIZE
        } else if size < MINSIZE {
            if size == 0 { fit } else { MINSIZE }
        } else {
            size.min(fit)
        }
    }

    /// Fill the grid with noise, and queue every cell to be drawn.
    fn random_soup(&mut self) {
        for row in 0..self.nrows {
            for col in 0..self.ncols {
                // Upstream truncates the random number to a byte before taking
                // the remainder, so the states are very slightly unequal.
                let state = (lrand() as u8) % (self.states as u8);
                self.oldcell[(col + row * self.ncols) as usize] = state;
                self.cell_list[state as usize].push(XPoint { x: col, y: row });
            }
        }
    }

    /// The rule: if the neighbour at `(k, l)` holds the state one past the one
    /// at `(i, j)`, take it.
    fn absorb(&mut self, i: i32, j: i32, k: i32, l: i32) {
        let me = (i + j * self.ncols) as usize;
        let them = (k + l * self.ncols) as usize;
        if self.oldcell[them] as i32 == (self.oldcell[me] as i32 + 1) % self.states {
            self.newcell[me] = self.oldcell[them];
        }
    }

    fn east(&self, i: i32) -> i32 {
        if i + 1 == self.ncols { 0 } else { i + 1 }
    }
    fn west(&self, i: i32) -> i32 {
        if i == 0 { self.ncols - 1 } else { i - 1 }
    }
    fn north(&self, j: i32) -> i32 {
        if j == 0 { self.nrows - 1 } else { j - 1 }
    }
    fn south(&self, j: i32) -> i32 {
        if j + 1 == self.nrows { 0 } else { j + 1 }
    }
    /// Two rows up, wrapping.
    fn north2(&self, j: i32) -> i32 {
        match j {
            0 => self.nrows - 2,
            1 => self.nrows - 1,
            _ => j - 2,
        }
    }
    /// Two rows down, wrapping.
    fn south2(&self, j: i32) -> i32 {
        if j + 1 == self.nrows {
            1
        } else if j + 2 == self.nrows {
            0
        } else {
            j + 2
        }
    }

    fn step_hexagon(&mut self) {
        for j in 0..self.nrows {
            for i in 0..self.ncols {
                // A hexagon row is offset from the one above it, so which
                // column the diagonal neighbours sit in alternates by row.
                let up_down = if j & 1 == 0 { self.east(i) } else { i };
                let down_up = if j & 1 != 0 { self.west(i) } else { i };
                self.absorb(i, j, up_down, self.north(j)); // NE
                self.absorb(i, j, self.east(i), j); // E
                self.absorb(i, j, up_down, self.south(j)); // SE
                self.absorb(i, j, down_up, self.south(j)); // SW
                self.absorb(i, j, self.west(i), j); // W
                self.absorb(i, j, down_up, self.north(j)); // NW
            }
        }
    }

    fn step_square(&mut self) {
        for j in 0..self.nrows {
            for i in 0..self.ncols {
                self.absorb(i, j, i, self.north(j));
                self.absorb(i, j, self.east(i), j);
                self.absorb(i, j, i, self.south(j));
                self.absorb(i, j, self.west(i), j);
            }
        }
        if self.neighbors == 8 {
            for j in 0..self.nrows {
                for i in 0..self.ncols {
                    self.absorb(i, j, self.east(i), self.north(j));
                    self.absorb(i, j, self.east(i), self.south(j));
                    self.absorb(i, j, self.west(i), self.south(j));
                    self.absorb(i, j, self.west(i), self.north(j));
                }
            }
        }
    }

    fn step_triangle(&mut self) {
        for j in 0..self.nrows {
            for i in 0..self.ncols {
                // A triangle has one side facing sideways, and which way it
                // faces alternates like a chessboard.
                if (i + j) % 2 != 0 {
                    self.absorb(i, j, self.west(i), j);
                } else {
                    self.absorb(i, j, self.east(i), j);
                }
                self.absorb(i, j, i, self.north(j));
                self.absorb(i, j, i, self.south(j));
            }
        }
        if self.neighbors == 9 || self.neighbors == 12 {
            for j in 0..self.nrows {
                for i in 0..self.ncols {
                    self.absorb(i, j, i, self.north2(j));
                    self.absorb(i, j, i, self.south2(j));
                    self.absorb(i, j, self.west(i), self.north(j));
                    self.absorb(i, j, self.east(i), self.north(j));
                    self.absorb(i, j, self.west(i), self.south(j));
                    self.absorb(i, j, self.east(i), self.south(j));
                }
            }
            if self.neighbors == 12 {
                for j in 0..self.nrows {
                    for i in 0..self.ncols {
                        if (i + j) % 2 != 0 {
                            self.absorb(i, j, self.west(i), self.north2(j));
                            self.absorb(i, j, self.west(i), self.south2(j));
                            self.absorb(i, j, self.east(i), j);
                        } else {
                            self.absorb(i, j, self.east(i), self.north2(j));
                            self.absorb(i, j, self.east(i), self.south2(j));
                            self.absorb(i, j, self.west(i), j);
                        }
                    }
                }
            }
        }
    }

    /// Pick the colour a state is drawn in. State zero is the background, so it
    /// erases; anything else takes a slice of the colormap.
    fn set_state_color(&mut self) {
        let npixels = self.mi.npixels();
        if self.state == 0 {
            self.mi.gc.set_foreground(self.mi.black);
        } else if npixels >= NUMSTIPPLES {
            let i = ((self.state - 1) * npixels / (self.states - 1)) % npixels;
            self.mi.gc.set_foreground(self.mi.pixel(i as usize));
        } else {
            // Where upstream would stipple, on a display it has no stipples for.
            self.mi.gc.set_foreground(self.mi.white);
        }
    }

    /// Draw every cell that changed into the current state.
    fn draw_state(&mut self, d: &mut Dpy) {
        self.set_state_color();
        let cells = std::mem::take(&mut self.cell_list[self.state as usize]);

        if self.neighbors == 6 {
            for c in &cells {
                let ccol = 2 * c.x + i32::from(c.y & 1 == 0);
                let crow = 2 * c.y;
                let (x, y) = (self.xb + ccol * self.xs, self.yb + crow * self.ys);
                if self.xs == 1 && self.ys == 1 {
                    d.win().draw_point(&self.mi.gc, x, y);
                } else {
                    let pts = absolute(&self.hexagon, x, y);
                    d.win().fill_polygon(&self.mi.gc, &pts);
                }
            }
        } else if self.neighbors == 4 || self.neighbors == 8 {
            // One call for the lot, which is what upstream builds the rectangle
            // list for.
            let rects: Vec<XRectangle> = cells
                .iter()
                .map(|c| XRectangle {
                    x: self.xb + c.x * self.xs,
                    y: self.yb + c.y * self.ys,
                    width: self.xs - i32::from(self.xs > 3),
                    height: self.ys - i32::from(self.ys > 3),
                })
                .collect();
            d.win().fill_rectangles(&self.mi.gc, &rects);
        } else {
            for c in &cells {
                let orient = ((c.x + c.y) % 2) as usize; // 0 left, 1 right.
                let x = self.xb + c.x * self.xs;
                let y = self.yb + c.y * self.ys;
                if self.xs <= 3 || self.ys <= 3 {
                    let nudge = if orient != 0 { -1 } else { 1 };
                    d.win().draw_point(&self.mi.gc, nudge + x, y);
                } else {
                    let x = if orient != 0 {
                        x + (self.xs / 2 - 1)
                    } else {
                        x - (self.xs / 2 - 1)
                    };
                    let pts = absolute(&self.triangle[orient], x, y);
                    d.win().fill_polygon(&self.mi.gc, &pts);
                }
            }
        }
    }
}

impl Screenhack for Demon {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.state >= self.states {
            self.newcell.copy_from_slice(&self.oldcell);

            if self.neighbors == 6 {
                self.step_hexagon();
            } else if self.neighbors == 4 || self.neighbors == 8 {
                self.step_square();
            } else {
                self.step_triangle();
            }

            // Only the cells that actually moved are queued for drawing; the
            // rest of the picture is simply left alone.
            for j in 0..self.nrows {
                for i in 0..self.ncols {
                    let k = (i + j * self.ncols) as usize;
                    if self.oldcell[k] != self.newcell[k] {
                        self.oldcell[k] = self.newcell[k];
                        self.cell_list[self.newcell[k] as usize].push(XPoint { x: i, y: j });
                    }
                }
            }

            self.generation += 1;
            if self.generation > self.mi.cycles {
                self.restart(d);
            }
            self.state = 0;
        } else {
            if !self.cell_list[self.state as usize].is_empty() {
                self.draw_state(d);
            }
            self.state += 1;
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
    "*count: 0",
    "*cycles: 1000",
    "*size: -30",
    "*ncolors: 64",
    "*fpsSolid: true",
    "*ignoreRotation: True",
    "*neighbors: 0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "50000").inverted(),
    Opt::slider("count", "States", 0.0, 20.0, 1.0, 0, "0"),
    Opt::slider("cycles", "Timeout", 0.0, 800_000.0, 1000.0, 0, "1000"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
    Opt::spin("size", "Cell size", -40.0, 40.0, "-30"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "demon",
    label: "Demon",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "David Bagley",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=OhHI-pIHddA"),
        blurb: "A cellular automaton that starts with a random field, and organizes it into stripes and spirals.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
