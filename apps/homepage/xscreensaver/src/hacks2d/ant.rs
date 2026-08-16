//! Port of `hacks/ant.c`.
//!
//! ```text
//! ant --- Chris Langton's generalized turing machine ants (also known
//!         as Greg Turk's turmites) whose tape is the screen
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
//!   Species Grid     Number of Neighbors
//!   ------- ----     ------------------
//!   Ants    Square   4 (or 8)
//!   Bees    Hexagon  6
//!   Bees    Triangle 3 (or 9, 12)
//!
//!   Neighbors 6 and neighbors 3 produce the same Turk ants.
//!
//! Coded from A.K. Dewdney's "Computer Recreations", Scientific
//! American Magazine" Sep 1989 pp 180-183, Mar 1990 p 121
//! Also used Ian Stewart's Mathematical Recreations, Scientific
//! American Jul 1994 pp 104-107
//! ```
//!
//! A Turing machine whose tape is the screen. An ant sits on a cell, reads its
//! colour, and a table says what colour to write there, how far to turn, and
//! which internal state to move to. Then it steps forward and does it again.
//! That is the whole program. Everything on screen is the tape.
//!
//! The machines are of two kinds. A few are written out by hand, including one
//! that builds a ladder and one that spirals. The rest are generated: take a
//! binary number, give the ant one colour per digit, and let each digit say
//! whether that colour means turn left or turn right. Langton's original ant is
//! the two-digit one, and it does the famous thing of making a mess for ten
//! thousand steps and then, without anything changing, walking off in a
//! straight diagonal highway forever.
//!
//! The grid is not always square. Cells can be hexagons, triangles, or squares
//! with four or eight neighbours, and a turn is one step around whatever
//! neighbour count that is, so the same binary number gives quite different
//! pictures on different grids. Six-sided and three-sided cells produce the same
//! ants, which is upstream's own note.
//!
//! Two options change what is drawn rather than what runs. The eyes are two
//! pixels on the leading edge of whichever cell the ant is on, which is the
//! only way to see which way it is about to go. Truchet lines draw an arc
//! across each cell the ant has visited, joining the edge it came in by to the
//! edge it left by, so the ant's whole history reads as a single continuous
//! curve winding through the pattern. Both are drawn with a good deal of
//! fudging, upstream's word, because the arcs have to meet at cell edges that
//! integer hexagon corners do not put where the trigonometry says.
//!
//! Two things here differ from the C. Upstream's driver forces full-random mode
//! on every hack, and in that mode ant flips a coin for eyes, truchet and sharp
//! turns rather than reading its own switches, so those three knobs in the
//! config XML cannot do anything; here the coin is flipped only for a knob the
//! panel has not set. And the nine-sided option the XML offers is compiled out
//! upstream, so asking for it there falls through to a random grid, which is
//! what it does here.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{
    About, Dpy, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XPoint,
};

/// How many colours a machine can have, from the stipple table upstream sizes
/// everything by.
const NUMSTIPPLES: usize = 11;
const STATES: usize = 2;
const MINANTS: i32 = 1;
const MINGRIDSIZE: i32 = 24;
const MINSIZE: i32 = 1;
const MINRANDOMSIZE: i32 = 5;
const ANGLES: i32 = 360;

/// The grids, and how many of them are the common ones.
const PLOTS: [i32; 5] = [3, 4, 6, 8, 12];
const GOODNEIGHBORKINDS: i32 = 3;

/// Relative ant moves: turn and then step, or step and then turn.
const FS: u8 = 0;
const TRS: u8 = 1;
const THRS: u8 = 2;
const TLS: u8 = 5;
const THLS: u8 = 4;
const TBS: u8 = 3;
const SF: u8 = 6;
const STR: u8 = 7;
const STHR: u8 = 8;
const STB: u8 = 9;
const STHL: u8 = 10;
const STL: u8 = 11;

/// `hexagonUnit` from `automata.h`, as steps from the point before.
const HEXAGON_UNIT: [XPoint; 6] = [
    XPoint { x: 0, y: 0 },
    XPoint { x: 1, y: 1 },
    XPoint { x: 0, y: 2 },
    XPoint { x: -1, y: 1 },
    XPoint { x: -1, y: -1 },
    XPoint { x: 0, y: -2 },
];

/// `triangleUnit`: the left-pointing and the right-pointing cell.
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

/// The hand-written machines. The first two numbers are the colour count and
/// the state count; then three numbers per entry: write this colour, turn this
/// way, go to this state.
const TABLES: [&[u8]; 3] = [
    // Ladder builder.
    &[4, 1, 1, STR, 0, 2, STL, 0, 3, TRS, 0, 0, TLS, 0],
    // Spiralling pattern.
    &[2, 2, 1, TLS, 0, 0, FS, 1, 1, TRS, 0, 1, TRS, 0],
    // Square (hexagon) builder.
    &[2, 2, 1, TLS, 0, 0, FS, 1, 0, TRS, 0, 1, TRS, 0],
];

#[derive(Clone, Copy, Default)]
struct StateEntry {
    color: u8,
    direction: i32,
    next: u8,
}

#[derive(Clone, Copy, Default)]
struct AntState {
    col: i32,
    row: i32,
    direction: i32,
    state: u8,
}

/// `fromTableDirection`: a relative move becomes an angle. Anything at or past
/// a full turn means step first and turn afterwards.
fn from_table_direction(dir: u8, neighbors: i32) -> i32 {
    match dir {
        FS => 0,
        TLS => ANGLES / neighbors,
        THLS => 2 * ANGLES / neighbors,
        TBS => (neighbors / 2) * ANGLES / neighbors,
        THRS => ANGLES - 2 * ANGLES / neighbors,
        TRS => ANGLES - ANGLES / neighbors,
        SF => ANGLES,
        STL => ANGLES + ANGLES / neighbors,
        STHL => ANGLES + 2 * ANGLES / neighbors,
        STB => ANGLES + (neighbors / 2) * ANGLES / neighbors,
        STHR => 2 * ANGLES - 2 * ANGLES / neighbors,
        STR => 2 * ANGLES - ANGLES / neighbors,
        _ => -1,
    }
}

struct Ant {
    mi: ModeInfo,
    neighbors: i32,
    generation: i32,
    xs: i32,
    ys: i32,
    xb: i32,
    yb: i32,
    init_dir: i32,
    nrows: i32,
    ncols: i32,
    width: i32,
    height: i32,
    ncolors: usize,
    nstates: usize,
    n: i32,
    truchet: bool,
    eyes: bool,
    sharpturn: bool,
    machine: [StateEntry; NUMSTIPPLES * STATES],
    /// The tape: one colour per cell.
    tape: Vec<u8>,
    /// Which arc, if any, was drawn in each cell.
    truchet_state: Vec<u8>,
    ants: Vec<AntState>,
    /// One palette index per machine colour.
    colors: [u8; NUMSTIPPLES - 1],
    hexagon: [XPoint; 7],
    triangle: [[XPoint; 4]; 2],
}

impl Ant {
    fn new(d: &mut Dpy) -> Self {
        let mi = ModeInfo::new(d, ColorScheme::Random);
        let mut st = Self {
            width: mi.width,
            height: mi.height,
            mi,
            neighbors: 4,
            generation: 0,
            xs: 1,
            ys: 1,
            xb: 0,
            yb: 0,
            init_dir: 0,
            nrows: 2,
            ncols: 2,
            ncolors: 2,
            nstates: 1,
            n: 1,
            truchet: false,
            eyes: false,
            sharpturn: false,
            machine: [StateEntry::default(); NUMSTIPPLES * STATES],
            tape: Vec::new(),
            truchet_state: Vec::new(),
            ants: Vec::new(),
            colors: [0; NUMSTIPPLES - 1],
            hexagon: [XPoint { x: 0, y: 0 }; 7],
            triangle: [[XPoint { x: 0, y: 0 }; 4]; 2],
        };
        st.restart(d);
        st
    }

    /// `init_ant`: pick a grid, a machine and a place to start.
    fn restart(&mut self, d: &mut Dpy) {
        self.generation = 0;
        self.n = self.mi.count;
        if self.n < -MINANTS {
            self.n = nrand(-self.n - MINANTS + 1) + MINANTS;
        } else if self.n < MINANTS {
            self.n = MINANTS;
        }

        self.width = self.mi.width;
        self.height = self.mi.height;

        let wanted = d.res.int("neighbors");
        self.neighbors = match PLOTS.iter().position(|&p| p == wanted) {
            Some(i) => PLOTS[i],
            // Make the grids above six rare.
            None if nrand(10) == 0 => PLOTS[nrand(PLOTS.len() as i32) as usize],
            None => PLOTS[nrand(GOODNEIGHBORKINDS) as usize],
        };

        let size = self.mi.size;
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
            for (i, u) in HEXAGON_UNIT.iter().enumerate() {
                self.hexagon[i].x = (self.xs - 1) * u.x;
                self.hexagon[i].y = ((self.ys - 1) * u.y / 2) * 4 / 3;
            }
            // Avoid an array bounds read of the unit shape.
            self.hexagon[6] = XPoint { x: 0, y: 0 };
        } else if self.neighbors == 4 || self.neighbors == 8 {
            self.ys = self.cell_size(size);
            self.xs = self.ys;
            self.ncols = (self.width / self.xs).max(2);
            self.nrows = (self.height / self.ys).max(2);
            self.xb = (self.width - self.xs * self.ncols) / 2;
            self.yb = (self.height - self.ys * self.nrows) / 2;
        } else {
            self.width = self.width.max(2);
            self.height = self.height.max(2);
            self.ys = self.cell_size(size);
            self.xs = (1.52 * self.ys as f64) as i32;
            self.ncols = ((self.width / self.xs - 1).max(2) / 2) * 2;
            self.nrows = ((self.height / self.ys - 1).max(2) / 2) * 2;
            self.xb = (self.width - self.xs * self.ncols) / 2 + self.xs / 2;
            self.yb = (self.height - self.ys * self.nrows) / 2 + self.ys;
            for (shape, unit) in self.triangle.iter_mut().zip(TRIANGLE_UNIT.iter()) {
                for (p, u) in shape.iter_mut().zip(unit.iter()) {
                    p.x = (self.xs - 2) * u.x;
                    p.y = (self.ys - 2) * u.y;
                }
                shape[3] = XPoint { x: 0, y: 0 };
            }
        }

        self.mi.gc.set_line_width(1);
        self.mi.clear_window(d);

        // Upstream's driver forces full-random mode, in which these three are
        // coin flips and the switches for them do nothing. Defer to the panel
        // when it has actually set one.
        self.truchet = match d.res.is_overridden("truchet") {
            true => d.res.bool("truchet"),
            false => lrand() & 1 != 0,
        };
        self.eyes = match d.res.is_overridden("eyes") {
            true => d.res.bool("eyes"),
            false => lrand() & 1 != 0,
        };
        self.sharpturn = match d.res.is_overridden("sharpturn") {
            true => d.res.bool("sharpturn"),
            false => lrand() & 1 != 0,
        };

        if nrand(NUMSTIPPLES as i32) == 0 {
            self.get_table(nrand(TABLES.len() as i32) as usize);
        } else {
            self.get_turk(nrand(NUMSTIPPLES as i32 - 1) as usize);
        }

        let npixels = self.mi.npixels();
        if npixels > 2 {
            for i in 0..self.ncolors - 1 {
                // The cast to a byte is upstream's, and it happens before the
                // division, so the spread wraps around rather than running up
                // the palette.
                self.colors[i] =
                    ((nrand(npixels) + i as i32 * npixels) as u8) / (self.ncolors as u8 - 1);
            }
        }

        self.ants = vec![AntState::default(); self.n.max(0) as usize];
        self.tape = vec![0; (self.ncols * self.nrows).max(0) as usize];
        self.truchet_state = vec![0; (self.ncols * self.nrows).max(0) as usize];

        let row = self.nrows / 2;
        let mut col = self.ncols / 2;
        if col > 0 && (self.neighbors % 2 != 0 || self.neighbors == 12) && (lrand() & 1 != 0) {
            col -= 1;
        }
        let dir = nrand(self.neighbors) * ANGLES / self.neighbors;
        self.init_dir = dir;
        // Have them all start in the same spot, why not?
        for a in self.ants.iter_mut() {
            *a = AntState {
                col,
                row,
                direction: dir,
                state: 0,
            };
        }
        self.draw_anant(d, dir, col, row);
    }

    /// The shared cell-size rule: a random size up to the requested one, but
    /// never so small that the grid stops being visible.
    fn cell_size(&self, size: i32) -> i32 {
        let fit = MINSIZE.max(self.width.min(self.height) / MINGRIDSIZE);
        if size < -MINSIZE {
            let ys = nrand((-size).min(fit) - MINSIZE + 1) + MINSIZE;
            if ys < MINRANDOMSIZE {
                MINRANDOMSIZE.min(fit)
            } else {
                ys
            }
        } else if size < MINSIZE {
            if size == 0 { fit } else { MINSIZE }
        } else {
            size.min(fit)
        }
    }

    /// `getTable`: one of the hand-written machines.
    fn get_table(&mut self, i: usize) {
        let t = TABLES[i];
        self.ncolors = t[0] as usize;
        self.nstates = t[1] as usize;
        let total = self.ncolors * self.nstates;
        for j in 0..total {
            self.machine[j].color = t[2 + j * 3];
            let mut k = t[3 + j * 3];
            if self.sharpturn && self.neighbors > 4 {
                // Swap each turn for the harder or softer one beside it.
                k = match k {
                    TRS => THRS,
                    THRS => TRS,
                    THLS => TLS,
                    TLS => THLS,
                    STR => STHR,
                    STHR => STR,
                    STHL => STL,
                    STL => STHL,
                    other => other,
                };
            }
            self.machine[j].direction = from_table_direction(k, self.neighbors);
            self.machine[j].next = t[4 + j * 3];
        }
        self.truchet = false;
    }

    /// `getTurk`: a machine straight out of a binary number, one colour per
    /// digit, each digit saying which way that colour turns.
    fn get_turk(&mut self, i: usize) {
        let mut power2 = 1 << (i + 1);
        // Not a number which in binary is all ones.
        let number = nrand(power2 - 1) + power2;

        self.ncolors = i + 2;
        self.nstates = 1;
        let total = self.ncolors * self.nstates;
        for j in 0..total {
            self.machine[j].color = ((j + 1) % total) as u8;
            let (right, left) = if self.sharpturn && self.neighbors > 4 {
                (THRS, THLS)
            } else {
                (TRS, TLS)
            };
            self.machine[j].direction = from_table_direction(
                if power2 & number != 0 { right } else { left },
                self.neighbors,
            );
            self.machine[j].next = 0;
            power2 >>= 1;
        }
        self.truchet = self.truchet
            && self.xs > 2
            && self.ys > 2
            && (self.neighbors == 3 || self.neighbors == 4 || self.neighbors == 6);
    }

    /// `position_of_neighbor`: one step in a direction, wrapping at the edges.
    fn position_of_neighbor(&self, dir: i32, pcol: &mut i32, prow: &mut i32) {
        let (mut col, mut row) = (*pcol, *prow);
        let (ncols, nrows) = (self.ncols, self.nrows);
        let right = |c: i32| if c + 1 == ncols { 0 } else { c + 1 };
        let left = |c: i32| if c == 0 { ncols - 1 } else { c - 1 };
        let up = |r: i32| if r == 0 { nrows - 1 } else { r - 1 };
        let down = |r: i32| if r + 1 == nrows { 0 } else { r + 1 };
        // Two rows at once, which is what a triangle's opposite edge is.
        let up2 = |r: i32| {
            if r == 0 {
                nrows - 2
            } else if r == 1 {
                nrows - 1
            } else {
                r - 2
            }
        };
        let down2 = |r: i32| {
            if r + 1 == nrows {
                1
            } else if r + 2 == nrows {
                0
            } else {
                r + 2
            }
        };

        if self.neighbors == 6 {
            match dir {
                0 => col = right(col),
                60 => {
                    if row & 1 == 0 {
                        col = right(col);
                    }
                    row = up(row);
                }
                120 => {
                    if row & 1 != 0 {
                        col = left(col);
                    }
                    row = up(row);
                }
                180 => col = left(col),
                240 => {
                    if row & 1 != 0 {
                        col = left(col);
                    }
                    row = down(row);
                }
                300 => {
                    if row & 1 == 0 {
                        col = right(col);
                    }
                    row = down(row);
                }
                _ => {}
            }
        } else if self.neighbors == 4 || self.neighbors == 8 {
            match dir {
                0 => col = right(col),
                45 => {
                    col = right(col);
                    row = up(row);
                }
                90 => row = up(row),
                135 => {
                    col = left(col);
                    row = up(row);
                }
                180 => col = left(col),
                225 => {
                    col = left(col);
                    row = down(row);
                }
                270 => row = down(row),
                315 => {
                    col = right(col);
                    row = down(row);
                }
                _ => {}
            }
        } else if (col + row) % 2 != 0 {
            // A right-pointing triangle.
            match dir {
                0 => col = left(col),
                30 | 40 => {
                    col = left(col);
                    row = up(row);
                }
                60 => {
                    col = left(col);
                    row = up2(row);
                }
                80 | 90 => row = up2(row),
                120 => row = up(row),
                150 | 160 => {
                    col = right(col);
                    row = up(row);
                }
                180 => col = right(col),
                200 | 210 => {
                    col = right(col);
                    row = down(row);
                }
                240 => row = down(row),
                270 | 280 => row = down2(row),
                300 => {
                    col = left(col);
                    row = down2(row);
                }
                320 | 330 => {
                    col = left(col);
                    row = down(row);
                }
                _ => {}
            }
        } else {
            // A left-pointing triangle: the same list, mirrored.
            match dir {
                0 => col = right(col),
                30 | 40 => {
                    col = right(col);
                    row = down(row);
                }
                60 => {
                    col = right(col);
                    row = down2(row);
                }
                80 | 90 => row = down2(row),
                120 => row = down(row),
                150 | 160 => {
                    col = left(col);
                    row = down(row);
                }
                180 => col = left(col),
                200 | 210 => {
                    col = left(col);
                    row = up(row);
                }
                240 => row = up(row),
                270 | 280 => row = up2(row),
                300 => {
                    col = right(col);
                    row = up2(row);
                }
                320 | 330 => {
                    col = right(col);
                    row = up(row);
                }
                _ => {}
            }
        }
        *pcol = col;
        *prow = row;
    }

    // ---- drawing ----------------------------------------------------------

    /// Turn a run of steps into absolute points, the way `CoordModePrevious`
    /// does.
    fn absolute(shape: &[XPoint], n: usize, x: i32, y: i32, out: &mut [XPoint]) {
        let (mut cx, mut cy) = (x, y);
        out[0] = XPoint { x: cx, y: cy };
        for i in 1..n {
            cx += shape[i].x;
            cy += shape[i].y;
            out[i] = XPoint { x: cx, y: cy };
        }
    }

    /// `fillcell`: one cell of whichever grid this is.
    fn fillcell(&self, d: &mut Dpy, col: i32, row: i32) {
        if self.neighbors == 6 {
            let ccol = 2 * col + if row & 1 == 0 { 1 } else { 0 };
            let crow = 2 * row;
            let x = self.xb + ccol * self.xs;
            let y = self.yb + crow * self.ys;
            if self.xs == 1 && self.ys == 1 {
                d.win().draw_point(&self.mi.gc, x, y);
            } else {
                let mut pts = [XPoint { x: 0, y: 0 }; 6];
                Self::absolute(&self.hexagon, 6, x, y, &mut pts);
                d.win().fill_polygon(&self.mi.gc, &pts);
            }
        } else if self.neighbors == 4 || self.neighbors == 8 {
            d.win().fill_rectangle(
                &self.mi.gc,
                self.xb + self.xs * col,
                self.yb + self.ys * row,
                self.xs - i32::from(self.xs > 3),
                self.ys - i32::from(self.ys > 3),
            );
        } else {
            let orient = ((col + row) % 2) as usize;
            let x = self.xb + col * self.xs;
            let y = self.yb + row * self.ys;
            if self.xs <= 3 || self.ys <= 3 {
                d.win()
                    .draw_point(&self.mi.gc, if orient != 0 { -1 } else { 1 } + x, y);
            } else {
                let x = if orient != 0 {
                    x + (self.xs / 2 - 1)
                } else {
                    x - (self.xs / 2 - 1)
                };
                let mut pts = [XPoint { x: 0, y: 0 }; 3];
                Self::absolute(&self.triangle[orient], 3, x, y, &mut pts);
                d.win().fill_polygon(&self.mi.gc, &pts);
            }
        }
    }

    /// `drawcell`: a cell in the colour the tape says.
    fn drawcell(&mut self, d: &mut Dpy, col: i32, row: i32, color: u8) {
        let p = if color == 0 {
            self.mi.black
        } else {
            self.mi.pixel(self.colors[color as usize - 1] as usize)
        };
        self.mi.gc.set_foreground(p);
        self.fillcell(d, col, row);
    }

    /// `truchetcell`: the arc across one cell that joins the edge the ant came
    /// in by to the one it left by. The fudge factors are upstream's: the arcs
    /// have to meet on cell edges that integer corners do not put where the
    /// trigonometry says they are.
    fn truchetcell(&self, d: &mut Dpy, col: i32, row: i32, truchetstate: i32) {
        if self.neighbors == 6 {
            let ccol = 2 * col + if row & 1 == 0 { 1 } else { 0 };
            let crow = 2 * row;
            let fudge = 7;
            if self.sharpturn {
                let mut hx = self.xb + ccol * self.xs - (self.xs as f64 / 2.0) as i32 - 1;
                let mut hy = self.yb + crow * self.ys - (self.ys as f64 / 2.0) as i32 - 1;
                for side in 0..6 {
                    if side != 0 {
                        hx += self.hexagon[side].x;
                        hy += self.hexagon[side].y;
                    }
                    if truchetstate == side as i32 % 2 {
                        d.win().draw_arc(
                            &self.mi.gc,
                            hx,
                            hy,
                            self.xs,
                            self.ys,
                            ((570 - (side as i32 * 60) + fudge) % 360) * 64,
                            (120 - 2 * fudge) * 64,
                        );
                    }
                }
            } else {
                // A very crude approximation of the square root of three, so
                // that it will not cause drawing errors.
                let mut hx = self.xb + ccol * self.xs - (self.xs as f64 * 1.6 / 2.0) as i32 - 1;
                let mut hy = self.yb + crow * self.ys - (self.ys as f64 * 1.6 / 2.0) as i32 - 1;
                for side in 0..6 {
                    if side != 0 {
                        hx += self.hexagon[side].x;
                        hy += self.hexagon[side].y;
                    }
                    let mut h2x = hx + self.hexagon[side + 1].x / 2;
                    let mut h2y = hy + self.hexagon[side + 1].y / 2 + 1;
                    // Lots of fudging here.
                    match side {
                        1 => {
                            h2x += (self.xs as f64 * 0.1 + 1.0) as i32;
                            h2y += (self.ys as f64 * 0.1 - f64::from(self.ys > 5)) as i32;
                        }
                        2 => h2x += (self.xs as f64 * 0.1) as i32,
                        4 => {
                            h2x += (self.xs as f64 * 0.1) as i32;
                            h2y += (self.ys as f64 * 0.1) as i32 - 1;
                        }
                        5 => {
                            h2x += (self.xs as f64 * 0.5) as i32;
                            h2y += (-(self.ys as f64) * 0.3 + 1.0) as i32;
                        }
                        _ => {}
                    }
                    if truchetstate == side as i32 % 3 {
                        // A crude approximation of a hundred and twenty
                        // degrees, likewise.
                        d.win().draw_arc(
                            &self.mi.gc,
                            h2x,
                            h2y,
                            (self.xs as f64 * 1.5) as i32,
                            (self.ys as f64 * 1.5) as i32,
                            ((555 - (side as i32 * 60)) % 360) * 64,
                            90 * 64,
                        );
                    }
                }
            }
        } else if self.neighbors == 4 {
            let (x, y) = (self.xb + self.xs * col, self.yb + self.ys * row);
            let (w, h) = (self.xs - 2, self.ys - 2);
            if truchetstate != 0 {
                d.win().draw_arc(
                    &self.mi.gc,
                    x - self.xs / 2 + 1,
                    y + self.ys / 2 - 1,
                    w,
                    h,
                    0,
                    90 * 64,
                );
                d.win().draw_arc(
                    &self.mi.gc,
                    x + self.xs / 2 - 1,
                    y - self.ys / 2 + 1,
                    w,
                    h,
                    -90 * 64,
                    -90 * 64,
                );
            } else {
                d.win().draw_arc(
                    &self.mi.gc,
                    x - self.xs / 2 + 1,
                    y - self.ys / 2 + 1,
                    w,
                    h,
                    0,
                    -90 * 64,
                );
                d.win().draw_arc(
                    &self.mi.gc,
                    x + self.xs / 2 - 1,
                    y + self.ys / 2 - 1,
                    w,
                    h,
                    90 * 64,
                    90 * 64,
                );
            }
        } else if self.neighbors == 3 {
            let orient = ((col + row) % 2) as usize;
            let fudge = 7;
            let fudge2 = 1.18;
            let mut tx = self.xb + col * self.xs;
            let mut ty = self.yb + row * self.ys;
            if orient != 0 {
                tx += self.xs / 2 - 1;
            } else {
                tx -= self.xs / 2 - 1;
            }
            for side in 0..3 {
                if side > 0 {
                    tx += self.triangle[orient][side].x;
                    ty += self.triangle[orient][side].y;
                }
                if truchetstate == side as i32 {
                    let ang = if orient != 0 {
                        (510 - side as i32 * 120) % 360
                    } else {
                        (690 - side as i32 * 120) % 360
                    };
                    d.win().draw_arc(
                        &self.mi.gc,
                        (tx as f64 - self.xs as f64 * fudge2 / 2.0) as i32,
                        (ty as f64 - 3.0 * self.ys as f64 * fudge2 / 4.0) as i32,
                        (self.xs as f64 * fudge2) as i32,
                        (3.0 * self.ys as f64 * fudge2 / 2.0) as i32,
                        (ang + fudge) * 64,
                        (60 - 2 * fudge) * 64,
                    );
                }
            }
        }
    }

    /// `drawtruchet`: the arc, in whichever of black and white shows up
    /// against the cell it crosses.
    fn drawtruchet(&mut self, d: &mut Dpy, col: i32, row: i32, color: u8, truchetstate: i32) {
        let p = if color == 0 {
            self.mi.white
        } else if self.mi.npixels() > 2 || color as usize > self.ncolors / 2 {
            self.mi.black
        } else {
            self.mi.white
        };
        self.mi.gc.set_foreground(p);
        self.truchetcell(d, col, row, truchetstate);
    }

    /// `draw_anant`: the ant itself, which is a white cell with two dots on
    /// the edge it is facing.
    fn draw_anant(&mut self, d: &mut Dpy, direction: i32, col: i32, row: i32) {
        self.mi.gc.set_foreground(self.mi.white);
        self.fillcell(d, col, row);
        if !self.eyes {
            return;
        }
        self.mi.gc.set_foreground(self.mi.black);
        if self.neighbors == 6 {
            if !(self.xs > 3 && self.ys > 3) {
                return;
            }
            let ccol = 2 * col + if row & 1 == 0 { 1 } else { 0 };
            let crow = 2 * row;
            let mut hx = self.xb + ccol * self.xs;
            let mut hy = self.yb + crow * self.ys + self.ys / 2;
            let ang = direction * self.neighbors / ANGLES;
            for side in 0..self.neighbors {
                if side != 0 {
                    hx -= self.hexagon[side as usize].x / 2;
                    hy += self.hexagon[side as usize].y / 2;
                }
                if side == (self.neighbors + ang - 2) % self.neighbors {
                    d.win().draw_point(&self.mi.gc, hx, hy);
                }
                if side == (self.neighbors + ang - 1) % self.neighbors {
                    d.win().draw_point(&self.mi.gc, hx, hy);
                }
            }
        } else if self.neighbors == 4 || self.neighbors == 8 {
            if !(self.xs > 3 && self.ys > 3) {
                return;
            }
            let (xs, ys, xb, yb) = (self.xs, self.ys, self.xb, self.yb);
            let pts: [(i32, i32); 2] = match direction {
                0 => [
                    (xb + xs * (col + 1) - 3, yb + ys * row + ys / 2 - 2),
                    (xb + xs * (col + 1) - 3, yb + ys * row + ys / 2),
                ],
                45 => [
                    (xb + xs * (col + 1) - 4, yb + ys * row + 1),
                    (xb + xs * (col + 1) - 3, yb + ys * row + 2),
                ],
                90 => [
                    (xb + xs * col + xs / 2 - 2, yb + ys * row + 1),
                    (xb + xs * col + xs / 2, yb + ys * row + 1),
                ],
                135 => [
                    (xb + xs * col + 2, yb + ys * row + 1),
                    (xb + xs * col + 1, yb + ys * row + 2),
                ],
                180 => [
                    (xb + xs * col + 1, yb + ys * row + ys / 2 - 2),
                    (xb + xs * col + 1, yb + ys * row + ys / 2),
                ],
                225 => [
                    (xb + xs * col + 2, yb + ys * (row + 1) - 3),
                    (xb + xs * col + 1, yb + ys * (row + 1) - 4),
                ],
                270 => [
                    (xb + xs * col + xs / 2 - 2, yb + ys * (row + 1) - 3),
                    (xb + xs * col + xs / 2, yb + ys * (row + 1) - 3),
                ],
                315 => [
                    (xb + xs * (col + 1) - 4, yb + ys * (row + 1) - 3),
                    (xb + xs * (col + 1) - 3, yb + ys * (row + 1) - 4),
                ],
                _ => return,
            };
            for (x, y) in pts {
                d.win().draw_point(&self.mi.gc, x, y);
            }
        } else {
            let orient = ((col + row) % 2) as usize;
            if !(self.xs > 6 && self.ys > 6) {
                return;
            }
            let mut tx = self.xb + col * self.xs;
            let mut ty = self.yb + row * self.ys;
            if orient != 0 {
                tx += self.xs / 6 - 1;
            } else {
                tx -= self.xs / 6 - 1;
            }
            let ang = direction * self.neighbors / ANGLES;
            // Upstream has no working eye placement for the finer triangle
            // grids and gives up here rather than drawing them wrong.
            if self.neighbors == 12 {
                return;
            }
            for side in 0..3 {
                if side != 0 {
                    tx += self.triangle[orient][side].x / 3;
                    ty += self.triangle[orient][side].y / 3;
                }
                if side as i32 == (ang + 2) % 3 || side as i32 == (ang + 1) % 3 {
                    d.win().draw_point(&self.mi.gc, tx, ty);
                }
            }
        }
    }
}

impl Screenhack for Ant {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for i in 0..self.ants.len() {
            let (col, row) = (self.ants[i].col, self.ants[i].row);
            let tape_pos = (col + row * self.ncols).max(0) as usize;
            if tape_pos >= self.tape.len() {
                break;
            }
            // Read the tape, look up what to do, write the tape.
            let color = self.tape[tape_pos];
            let state_pos = color as usize + self.ants[i].state as usize * self.ncolors;
            let status = self.machine[state_pos.min(self.machine.len() - 1)];
            self.drawcell(d, col, row, status.color);
            self.tape[tape_pos] = status.color;

            // Translate the relative turn into an actual direction.
            let old_dir = self.ants[i].direction;
            let chg_dir = (2 * ANGLES - status.direction) % ANGLES;
            self.ants[i].direction = (chg_dir + old_dir) % ANGLES;
            if self.truchet {
                let new_dir = self.ants[i].direction;
                let mut a = 0;
                if self.neighbors == 6 {
                    if self.sharpturn {
                        let x = ((ANGLES + new_dir - old_dir) % ANGLES) == 240;
                        // There should be some way of getting rid of the
                        // dependency on the starting direction.
                        let b = self.init_dir % 120 == 0;
                        a = i32::from((x && !b) || (b && !x));
                    } else {
                        let x = (old_dir / 60) % 3;
                        let b = (new_dir / 60) % 3;
                        a = (x + b + 1) % 3;
                    }
                    self.drawtruchet(d, col, row, status.color, a);
                } else if self.neighbors == 4 {
                    let x = old_dir / 180;
                    let b = new_dir / 180;
                    a = i32::from((x != 0 && b == 0) || (b != 0 && x == 0));
                    self.drawtruchet(d, col, row, status.color, a);
                } else if self.neighbors == 3 {
                    a = if chg_dir == 240 {
                        (2 + new_dir / 120) % 3
                    } else {
                        (1 + new_dir / 120) % 3
                    };
                    self.drawtruchet(d, col, row, status.color, a);
                }
                self.truchet_state[tape_pos] = a as u8 + 1;
            }
            self.ants[i].state = status.next;

            // A direction of a full turn or more means step first, then turn.
            let step_dir = if status.direction < ANGLES {
                self.ants[i].direction
            } else {
                old_dir
            };
            let (mut c, mut r) = (self.ants[i].col, self.ants[i].row);
            self.position_of_neighbor(step_dir, &mut c, &mut r);
            self.ants[i].col = c;
            self.ants[i].row = r;
            let dir = self.ants[i].direction;
            self.draw_anant(d, dir, c, r);
        }

        self.generation += 1;
        if self.generation > self.mi.cycles {
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

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(Ant::new(d))
}

const DEFAULTS: &[&str] = &[
    "*delay: 20000",
    "*count: -3",
    "*cycles: 40000",
    "*size: -12",
    "*ncolors: 64",
    "*fpsSolid: true",
    "*neighbors: 0",
    "*truchet: False",
    "*eyes: False",
    "*sharpturn: False",
];

const NEIGHBORS: &[SelectItem] = &[
    SelectItem {
        value: "0",
        label: "Random cell shape",
    },
    SelectItem {
        value: "3",
        label: "Three sided cells",
    },
    SelectItem {
        value: "4",
        label: "Four sided cells",
    },
    SelectItem {
        value: "6",
        label: "Six sided cells",
    },
    SelectItem {
        value: "9",
        label: "Nine sided cells",
    },
    SelectItem {
        value: "12",
        label: "Twelve sided cells",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("cycles", "Timeout", 0.0, 800_000.0, 1000.0, 0, "40000"),
    Opt::slider("ncolors", "Number of colors", 3.0, 255.0, 1.0, 0, "64"),
    Opt::spin("count", "Ants count", -20.0, 20.0, "-3"),
    Opt::spin("size", "Ant size", -18.0, 18.0, "-12"),
    Opt::select("neighbors", "Cell shape", NEIGHBORS, "0"),
    Opt::boolean("sharpturn", "Sharp turns", "false"),
    Opt::boolean("truchet", "Truchet lines", "false"),
    Opt::boolean("eyes", "Draw eyes", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "ant",
    label: "Ant",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "David Bagley",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=PaG7RCO4ezs"),
        blurb: "A cellular automaton that is really a two-dimensional Turing machine.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
