//! Port of `hacks/loop.c`.
//!
//! ```text
//! loop --- Chris Langton's self-producing loops
//!
//! Copyright (c) 1996 by David Bagley.
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
//! From Steven Levy's Artificial Life
//! Chris Langton's cellular automata "loops" reproduce in the spirit of life.
//! Beginning from a single organism, the loops from a colony.  As the loops
//! on the outer fringes reproduce, the inner loops -- blocked by their
//! daughters -- can no longer produce offspring.  These dead progenitors
//! provide a base for future generations' expansion, much like the formation
//! of a coral reef.  This self-organizing behavior emerges spontaneously,
//! from the bottom up -- a key characteristic of artificial life.
//!
//! Don't Panic  --  When the artificial life tries to leave its petri
//! dish (ie. the screen) it will (usually) die...
//!
//! 15-Mar-2001: Added some flaws, random blue wall spots, to liven it up.
//! 16-Jun-2000: Fully coded the hexagonal rules.
//! 15-Nov-1995: Coded from Chris Langton's Self-Reproduction in Cellular
//!              Automata Physica 10D 135-144 1984, also used wire.c as a
//!              guide.
//! ```
//!
//! One loop of cells copies itself, and then the copies copy themselves. Each
//! cell holds one of eight states and takes its next state from a table indexed
//! by its own state and its four neighbours, so nothing in the rule knows about
//! loops: the shape is the only pattern the rule happens to reproduce.
//!
//! Inside the loop a signal runs round and round, a train of coloured cells in
//! a sheath. It is both the machine and the tape. Where the sheath opens into
//! an arm, the signal passing the junction extends the arm one cell; a
//! particular pair of signals turns it; four turns close a new loop, the
//! umbilical retracts, and the daughter starts its own signal going round. The
//! parent goes on making more until it is walled in by its own children, and
//! then it dies: colonies grow as a ring of live loops around a core of dead
//! ones, which is what Langton was pointing at.
//!
//! There are two grids. On squares this is Langton's original rule, two hundred
//! and nineteen entries, each stored four times over for the four rotations of
//! the neighbourhood. On hexagons it is upstream's own six-neighbour rule, six
//! rotations of three hundred and six entries, with a loop that has six sides
//! to store its data on instead of four.
//!
//! The colony is seeded with a few flaws, single blue cells that stand in the
//! way, because a run that is nothing but healthy growth is less interesting
//! than one where some of the offspring come out wrong.
//!
//! Two notes. Where upstream keeps the rule table and the choice of grid in
//! file statics, shared by every screen and built once per process, they are
//! per saver here, which comes to the same thing for one saver in one page. And
//! upstream draws with stipple patterns rather than colours when the display
//! has fewer than eight, which a canvas never does; that path draws white here.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::Pixel;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

const COLORS: usize = 8;
const REALCOLORS: i32 = COLORS as i32 - 2;
/// The smallest cell, in pixels.
const MINSIZE: i32 = 5;
const ANGLES: i32 = 360;
const MAXNEIGHBORS: usize = 6;

/// The starting loop is ten by ten on squares and eleven by eleven on
/// hexagons.
const ADAM_LOOPX: usize = 10;
const ADAM_LOOPY: usize = 10;
const HEX_ADAM_LOOPX: usize = 11;
const HEX_ADAM_LOOPY: usize = 11;
const MINGRIDSIZE: i32 = 3 * ADAM_LOOPX as i32;
const HEX_MINGRIDSIZE: i32 = 6 * HEX_ADAM_LOOPX as i32;

/// `hexagonUnit` from `automata.h`, as steps from the point before.
const HEXAGON_UNIT: [XPoint; 6] = [
    XPoint { x: 0, y: 0 },
    XPoint { x: 1, y: 1 },
    XPoint { x: 0, y: 2 },
    XPoint { x: -1, y: 1 },
    XPoint { x: -1, y: -1 },
    XPoint { x: 0, y: -2 },
];

/// Langton's rule, as octal `CBLTR->I`: the centre and its four neighbours in
/// the high digits, the cell's next state in the lowest.
const TRANSITION_TABLE: [u32; 219] = [
    0o000000, 0o025271, 0o113221, 0o202422, 0o301021, 0o000012, 0o100011, 0o122244, 0o202452,
    0o301220, 0o000020, 0o100061, 0o122277, 0o202520, 0o302511, 0o000030, 0o100077, 0o122434,
    0o202552, 0o401120, 0o000050, 0o100111, 0o122547, 0o202622, 0o401220, 0o000063, 0o100121,
    0o123244, 0o202722, 0o401250, 0o000071, 0o100211, 0o123277, 0o203122, 0o402120, 0o000112,
    0o100244, 0o124255, 0o203216, 0o402221, 0o000122, 0o100277, 0o124267, 0o203226, 0o402326,
    0o000132, 0o100511, 0o125275, 0o203422, 0o402520, 0o000212, 0o101011, 0o200012, 0o204222,
    0o403221, 0o000220, 0o101111, 0o200022, 0o205122, 0o500022, 0o000230, 0o101244, 0o200042,
    0o205212, 0o500215, 0o000262, 0o101277, 0o200071, 0o205222, 0o500225, 0o000272, 0o102026,
    0o200122, 0o205521, 0o500232, 0o000320, 0o102121, 0o200152, 0o205725, 0o500272, 0o000525,
    0o102211, 0o200212, 0o206222, 0o500520, 0o000622, 0o102244, 0o200222, 0o206722, 0o502022,
    0o000722, 0o102263, 0o200232, 0o207122, 0o502122, 0o001022, 0o102277, 0o200242, 0o207222,
    0o502152, 0o001120, 0o102327, 0o200250, 0o207422, 0o502220, 0o002020, 0o102424, 0o200262,
    0o207722, 0o502244, 0o002030, 0o102626, 0o200272, 0o211222, 0o502722, 0o002050, 0o102644,
    0o200326, 0o211261, 0o512122, 0o002125, 0o102677, 0o200423, 0o212222, 0o512220, 0o002220,
    0o102710, 0o200517, 0o212242, 0o512422, 0o002322, 0o102727, 0o200522, 0o212262, 0o512722,
    0o005222, 0o105427, 0o200575, 0o212272, 0o600011, 0o012321, 0o111121, 0o200722, 0o214222,
    0o600021, 0o012421, 0o111221, 0o201022, 0o215222, 0o602120, 0o012525, 0o111244, 0o201122,
    0o216222, 0o612125, 0o012621, 0o111251, 0o201222, 0o217222, 0o612131, 0o012721, 0o111261,
    0o201422, 0o222272, 0o612225, 0o012751, 0o111277, 0o201722, 0o222442, 0o700077, 0o014221,
    0o111522, 0o202022, 0o222462, 0o701120, 0o014321, 0o112121, 0o202032, 0o222762, 0o701220,
    0o014421, 0o112221, 0o202052, 0o222772, 0o701250, 0o014721, 0o112244, 0o202073, 0o300013,
    0o702120, 0o016251, 0o112251, 0o202122, 0o300022, 0o702221, 0o017221, 0o112277, 0o202152,
    0o300041, 0o702251, 0o017255, 0o112321, 0o202212, 0o300076, 0o702321, 0o017521, 0o112424,
    0o202222, 0o300123, 0o702525, 0o017621, 0o112621, 0o202272, 0o300421, 0o702720, 0o017721,
    0o112727, 0o202321, 0o300622,
];

/// Upstream's six-neighbour rule, as octal `CBbltTR->I`.
const HEX_TRANSITION_TABLE: [u32; 306] = [
    0o00000000, 0o00000020, 0o00000220, 0o00002220, 0o11212121, 0o11212221, 0o11221221, 0o11222221,
    0o20002122, 0o20021122, 0o20211122, 0o10221221, 0o10222121, 0o20002022, 0o20021022, 0o20020122,
    0o20112022, 0o10202121, 0o20102022, 0o20202112, 0o00000012, 0o00000122, 0o00000212, 0o10002121,
    0o20001122, 0o20002112, 0o20011122, 0o01227221, 0o01272221, 0o01272721, 0o12212277, 0o11222727,
    0o11212727, 0o20021722, 0o20027122, 0o20020722, 0o20027022, 0o20211722, 0o20202172, 0o20120272,
    0o20271122, 0o20202172, 0o20207122, 0o20217122, 0o20120272, 0o20210722, 0o20270722, 0o70212220,
    0o70221220, 0o70212120, 0o12222277, 0o20002727, 0o70222220, 0o01277721, 0o00000070, 0o00000270,
    0o00000720, 0o00000770, 0o20070122, 0o20021072, 0o70002072, 0o70007022, 0o70007071, 0o20070722,
    0o70002022, 0o10227227, 0o10222727, 0o10202727, 0o20172022, 0o20202712, 0o01224221, 0o01242221,
    0o01242421, 0o12212244, 0o11222424, 0o11212424, 0o20021422, 0o20024122, 0o20020422, 0o20024022,
    0o20211422, 0o20202142, 0o20120242, 0o20241122, 0o20202142, 0o20204122, 0o20214122, 0o20120242,
    0o20210422, 0o20240422, 0o40212220, 0o40221220, 0o40212120, 0o12222244, 0o20002424, 0o40222220,
    0o01244421, 0o00000040, 0o00000240, 0o00000420, 0o00000440, 0o20040122, 0o20021042, 0o40002042,
    0o40004021, 0o40004042, 0o20040422, 0o40002022, 0o10224224, 0o10222424, 0o10202424, 0o20142022,
    0o20202412, 0o20011722, 0o20112072, 0o20172072, 0o20142072, 0o00210225, 0o00022015, 0o00022522,
    0o11225521, 0o20120525, 0o20020152, 0o20005122, 0o20214255, 0o20021152, 0o20255242, 0o50215222,
    0o50225121, 0o00225220, 0o01254222, 0o10221250, 0o11221251, 0o11225221, 0o20025122, 0o20152152,
    0o20211252, 0o20214522, 0o20511125, 0o50212241, 0o5221120, 0o40521225, 0o00000250, 0o00000520,
    0o00150220, 0o00220520, 0o00222210, 0o01224251, 0o10022152, 0o10251221, 0o10522121, 0o11212151,
    0o11221251, 0o11215221, 0o20000220, 0o20002152, 0o20020220, 0o20022152, 0o20021422, 0o20022152,
    0o20022522, 0o20025425, 0o20050422, 0o20051022, 0o20051122, 0o20211122, 0o20211222, 0o20215222,
    0o20245122, 0o50021125, 0o50021025, 0o50011125, 0o51242221, 0o41225220, 0o00220250, 0o00220520,
    0o01227521, 0o01275221, 0o11257227, 0o11522727, 0o20002052, 0o20002752, 0o20021052, 0o20057125,
    0o50020722, 0o50027125, 0o70215220, 0o70212255, 0o71225220, 0o20275122, 0o51272521, 0o20055725,
    0o20021552, 0o12252277, 0o50002521, 0o20005725, 0o50011022, 0o00000155, 0o20050722, 0o01227250,
    0o10512727, 0o10002151, 0o20027112, 0o01227251, 0o12227257, 0o50002125, 0o20517122, 0o50002025,
    0o20050102, 0o50002725, 0o20570722, 0o01252721, 0o20007051, 0o20102052, 0o20271072, 0o50001122,
    0o10002151, 0o11227257, 0o20051722, 0o20057022, 0o20050122, 0o20051422, 0o11224254, 0o12224254,
    0o20054022, 0o50002425, 0o40252220, 0o20002454, 0o00000540, 0o01254425, 0o50004024, 0o40004051,
    0o00000142, 0o40001522, 0o10002547, 0o20045122, 0o51221240, 0o20002512, 0o20021522, 0o20020022,
    0o21125522, 0o20521122, 0o20025022, 0o20025522, 0o20020522, 0o20202222, 0o20212222, 0o21212222,
    0o21222722, 0o21222422, 0o20002222, 0o20021222, 0o20022122, 0o20212122, 0o20027222, 0o20024222,
    0o20212722, 0o20212422, 0o20202122, 0o01222221, 0o20002522, 0o20017125, 0o10022722, 0o20212052,
    0o20205052, 0o70221250, 0o00000050, 0o00005220, 0o00002270, 0o70252220, 0o00000450, 0o00007220,
    0o00220220, 0o00202220, 0o00022020, 0o00020220, 0o00222040, 0o00220440, 0o00022040, 0o00040220,
    0o00252220, 0o50221120, 0o10221520, 0o02222220, 0o00070220, 0o00220720, 0o00020520, 0o00070250,
    0o00222070, 0o00027020, 0o00022070, 0o00202270, 0o00024020, 0o00220420, 0o00220270, 0o00220240,
    0o00072020, 0o00042020, 0o00002020, 0o00002070, 0o00020270, 0o00020250, 0o00270270, 0o00007020,
    0o00040270, 0o00050220,
];

/// The starting organism: a loop with its data already in it.
const SELF_REPRODUCING_LOOP: [[u8; ADAM_LOOPX]; ADAM_LOOPY] = [
    [0, 2, 2, 2, 2, 2, 2, 2, 2, 0],
    [2, 4, 0, 1, 4, 0, 1, 1, 1, 2],
    [2, 1, 2, 2, 2, 2, 2, 2, 1, 2],
    [2, 0, 2, 0, 0, 0, 0, 2, 1, 2],
    [2, 7, 2, 0, 0, 0, 0, 2, 7, 2],
    [2, 1, 2, 0, 0, 0, 0, 2, 0, 2],
    [2, 0, 2, 0, 0, 0, 0, 2, 1, 2],
    [2, 7, 2, 2, 2, 2, 2, 2, 7, 2],
    [2, 1, 0, 6, 1, 0, 7, 1, 0, 2],
    [0, 2, 2, 2, 2, 2, 2, 2, 2, 0],
];

/// The same for hexagons, which has six sides to store its data on.
const HEX_SELF_REPRODUCING_LOOP: [[u8; HEX_ADAM_LOOPX]; HEX_ADAM_LOOPY] = [
    [2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0],
    [2, 1, 1, 7, 0, 1, 2, 0, 0, 0, 0],
    [2, 1, 2, 2, 2, 2, 7, 2, 0, 0, 0],
    [2, 1, 2, 0, 0, 0, 2, 0, 2, 0, 0],
    [2, 1, 2, 0, 0, 0, 0, 2, 1, 2, 0],
    [2, 1, 2, 0, 0, 0, 0, 0, 2, 7, 2],
    [0, 2, 1, 2, 0, 0, 0, 0, 2, 0, 2],
    [0, 0, 2, 1, 2, 0, 0, 0, 2, 1, 2],
    [0, 0, 0, 2, 1, 2, 2, 2, 2, 4, 2],
    [0, 0, 0, 0, 2, 1, 1, 1, 1, 5, 2],
    [0, 0, 0, 0, 0, 2, 2, 2, 2, 2, 2],
];

struct Loop {
    mi: ModeInfo,
    /// Four or six; chosen once and kept for the life of the saver, as
    /// upstream keeps it for the life of the process.
    neighbors: i32,
    /// The rule, indexed by the neighbourhood, holding one next state per
    /// centre state in three bits each.
    table: Vec<u32>,

    generation: i32,
    xs: i32,
    ys: i32,
    xb: i32,
    yb: i32,
    nrows: i32,
    ncols: i32,
    bx: i32,
    by: i32,
    bnrows: i32,
    bncols: i32,
    /// The bounding box of everything that has changed, which is all that has
    /// to be looked at next generation.
    mincol: i32,
    minrow: i32,
    maxcol: i32,
    maxrow: i32,
    width: i32,
    height: i32,
    dead: bool,
    /// Which way round the loop's signal runs. Only one way works for a given
    /// rule, so the organism is mirrored to match.
    clockwise: bool,
    newcells: Vec<u8>,
    oldcells: Vec<u8>,
    colors: [Pixel; COLORS],
    hexagon: [XPoint; 6],
}

impl Loop {
    fn new(d: &mut Dpy) -> Self {
        let mi = ModeInfo::new(d, ColorScheme::Uniform);
        let wanted = d.res.int("neighbors");
        let neighbors = if wanted == 4 || wanted == 6 {
            wanted
        } else if nrand(2) != 0 {
            6
        } else {
            4
        };
        let mut st = Self {
            width: mi.width,
            height: mi.height,
            mi,
            neighbors,
            table: Vec::new(),
            generation: 0,
            xs: 1,
            ys: 1,
            xb: 0,
            yb: 0,
            nrows: 1,
            ncols: 1,
            bx: 1,
            by: 1,
            bnrows: 3,
            bncols: 3,
            mincol: 0,
            minrow: 0,
            maxcol: 0,
            maxrow: 0,
            dead: false,
            clockwise: false,
            newcells: Vec::new(),
            oldcells: Vec::new(),
            colors: [0; COLORS],
            hexagon: [XPoint { x: 0, y: 0 }; 6],
        };
        st.init_table();
        st.restart(d);
        st
    }

    /// Expand each rule entry into every rotation of its neighbourhood.
    fn init_table(&mut self) {
        let mult = 8usize.pow(self.neighbors as u32);
        self.table = vec![0; mult];

        if self.neighbors == 6 {
            for &entry in HEX_TRANSITION_TABLE.iter() {
                let mut tt = entry;
                let i = tt & 7;
                tt >>= 3;
                let mut n = [0u32; MAXNEIGHBORS];
                for slot in n.iter_mut() {
                    *slot = tt & 7;
                    tt >>= 3;
                }
                let c = tt & 7;
                for rot in 0..6 {
                    let r = |k: usize| n[(k + rot) % 6];
                    let at = ((r(5) << 15)
                        | (r(4) << 12)
                        | (r(3) << 9)
                        | (r(2) << 6)
                        | (r(1) << 3)
                        | r(0)) as usize;
                    self.table[at] |= i << (c * 3);
                }
            }
        } else {
            for &entry in TRANSITION_TABLE.iter() {
                let mut tt = entry;
                let i = tt & 7;
                tt >>= 3;
                let mut n = [0u32; 4];
                for slot in n.iter_mut() {
                    *slot = tt & 7;
                    tt >>= 3;
                }
                let c = tt & 7;
                for rot in 0..4 {
                    let r = |k: usize| n[(k + rot) % 4];
                    let at = ((r(3) << 9) | (r(2) << 6) | (r(1) << 3) | r(0)) as usize;
                    self.table[at] |= i << (c * 3);
                }
            }
        }
    }

    fn table_out(&self, c: u32, n: &[u32]) -> u8 {
        let at = if self.neighbors == 6 {
            ((n[5] << 15) | (n[4] << 12) | (n[3] << 9) | (n[2] << 6) | (n[1] << 3) | n[0]) as usize
        } else {
            ((n[3] << 9) | (n[2] << 6) | (n[1] << 3) | n[0]) as usize
        };
        ((self.table[at] >> (c * 3)) & 7) as u8
    }

    /// One step in a direction. Nothing wraps: a loop that walks off the edge
    /// dies there.
    fn position_of_neighbor(&self, dir: i32, pcol: &mut i32, prow: &mut i32) {
        let (mut col, mut row) = (*pcol, *prow);
        if self.neighbors == 6 {
            match dir {
                0 => col += 1,
                60 => {
                    col += row & 1;
                    row -= 1;
                }
                120 => {
                    col -= i32::from(row & 1 == 0);
                    row -= 1;
                }
                180 => col -= 1,
                240 => {
                    col -= i32::from(row & 1 == 0);
                    row += 1;
                }
                300 => {
                    col += row & 1;
                    row += 1;
                }
                _ => {}
            }
        } else {
            match dir {
                0 => col += 1,
                90 => row -= 1,
                180 => col -= 1,
                270 => row += 1,
                _ => {}
            }
        }
        *pcol = col;
        *prow = row;
    }

    fn within_bounds(&self, col: i32, row: i32) -> bool {
        row >= 1
            && row < self.bnrows - 1
            && col >= 1
            && col < self.bncols - 1 - i32::from(self.neighbors == 6 && (row % 2) != 0)
    }

    fn cell(&self, col: i32, row: i32) -> u8 {
        self.oldcells[(col + row * self.bncols) as usize]
    }

    fn fillcell(&mut self, d: &mut Dpy, col: i32, row: i32, state: usize) {
        let p = if self.mi.npixels() >= COLORS as i32 {
            self.colors[state]
        } else {
            // With too few colours upstream draws stipple patterns; a canvas
            // always has enough, so this is only reachable by asking for it.
            self.mi.white
        };
        self.mi.gc.set_foreground(p);
        if self.neighbors == 6 {
            let ccol = 2 * col + i32::from(row & 1 == 0);
            let crow = 2 * row;
            let x = self.xb + ccol * self.xs;
            let y = self.yb + crow * self.ys;
            if self.xs == 1 && self.ys == 1 {
                d.win().draw_point(&self.mi.gc, x, y);
            } else {
                let mut pts = [XPoint { x: 0, y: 0 }; 6];
                let (mut cx, mut cy) = (x, y);
                pts[0] = XPoint { x: cx, y: cy };
                for (p, step) in pts.iter_mut().zip(self.hexagon.iter()).skip(1) {
                    cx += step.x;
                    cy += step.y;
                    *p = XPoint { x: cx, y: cy };
                }
                d.win().fill_polygon(&self.mi.gc, &pts);
            }
        } else {
            d.win().fill_rectangle(
                &self.mi.gc,
                self.xb + self.xs * col,
                self.yb + self.ys * row,
                self.xs - i32::from(self.xs > 3),
                self.ys - i32::from(self.ys > 3),
            );
        }
    }

    /// A flaw: a few blue cells dropped in the way of the growing colony.
    fn init_flaw(&mut self) {
        const BLUE: u8 = 2;
        if self.bncols <= 3 || self.bnrows <= 3 {
            return;
        }
        let grid = if self.neighbors == 6 {
            HEX_MINGRIDSIZE
        } else {
            MINGRIDSIZE
        };
        let mut a = (self.bncols - 3).min(2 * grid);
        a = nrand(a) + (self.bncols - a) / 2;
        let mut b = (self.bnrows - 3).min(2 * grid);
        b = nrand(b) + (self.bnrows - b) / 2;
        self.mincol = self.mincol.min(a);
        self.minrow = self.minrow.min(b);
        self.maxcol = self.maxcol.max(a + 2);
        self.maxrow = self.maxrow.max(b + 2);

        let put = |cells: &mut Vec<u8>, col: i32, row: i32, w: i32| {
            let at = (row * w + col) as usize;
            if at < cells.len() {
                cells[at] = BLUE;
            }
        };
        let w = self.bncols;
        if self.neighbors == 6 {
            let odd = i32::from(b % 2 == 0);
            put(&mut self.newcells, a + odd, b, w);
            put(&mut self.newcells, a + 1 + odd, b, w);
            put(&mut self.newcells, a, b + 1, w);
            put(&mut self.newcells, a + 2, b + 1, w);
            put(&mut self.newcells, a + odd, b + 2, w);
            put(&mut self.newcells, a + 1 + odd, b + 2, w);
        } else {
            let orient = nrand(4);
            put(&mut self.newcells, a + 1, b + 1, w);
            if orient == 0 || orient == 1 {
                put(&mut self.newcells, a + 1, b, w);
            }
            if orient == 1 || orient == 2 {
                put(&mut self.newcells, a + 2, b + 1, w);
            }
            if orient == 2 || orient == 3 {
                put(&mut self.newcells, a + 1, b + 2, w);
            }
            if orient == 3 || orient == 0 {
                put(&mut self.newcells, a, b + 1, w);
            }
        }
    }

    /// Place the starting organism, in one of four or six orientations.
    fn init_adam(&mut self) {
        self.clockwise = lrand() & 1 != 0;
        let dir = nrand(self.neighbors);
        let bncols = self.bncols;
        let bnrows = self.bnrows;
        let cw = self.clockwise;

        if self.neighbors == 6 {
            let lx = HEX_ADAM_LOOPX as i32;
            let ly = HEX_ADAM_LOOPY as i32;
            let put = |cells: &mut Vec<u8>, col: i32, row: i32, v: u8| {
                if col >= 0 && col < bncols && row >= 0 && row < bnrows {
                    cells[(row * bncols + col) as usize] = v;
                }
            };
            match dir {
                0 | 3 => {
                    let sx = (bncols - lx / 2) / 2;
                    let sy = (bnrows - ly) / 2;
                    self.mincol = self.mincol.min(sx - if dir == 0 { 2 } else { 1 });
                    self.minrow = self.minrow.min(sy - 1);
                    self.maxcol = self.maxcol.max(sx + lx + 1);
                    self.maxrow = self.maxrow.max(sy + ly + 1);
                    for j in 0..ly {
                        for i in 0..lx {
                            let k = if (bnrows / 2 + ly / 2) % 2 != 0 {
                                -j / 2
                            } else {
                                -(j + 1) / 2
                            };
                            let v = if dir == 0 {
                                if cw {
                                    HEX_SELF_REPRODUCING_LOOP[i as usize][j as usize]
                                } else {
                                    HEX_SELF_REPRODUCING_LOOP[j as usize][i as usize]
                                }
                            } else if cw {
                                HEX_SELF_REPRODUCING_LOOP[(lx - i - 1) as usize]
                                    [(ly - j - 1) as usize]
                            } else {
                                HEX_SELF_REPRODUCING_LOOP[(ly - j - 1) as usize]
                                    [(lx - i - 1) as usize]
                            };
                            put(&mut self.newcells, sx + i + k, sy + j, v);
                        }
                    }
                }
                1 | 4 => {
                    let sx = (bncols - (lx + ly) / 2) / 2;
                    let sy = (bnrows - lx + ly) / 2;
                    self.mincol = self.mincol.min(sx - 1);
                    self.minrow = self.minrow.min(sy - lx);
                    self.maxcol = self.maxcol.max(sx + (lx + ly) / 2 + 1);
                    self.maxrow = self.maxrow.max(sy + ly + 1);
                    for j in 0..ly {
                        for i in 0..lx {
                            let k = if (bnrows / 2 + (lx + ly) / 2) % 2 != 0 {
                                -(i + j + 1) / 2
                            } else {
                                -(i + j) / 2
                            };
                            let v = if dir == 1 {
                                if cw {
                                    HEX_SELF_REPRODUCING_LOOP[i as usize][j as usize]
                                } else {
                                    HEX_SELF_REPRODUCING_LOOP[j as usize][i as usize]
                                }
                            } else if cw {
                                HEX_SELF_REPRODUCING_LOOP[(lx - i - 1) as usize]
                                    [(ly - j - 1) as usize]
                            } else {
                                HEX_SELF_REPRODUCING_LOOP[(ly - j - 1) as usize]
                                    [(lx - i - 1) as usize]
                            };
                            put(&mut self.newcells, sx + i + j + k, sy + j - i, v);
                        }
                    }
                }
                _ => {
                    let sx = (bncols - ly / 2) / 2;
                    let sy = (bnrows - lx) / 2;
                    self.mincol = self.mincol.min(sx - 2);
                    self.minrow = self.minrow.min(sy - 1);
                    self.maxcol = self.maxcol.max(sx + if dir == 2 { lx } else { ly } + 1);
                    self.maxrow = self.maxrow.max(sy + if dir == 2 { ly } else { lx } + 1);
                    for j in 0..lx {
                        for i in 0..ly {
                            let k = if (bnrows / 2 + lx / 2) % 2 != 0 {
                                -(lx - j - 1) / 2
                            } else {
                                -(lx - j) / 2
                            };
                            let v = if dir == 2 {
                                if cw {
                                    HEX_SELF_REPRODUCING_LOOP[j as usize][(lx - i - 1) as usize]
                                } else {
                                    HEX_SELF_REPRODUCING_LOOP[i as usize][(ly - j - 1) as usize]
                                }
                            } else if cw {
                                HEX_SELF_REPRODUCING_LOOP[(ly - j - 1) as usize][i as usize]
                            } else {
                                HEX_SELF_REPRODUCING_LOOP[(lx - i - 1) as usize][j as usize]
                            };
                            put(&mut self.newcells, sx + i + k, sy + j, v);
                        }
                    }
                }
            }
        } else {
            let lx = ADAM_LOOPX as i32;
            let ly = ADAM_LOOPY as i32;
            let (sx, sy, dirx, diry) = match dir {
                0 => ((bncols - lx) / 2, (bnrows - ly) / 2, (1, 0), (0, 1)),
                1 => ((bncols + ly) / 2, (bnrows - lx) / 2, (0, 1), (-1, 0)),
                2 => ((bncols + lx) / 2, (bnrows + ly) / 2, (-1, 0), (0, -1)),
                _ => ((bncols - ly) / 2, (bnrows + lx) / 2, (0, -1), (1, 0)),
            };
            match dir {
                0 => {
                    self.mincol = self.mincol.min(sx);
                    self.minrow = self.minrow.min(sy);
                    self.maxcol = self.maxcol.max(sx + lx);
                    self.maxrow = self.maxrow.max(sy + ly);
                }
                1 => {
                    self.mincol = self.mincol.min(sx - ly);
                    self.minrow = self.minrow.min(sy);
                    self.maxcol = self.maxcol.max(sx);
                    self.maxrow = self.maxrow.max(sy + lx);
                }
                2 => {
                    self.mincol = self.mincol.min(sx - lx);
                    self.minrow = self.minrow.min(sy - ly);
                    self.maxcol = self.maxcol.max(sx);
                    self.maxrow = self.maxrow.max(sy);
                }
                _ => {
                    self.mincol = self.mincol.min(sx);
                    self.minrow = self.minrow.min(sy - lx);
                    self.maxcol = self.maxcol.max(sx + lx);
                    self.maxrow = self.maxrow.max(sy);
                }
            }
            for j in 0..ly {
                for i in 0..lx {
                    let col = sx + dirx.0 * i + diry.0 * j;
                    let row = sy + dirx.1 * i + diry.1 * j;
                    let v = if cw {
                        SELF_REPRODUCING_LOOP[j as usize][(lx - i - 1) as usize]
                    } else {
                        SELF_REPRODUCING_LOOP[j as usize][i as usize]
                    };
                    if col >= 0 && col < bncols && row >= 0 && row < bnrows {
                        self.newcells[(row * bncols + col) as usize] = v;
                    }
                }
            }
        }
    }

    /// `init_loop`: a new colony on a fresh grid.
    fn restart(&mut self, d: &mut Dpy) {
        let mut size = self.mi.size;
        self.generation = 0;
        self.width = self.mi.width;
        self.height = self.mi.height;
        if self.width < 100 || self.height < 100 {
            // A tiny window.
            size = self.width.min(self.height);
        }

        if self.mi.npixels() >= COLORS as i32 {
            let n = self.mi.npixels();
            self.colors[0] = self.mi.black;
            self.colors[1] = self.mi.pixel(0);
            self.colors[5] = self.mi.pixel((n / REALCOLORS) as usize);
            self.colors[4] = self.mi.pixel((2 * n / REALCOLORS) as usize);
            self.colors[6] = self.mi.pixel((3 * n / REALCOLORS) as usize);
            self.colors[2] = self.mi.pixel((4 * n / REALCOLORS) as usize);
            self.colors[3] = self.mi.pixel((5 * n / REALCOLORS) as usize);
            self.colors[7] = self.mi.white;
        }

        let grid = if self.neighbors == 6 {
            HEX_MINGRIDSIZE
        } else {
            MINGRIDSIZE
        };
        if self.neighbors == 6 {
            self.width = self.width.max(8);
            self.height = self.height.max(8);
        }
        let fit = MINSIZE.max(self.width.min(self.height) / grid);
        self.ys = if size < -MINSIZE {
            nrand((-size).min(fit) - MINSIZE + 1) + MINSIZE
        } else if size < MINSIZE {
            if size == 0 { fit } else { MINSIZE }
        } else {
            size.min(fit)
        };
        if self.width > 2560 || self.height > 2560 {
            // Retina displays.
            self.ys *= 3;
        }
        self.xs = self.ys;

        if self.neighbors == 6 {
            let nccols = (self.width / self.xs - 2).max(HEX_MINGRIDSIZE);
            let ncrows = (self.height / self.ys - 1).max(HEX_MINGRIDSIZE);
            self.ncols = nccols / 2;
            self.nrows = ncrows / 2;
            // Must be odd.
            self.nrows -= i32::from(self.nrows & 1 == 0);
            self.xb = (self.width - self.xs * nccols) / 2 + self.xs;
            self.yb = (self.height - self.ys * ncrows) / 2 + self.ys;
            for (i, u) in HEXAGON_UNIT.iter().enumerate() {
                self.hexagon[i].x = (self.xs - 1) * u.x;
                self.hexagon[i].y = ((self.ys - 1) * u.y / 2) * 4 / 3;
            }
        } else {
            self.ncols = (self.width / self.xs).max(ADAM_LOOPX as i32 + 1);
            self.nrows = (self.height / self.ys).max(ADAM_LOOPX as i32 + 1);
            self.xb = (self.width - self.xs * self.ncols) / 2;
            self.yb = (self.height - self.ys * self.nrows) / 2;
        }
        self.bx = 1;
        self.by = 1;
        self.bncols = self.ncols + 2 * self.bx;
        self.bnrows = self.nrows + 2 * self.by;

        self.mi.clear_window(d);

        let cells = (self.bncols * self.bnrows).max(1) as usize;
        self.oldcells = vec![0; cells];
        self.newcells = vec![0; cells];

        self.mincol = self.bncols - 1;
        self.minrow = self.bnrows - 1;
        self.maxcol = 0;
        self.maxrow = 0;

        let count = self.mi.count;
        let flaws = if count < 0 { nrand(-count + 1) } else { count };
        for _ in 0..flaws {
            self.init_flaw();
        }
        self.init_adam();
    }

    /// One generation of the rule, over the changed region only.
    fn do_gen(&mut self) {
        let mut n = [0u32; MAXNEIGHBORS];
        for j in self.minrow..=self.maxrow {
            for i in self.mincol..=self.maxcol {
                if i < 0 || j < 0 || i >= self.bncols || j >= self.bnrows {
                    continue;
                }
                let c = self.cell(i, j) as u32;
                for (k, slot) in n.iter_mut().enumerate().take(self.neighbors as usize) {
                    let (mut newi, mut newj) = (i, j);
                    self.position_of_neighbor(
                        k as i32 * ANGLES / self.neighbors,
                        &mut newi,
                        &mut newj,
                    );
                    *slot = if self.within_bounds(newi, newj) {
                        self.cell(newi, newj) as u32
                    } else {
                        0
                    };
                }
                let v = if self.neighbors == 6 {
                    if self.clockwise {
                        self.table_out(c, &[n[5], n[4], n[3], n[2], n[1], n[0]])
                    } else {
                        self.table_out(c, &n)
                    }
                } else if self.clockwise {
                    self.table_out(c, &[n[3], n[2], n[1], n[0]])
                } else {
                    self.table_out(c, &n[..4])
                };
                self.newcells[(i + j * self.bncols) as usize] = v;
            }
        }
    }
}

impl Screenhack for Loop {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.dead = true;
        // Draw every cell that changed, and grow the region of interest
        // wherever a change reached its edge.
        for j in self.minrow..=self.maxrow {
            for i in self.mincol..=self.maxcol {
                if i < 0 || j < 0 || i >= self.bncols || j >= self.bnrows {
                    continue;
                }
                let offset = (j * self.bncols + i) as usize;
                if self.oldcells[offset] == self.newcells[offset] {
                    continue;
                }
                self.dead = false;
                let state = self.newcells[offset];
                self.oldcells[offset] = state;
                self.fillcell(d, i - self.bx, j - self.by, state as usize);
                if i == self.mincol && i > self.bx {
                    self.mincol -= 1;
                }
                if j == self.minrow && j > self.by {
                    self.minrow -= 1;
                }
                if i == self.maxcol && i < self.bncols - 2 * self.bx {
                    self.maxcol += 1;
                }
                if j == self.maxrow && j < self.bnrows - 2 * self.by {
                    self.maxrow += 1;
                }
            }
        }

        self.generation += 1;
        if self.generation > self.mi.cycles || self.dead {
            self.restart(d);
        } else {
            self.do_gen();
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
    Box::new(Loop::new(d))
}

const DEFAULTS: &[&str] = &[
    "*delay: 100000",
    "*count: -5",
    "*cycles: 1600",
    "*size: -12",
    "*ncolors: 15",
    "*fpsSolid: true",
    "*ignoreRotation: True",
    "*neighbors: 0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 200_000.0, 1000.0, 0, "100000").inverted(),
    Opt::slider("cycles", "Timeout", 0.0, 8000.0, 100.0, 0, "1600"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "15"),
    Opt::spin("size", "Size", -50.0, 50.0, "-12"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "loop",
    label: "Loop",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "David Bagley",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=_kTMO7oEN8U"),
        blurb: "A cellular automaton that generates loop-shaped colonies that spawn, age, and eventually die.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
