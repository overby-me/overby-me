//! Port of `hacks/abstractile.c`.
//!
//! ```text
//! Copyright (c) 2004-2009 Steve Sundstrom
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! A mosaic of interlocking tiles, laid one at a time until the screen is full,
//! then taken apart again piece by piece and rebuilt differently.
//!
//! The laying is a packing algorithm on a coarse grid. Every cell of the grid is
//! visited in a shuffled order, and each visit lays one line: from an empty cell
//! it starts a new line in a random open direction, and from a filled cell it
//! branches off the line already there, so the mosaic grows as a web rather than
//! as a scan. A line runs until it meets another or the edge, capped at a random
//! maximum; there is a fixed chance it deliberately runs into whatever blocked
//! it, which is what welds separate pieces into one shape. A line that starts
//! fresh gets a new colour, and one that branches inherits the colour of what it
//! branched from, so the mosaic is a few dozen multi-limbed shapes rather than
//! several thousand separate bars.
//!
//! Where a new line takes its colour from is the other half of it. The screen
//! has an invisible pattern painted over it, and a new line reads the colour of
//! the cell it starts in. That pattern is up to four layers deep, each layer one
//! of forty designs, stripes and checkerboards and waves and concentric shapes,
//! optionally sheared or bent first, and the layers are combined by one of
//! several rules. So the tiles are laid without regard to the picture and the
//! picture appears anyway.
//!
//! Neither the drawing nor the erasing happens in the order the lines were laid.
//! Both are sorted by a key computed from where each line starts, and there are
//! forty of those keys: left to right, outwards from a point, by diagonal, along
//! a wave, by length, by object, by colour. That is why the mosaic assembles
//! itself as a sweep or a spiral or a blossom and comes apart as a different
//! one.
//!
//! There are four tile styles: flat rectangles, rectangles with a hole punched
//! in them, and two three-dimensional ones. Both of those work by drawing every
//! line of a shape several times, each pass a pixel further in and a shade
//! along, which gives a bevel; one runs the shades light to dark and the other
//! dark to light, so one looks like tubing and the other like blocks. A fifth
//! style draws each grid cell as a piece of a tile floor with mitred corners,
//! choosing between sixteen cases by which of its four sides carry a line.
//!
//! Two things here are not upstream's. Its frame delay subtracts the time the
//! frame actually took, measured with a system clock; there is nothing sensible
//! to measure here, and the correction is small, so the delay is the target
//! interval alone. And the array of flags it allocates and never reads is not
//! allocated.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{
    Pixel, XColor, make_color_loop, make_color_ramp, make_random_colormap, make_smooth_colormap,
    make_uniform_colormap, rgb_to_hsv,
};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent, XPoint,
    random, screenhack_event_helper,
};

const BASECOLORS: usize = 30;
const MAXCOLORS: usize = 40;
const LAYERS: usize = 4;
const PATTERNS: i32 = 40;
const SHAPES: i32 = 18;
const COLORMAPS: i32 = 20;
const WAVES: i32 = 6;
const STRETCHES: i32 = 8;

/// The thirty colours the palettes are built from.
const BASECOL: [[u16; 3]; BASECOLORS] = [
    [0x3333, 0x3333, 0x3333], // dgray
    [0x6666, 0x3333, 0x0000], // dbrown
    [0x9999, 0x0000, 0x0000], // dred
    [0xFFFF, 0x6666, 0x0000], // orange
    [0xFFFF, 0xCCCC, 0x0000], // gold
    [0x6666, 0x6666, 0x0000], // olive
    [0x0000, 0x6666, 0x0000], // ivy
    [0x0000, 0x9999, 0x0000], // dgreen
    [0x3333, 0x6666, 0x6666], // bluegray
    [0x0000, 0x0000, 0x9999], // dblue
    [0x3333, 0x3333, 0xFFFF], // blue
    [0x6666, 0x0000, 0xCCCC], // dpurple
    [0x6666, 0x3333, 0xFFFF], // purple
    [0x9999, 0x3333, 0x9999], // violet
    [0xCCCC, 0x3333, 0xCCCC], // magenta
    // The lights.
    [0x3333, 0x3333, 0x3333], // gray
    [0x9999, 0x6666, 0x3333], // brown
    [0xCCCC, 0x9999, 0x3333], // tan
    [0xFFFF, 0x0000, 0x0000], // red
    [0xFFFF, 0x9999, 0x0000], // lorange
    [0xFFFF, 0xFFFF, 0x0000], // yellow
    [0x9999, 0x9999, 0x0000], // lolive
    [0x3333, 0xCCCC, 0x0000], // green
    [0x3333, 0xFFFF, 0x3333], // lgreen
    [0x0000, 0xCCCC, 0xCCCC], // cyan
    [0x3333, 0xFFFF, 0xFFFF], // sky
    [0x3333, 0x6666, 0xFFFF], // marine
    [0x3333, 0xCCCC, 0xFFFF], // lblue
    [0x9999, 0x9999, 0xFFFF], // lpurple
    [0xFFFF, 0x9999, 0xFFFF], // pink
];

const DIR_NONE: i32 = 0;
const DIR_UP: i32 = 1;
const DIR_DOWN: i32 = 2;
const DIR_LEFT: i32 = 3;
const DIR_RIGHT: i32 = 4;

const D3D_NONE: i32 = 0;
const D3D_BLOCK: i32 = 1;
const D3D_NEON: i32 = 2;
const D3D_TILED: i32 = 3;

const TILE_RANDOM: i32 = 0;
const TILE_FLAT: i32 = 1;
const TILE_THIN: i32 = 2;
const TILE_OUTLINE: i32 = 3;
const TILE_BLOCK: i32 = 4;
const TILE_NEON: i32 = 5;
const TILE_TILED: i32 = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Lay out a whole screen's worth of lines, then linger.
    Create,
    Erase,
    Draw,
}

/// One laid line: where it starts, how long, which way, and what it belongs to.
#[derive(Clone, Copy, Default)]
struct Line {
    x: i32,
    y: i32,
    len: i32,
    obj: i32,
    color: i32,
    /// The next line of the same object, for the bevelled styles.
    ndol: i32,
    /// The sort key that decides when this line is drawn or erased.
    deo: i32,
    /// True for horizontal.
    hv: bool,
}

/// What passes through one cell of the grid.
#[derive(Clone, Copy, Default)]
struct Cell {
    line: i32,
    hl: i32,
    hr: i32,
    vu: i32,
    vd: i32,
    dhl: i32,
    dhr: i32,
    dvu: i32,
    dvd: i32,
}

fn rnd(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    (random() % n as u32) as i32
}

struct Abstractile {
    fgc: Gc,
    bgc: Gc,
    colors: Vec<Pixel>,

    dline: Vec<Line>,
    eline: Vec<Line>,
    grid: Vec<Cell>,
    zlist: Vec<usize>,
    /// The most recently drawn line of each object.
    fdol: Vec<i32>,

    width: i32,
    height: i32,
    /// Draw, erase, fill, init, batch, line, erase-line, object and z indices.
    di: i32,
    fi: i32,
    ii: i32,
    bi: i32,
    li: i32,
    eli: i32,
    oi: i32,
    zi: usize,

    gridx: i32,
    gridy: i32,
    gridn: usize,
    lwid: i32,
    narray: usize,
    elwid: i32,
    elpu: i32,
    egridx: i32,
    egridy: i32,

    bnratio: i32,
    maxlen: i32,
    forcemax: i32,
    olen: i32,
    bln: i32,

    ncolors: i32,
    shades: i32,
    rco: [i32; MAXCOLORS],
    cmap: i32,
    layers: i32,
    newcols: bool,

    dmap: i32,
    emap: i32,
    dvar: i32,
    evar: i32,
    ddir: i32,
    edir: i32,
    lpu: i32,
    d3d: i32,
    round: i32,
    outline: i32,

    pattern: [i32; LAYERS],
    shape: [i32; LAYERS],
    mix: [i32; LAYERS],
    csw: [i32; LAYERS],
    wsx: [i32; LAYERS],
    wsy: [i32; LAYERS],
    sec: [i32; LAYERS],
    cs1: [i32; LAYERS],
    cs2: [i32; LAYERS],
    cs3: [i32; LAYERS],
    cs4: [i32; LAYERS],
    wave: [i32; LAYERS],
    waveh: [i32; LAYERS],
    wavel: [i32; LAYERS],
    rx1: [i32; LAYERS],
    rx2: [i32; LAYERS],
    rx3: [i32; LAYERS],
    ry1: [i32; LAYERS],
    ry2: [i32; LAYERS],
    ry3: [i32; LAYERS],

    mode: Mode,
    sleep: i32,
    speed: i32,
    tile: i32,
    dialog: i32,
    grid_full: bool,
    resized: bool,
}

fn min(a: i32, b: i32) -> i32 {
    a.min(b)
}
fn max(a: i32, b: i32) -> i32 {
    a.max(b)
}

impl Abstractile {
    fn new(d: &mut Dpy) -> Self {
        let tile = match d.res.string("tile") {
            "flat" => TILE_FLAT,
            "thin" => TILE_THIN,
            "outline" => TILE_OUTLINE,
            "block" => TILE_BLOCK,
            "neon" => TILE_NEON,
            "tiled" => TILE_TILED,
            _ => TILE_RANDOM,
        };
        Self {
            fgc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
            bgc: Gc::new(d.res.pixel("background"), d.res.pixel("background")),
            colors: vec![0; 255],
            dline: Vec::new(),
            eline: Vec::new(),
            grid: Vec::new(),
            zlist: Vec::new(),
            fdol: Vec::new(),
            width: d.width(),
            height: d.height(),
            di: 0,
            fi: 0,
            ii: 0,
            bi: 0,
            li: 0,
            eli: 0,
            oi: 0,
            zi: 0,
            gridx: 1,
            gridy: 1,
            gridn: 1,
            lwid: 3,
            narray: 0,
            elwid: 3,
            elpu: 1,
            egridx: 1,
            egridy: 1,
            bnratio: 4,
            maxlen: 2,
            forcemax: 0,
            olen: 0,
            bln: 0,
            ncolors: 1,
            shades: 1,
            rco: [0; MAXCOLORS],
            cmap: 0,
            layers: 1,
            newcols: false,
            dmap: 0,
            emap: 0,
            dvar: 10,
            evar: 10,
            ddir: 1,
            edir: 1,
            lpu: 1,
            d3d: D3D_NONE,
            round: 0,
            outline: 0,
            pattern: [0; LAYERS],
            shape: [0; LAYERS],
            mix: [0; LAYERS],
            csw: [5; LAYERS],
            wsx: [0; LAYERS],
            wsy: [0; LAYERS],
            sec: [0; LAYERS],
            cs1: [1; LAYERS],
            cs2: [1; LAYERS],
            cs3: [1; LAYERS],
            cs4: [1; LAYERS],
            wave: [0; LAYERS],
            waveh: [1; LAYERS],
            wavel: [1; LAYERS],
            rx1: [1; LAYERS],
            rx2: [1; LAYERS],
            rx3: [1; LAYERS],
            ry1: [1; LAYERS],
            ry2: [1; LAYERS],
            ry3: [1; LAYERS],
            mode: Mode::Create,
            sleep: d.res.int("sleep").clamp(0, 60),
            speed: d.res.int("speed").clamp(0, 5),
            tile,
            dialog: 0,
            grid_full: false,
            resized: true,
        }
    }

    // ---- little helpers ---------------------------------------------------

    fn dist(&self, x1: i32, x2: i32, y1: i32, y2: i32, s: i32) -> i32 {
        let xd = (x1 - x2) as f64;
        let yd = (y1 - y2) as f64;
        match s {
            0 => (xd * xd + yd * yd).sqrt() as i32,
            1 => (xd * xd * self.cs1[0] as f64 * 2.0 + yd * yd).sqrt() as i32,
            2 => (xd * xd + yd * yd * self.cs2[0] as f64 * 2.0).sqrt() as i32,
            _ => (xd * xd * self.cs1[0] as f64 / self.cs2[0] as f64
                + yd * yd * self.cs3[0] as f64 / self.cs4[0] as f64)
                .sqrt() as i32,
        }
    }

    fn wave(&self, x: i32, h: i32, l: i32, wave: i32) -> i32 {
        let l = l + 1;
        let pi = std::f64::consts::PI;
        match wave {
            // Cosine wave.
            0 => ((x as f64 * pi / l as f64).cos() * h as f64) as i32,
            // Double wave.
            1 | 2 => {
                ((x as f64 * pi / l as f64).cos() * h as f64) as i32
                    + ((x as f64 * pi / l as f64 / self.cs1[1] as f64).sin() * h as f64) as i32
            }
            // Zig zag.
            3 => (x.rem_euclid(l * 2) - l).abs() * h / l,
            // Giant zig zag.
            4 => (x.rem_euclid(l * 4) - l * 2).abs() * h * 3 / l,
            // Sawtooth.
            5 => x.rem_euclid(l) * h / l,
            // No wave.
            _ => 0,
        }
    }

    fn triangle(&self, x: i32, y: i32, rx: i32, ry: i32, t: i32) -> i32 {
        match t {
            1 => min(
                min(x + y + rx - (self.gridx / 2), self.gridx - x + y),
                (self.gridy - y + (ry / 2)) * 3 / 2,
            ),
            2 => min(min(x - rx, y - ry), (rx + ry - x - y) * 2 / 3),
            3 => min(
                min(self.gridx - x - rx, y - ry),
                (rx + ry - self.gridx + x - y) * 2 / 3,
            ),
            4 => min(
                min(x - rx, self.gridy - y - ry),
                (rx + ry - x - self.gridy + y) * 2 / 3,
            ),
            _ => min(
                min(self.gridx - x - rx, self.gridy - y - ry),
                (rx + ry - self.gridx + x - self.gridy + y) * 2 / 3,
            ),
        }
    }

    // ---- the grid ---------------------------------------------------------

    fn init_zlist(&mut self) {
        self.gridx = (self.width / self.lwid).max(1);
        self.gridy = (self.height / self.lwid).max(1);
        self.gridn = (self.gridx * self.gridy) as usize;
        for z in 0..self.gridn {
            self.grid[z] = Cell::default();
            self.zlist[z] = z;
        }
        // Rather than pulling points at random and waiting to hit the last
        // empty cells, the whole list is shuffled so they do get hit last.
        for z in 0..self.gridn {
            let y = (random() % self.gridn as u32) as usize;
            self.zlist.swap(y, z);
        }
    }

    // ---- colours ----------------------------------------------------------

    fn ramp_rgb(c1: [u16; 3], c2: [u16; 3], n: usize) -> Vec<XColor> {
        let (h1, s1, v1) = rgb_to_hsv(c1[0], c1[1], c1[2]);
        let (h2, s2, v2) = rgb_to_hsv(c2[0], c2[1], c2[2]);
        make_color_ramp(h1, s1, v1, h2, s2, v2, n, false)
    }

    fn store(&mut self, at: usize, cols: &[XColor]) {
        for (i, c) in cols.iter().enumerate() {
            if at + i < self.colors.len() {
                self.colors[at + i] = c.pixel;
            }
        }
    }

    fn init_colors(&mut self) {
        let mut basecol = BASECOL;
        let bg = self.bgc.background;
        self.colors.iter_mut().for_each(|c| *c = bg);

        if self.d3d != D3D_NONE {
            self.shades = if self.d3d == D3D_TILED {
                5
            } else {
                self.lwid / 2 + 1
            };
            self.ncolors = 4 + rnd(4);
            if self.cmap > 0 {
                // Tint the base colours a bit. Only the first two channels,
                // which is upstream's.
                for c in basecol.iter_mut() {
                    for v in c.iter_mut().take(2) {
                        if *v == 0 {
                            *v += rnd(16000) as u16;
                        } else if *v == 0xFFFF {
                            *v -= rnd(16000) as u16;
                        } else {
                            *v = v.wrapping_sub(8000).wrapping_add(rnd(16000) as u16);
                        }
                    }
                }
            }
            let mut col = [0usize; MAXCOLORS];
            match self.cmap % 4 {
                0 => {
                    for c in col.iter_mut().take(self.ncolors as usize) {
                        *c = rnd(BASECOLORS as i32) as usize;
                    }
                }
                1 => {
                    for c in col.iter_mut().take(self.ncolors as usize) {
                        *c = rnd(15) as usize;
                    }
                }
                2 => {
                    col[0] = rnd(15) as usize;
                    for c1 in 1..self.ncolors as usize {
                        col[c1] = (col[c1 - 1] + 1 + rnd(2) as usize) % 15;
                    }
                }
                _ => {
                    col[0] = rnd(15 - self.ncolors) as usize;
                    for c1 in 1..self.ncolors as usize {
                        col[c1] = col[c1 - 1] + 1;
                    }
                }
            }
            for c1 in 0..self.ncolors as usize {
                // Shift what is already there up, so the last ramp built ends
                // up first.
                let shades = self.shades as usize;
                for h1 in (0..c1 * shades).rev() {
                    self.colors[h1 + shades] = self.colors[h1];
                }
                let ramp = Self::ramp_rgb(basecol[col[c1]], [0xFFFF; 3], shades);
                self.store(0, &ramp);
            }
            return;
        }

        // Not three-dimensional.
        self.shades = 1;
        let (c1, c2, c3);
        if self.cmap % 2 != 0 {
            // Base colours.
            if rnd(3) != 0 {
                c1 = rnd(15) as usize;
                c2 = (c1 + 3 + rnd(5) as usize) % 15;
                c3 = (c2 + 3 + rnd(5) as usize) % 15;
            } else {
                c1 = rnd(BASECOLORS as i32) as usize;
                c2 = (c1 + 5 + rnd(10) as usize) % BASECOLORS;
                c3 = (c2 + 5 + rnd(10) as usize) % BASECOLORS;
            }
        } else {
            c1 = usize::MAX;
            c2 = usize::MAX;
            c3 = usize::MAX;
        }
        let (col1, mut col2, col3);
        if c1 != usize::MAX {
            col1 = basecol[c1];
            col2 = basecol[c2];
            col3 = basecol[c3];
        } else {
            // Random colours.
            col1 = [rnd(65535) as u16, rnd(65535) as u16, rnd(65535) as u16];
            let step = |v: u16| ((v as i32 + 16384 + rnd(32768)) % 65535) as u16;
            col2 = [step(col1[0]), step(col1[1]), step(col1[2])];
            col3 = [step(col2[0]), step(col2[1]), step(col2[2])];
        }

        match self.cmap {
            // A ramp from one colour to another, or to white.
            0..=3 => {
                self.ncolors = 5 + rnd(5);
                if self.cmap > 1 {
                    col2 = [0xFFFF; 3];
                }
                let closed = rnd(2) != 0;
                let (h1, s1, v1) = rgb_to_hsv(col1[0], col1[1], col1[2]);
                let (h2, s2, v2) = rgb_to_hsv(col2[0], col2[1], col2[2]);
                let ramp = make_color_ramp(h1, s1, v1, h2, s2, v2, self.ncolors as usize, closed);
                self.store(0, &ramp);
            }
            // A loop through three colours.
            4..=7 => {
                self.ncolors = 8 + rnd(12);
                let (h1, s1, v1) = rgb_to_hsv(col1[0], col1[1], col1[2]);
                let (h2, s2, v2) = rgb_to_hsv(col2[0], col2[1], col2[2]);
                let (h3, s3, v3) = rgb_to_hsv(col3[0], col3[1], col3[2]);
                let loop_ =
                    make_color_loop(h1, s1, v1, h2, s2, v2, h3, s3, v3, self.ncolors as usize);
                self.store(0, &loop_);
            }
            8 | 9 => {
                self.ncolors = rnd(4) * 6 + 12;
                let cols = make_smooth_colormap(self.ncolors as usize);
                self.store(0, &cols);
            }
            10 => {
                self.ncolors = rnd(4) * 6 + 12;
                let cols = make_uniform_colormap(self.ncolors as usize);
                self.store(0, &cols);
            }
            // Two or three dark-to-light blends, interleaved.
            11..=14 => {
                self.ncolors = 7;
                let n = self.ncolors as usize;
                let t1 = Self::ramp_rgb(col1, [0xFFFF; 3], n);
                let t2 = Self::ramp_rgb(col2, [0xFFFF; 3], n);
                if self.cmap < 13 {
                    for c in 0..=4 {
                        self.colors[c * 2] = t1[c].pixel;
                        self.colors[c * 2 + 1] = t2[c].pixel;
                    }
                    self.ncolors = 10;
                } else {
                    let t3 = Self::ramp_rgb(col3, [0xFFFF; 3], n);
                    for c in 0..=4 {
                        self.colors[c * 3] = t1[c].pixel;
                        self.colors[c * 3 + 1] = t2[c].pixel;
                        self.colors[c * 3 + 2] = t3[c].pixel;
                    }
                    self.ncolors = 15;
                }
            }
            _ => {
                self.ncolors = rnd(4) * 6 + 12;
                let cols = make_random_colormap(self.ncolors as usize, false);
                self.store(0, &cols);
            }
        }

        // A random colour order for drawing and erasing by colour.
        for (i, c) in self.rco.iter_mut().enumerate() {
            *c = i as i32;
        }
        for c1 in 0..MAXCOLORS {
            let c3 = rnd(MAXCOLORS as i32) as usize;
            self.rco.swap(c1, c3);
        }
    }

    // ---- the sort keys ----------------------------------------------------

    fn hv(&self, x: i32, y: i32, d1: i32, d2: i32, sign: i32, de: i32) -> i32 {
        let pick = |d: i32| match d {
            0 => {
                if de != 0 {
                    self.egridx - x
                } else {
                    self.gridx - x
                }
            }
            1 => y,
            2 => x,
            _ => {
                if de != 0 {
                    self.egridy - y
                } else {
                    self.gridy - y
                }
            }
        };
        let (v1, v2) = (pick(d1), pick(d2));
        let li = self.li as usize;
        let horizontal = if de != 0 {
            self.dline[li].hv
        } else {
            self.eline[li].hv
        };
        if horizontal {
            (v1 + 10000) * sign
        } else {
            (v2 + 10000) * -sign
        }
    }

    /// `_getdeo`: the key that decides when a line is drawn or erased.
    fn getdeo(&self, x: i32, y: i32, map: i32, de: i32) -> i32 {
        let d = de as usize;
        let li = self.li as usize;
        match map {
            0 => x,
            1 => y,
            2 => min(x, self.gridx - x) + 1,
            3 => min(y, self.gridy - y) + 1,
            4 => max((x - self.rx3[d]).abs(), (y - self.ry3[d]).abs()) + 1,
            5 => {
                min(
                    max((x - (self.rx3[d] / 2)).abs(), (y - self.ry3[d]).abs()),
                    max(
                        (x - (self.gridx - (self.rx2[d] / 2))).abs(),
                        (y - self.ry2[d]).abs(),
                    ),
                ) + 1
            }
            6 => {
                max(
                    (x - self.rx3[d]).abs(),
                    (y - self.ry3[d]).abs() * self.cs1[d],
                ) + 1
            }
            7 => {
                max(
                    (x - self.rx3[d]).abs() * self.cs1[d],
                    (y - self.ry3[d]).abs(),
                ) + 1
            }
            8 => min((x - self.rx3[d]).abs(), (y - self.ry3[d]).abs()) + 1,
            9 => (x * 3 / 4 + y) + 1,
            10 => (x * 3 / 4 + self.gridy - y) + 1,
            11 => ((x - self.rx3[d]).abs() + (y - self.ry3[d]).abs()) / 2 + 1,
            12 => {
                min(
                    (x - (self.rx3[d] / 2)).abs() + (y - self.ry3[d]).abs(),
                    (x - (self.gridx - (self.rx2[d] / 2))).abs() + (y - self.ry2[d]).abs(),
                ) / 2
                    + 1
            }
            13 => self.dist(x, self.rx3[d], y, self.ry3[d], 0) + 1,
            14 => self.dist(x, self.rx3[d], y, self.ry3[d], 1) + 1,
            15 => self.dist(x, self.rx3[d], y, self.ry3[d], 2) + 1,
            16 => {
                min(
                    self.dist(x, self.rx3[d] / 2, y, self.ry3[d], 0),
                    self.dist(x, self.gridx - (self.rx2[d] / 2), y, self.ry2[d], 0),
                ) + 1
            }
            17 => {
                x + self.wave(
                    self.gridy + y,
                    self.csw[0] * self.cs1[0],
                    self.csw[0] * self.cs2[0],
                    self.wave[d],
                )
            }
            18 => {
                y + self.wave(
                    self.gridx + x,
                    self.csw[0] * self.cs1[0],
                    self.csw[0] * self.cs2[0],
                    self.wave[d],
                )
            }
            19 => {
                x + self.wave(
                    self.gridy + y + ((x / 5) * self.edir),
                    self.csw[d] * self.cs1[d],
                    self.csw[d] * self.cs2[d],
                    self.wave[d],
                ) + 1
            }
            20 => {
                y + self.wave(
                    self.gridx + x + ((y / 5) * self.edir),
                    self.csw[d] * self.cs1[d],
                    self.csw[d] * self.cs2[d],
                    self.wave[d],
                ) + 1
            }
            21 => self.hv(x, y, self.cs1[0] % 2, self.cs2[0] % 2, 1, de),
            22 => self.hv(x, y, self.cs1[0] % 2, self.cs2[0] % 2, -1, de),
            23 => {
                let len = if de != 0 {
                    self.dline[li].len
                } else {
                    self.eline[li].len
                };
                len * 1000 + rnd(5000)
            }
            24..=27 => {
                let obj = if de != 0 {
                    self.dline[li].obj
                } else {
                    self.eline[li].obj
                };
                obj * 100
            }
            _ => {
                let mut cr = if de != 0 {
                    self.dline[li].color
                } else {
                    self.eline[li].color
                };
                if map < 34 {
                    cr = self.rco[(cr as usize).min(MAXCOLORS - 1)];
                }
                if (map % 6 < 4) || (de != 0) {
                    cr * 1000 + rnd(1000)
                } else if map % 6 == 4 {
                    cr * self.gridx + (x + rnd(self.gridx / 2))
                } else {
                    cr * self.gridy + (y + rnd(self.gridy / 2))
                }
            }
        }
    }

    // ---- the invisible picture --------------------------------------------

    fn shape(&self, x: i32, y: i32, rx: i32, ry: i32, n: usize) -> i32 {
        match self.shape[n] {
            // Square or rectangle.
            0..=2 => {
                1 + max(
                    (x - rx).abs() * self.cs1[n] / self.cs2[n],
                    (y - ry).abs() * self.cs3[n] / self.cs4[n],
                )
            }
            // Diamond.
            3 | 4 => {
                1 + ((x - rx).abs() * self.cs1[n] / self.cs2[n]
                    + (y - ry).abs() * self.cs3[n] / self.cs4[n])
            }
            // Eight-pointed star.
            5 => {
                1 + min(
                    max((x - rx).abs(), (y - ry).abs()) * 3 / 2,
                    (x - rx).abs() + (y - ry).abs(),
                )
            }
            // Circle or oval.
            6..=8 => 1 + self.dist(x, rx, y, ry, self.cs1[n]),
            // Black hole circle.
            9 => 1 + (self.gridx * self.gridy / (1 + self.dist(x, rx, y, ry, self.cs2[n]))),
            // Sun.
            10 => {
                1 + min(
                    (x - rx).abs() * self.gridx / ((y - ry).abs() + 1),
                    (y - ry).abs() * self.gridx / ((x - rx).abs() + 1),
                )
            }
            // Two circles and an inverted one.
            11 => {
                1 + (self.dist(x, rx, y, ry, self.cs1[n])
                    * self.dist(
                        x,
                        (rx * 3).rem_euclid(self.gridx),
                        y,
                        (ry * 5).rem_euclid(self.gridy),
                        self.cs1[n],
                    )
                    / (1 + self.dist(
                        x,
                        (rx * 4).rem_euclid(self.gridx),
                        y,
                        (ry * 7).rem_euclid(self.gridy),
                        self.cs1[n],
                    )))
            }
            // Star.
            12 => 1 + (((x - rx) * (y - ry)).abs() as f64).sqrt() as i32,
            // Centred ellipse.
            13 => {
                1 + self.dist(x, rx, y, ry, 0)
                    + self.dist(x, self.gridx - rx, y, self.gridy - ry, 0)
            }
            // Triangle.
            _ => 1 + self.triangle(x, y, rx, ry, self.cs4[n]),
        }
    }

    fn pattern(&self, x: i32, y: i32, n: usize) -> i32 {
        let ox = x;
        let mut x = x;
        let mut y = y;
        let pi = std::f64::consts::PI;
        match self.wsx[n] {
            // Slants.
            0 => x += y / (1 + self.cs4[n]),
            1 => x += (self.gridy - y) / (1 + self.cs4[n]),
            // Curves.
            2 => x += self.wave(y, self.gridx / (1 + self.cs1[n]), self.gridy, 0),
            3 => {
                x += self.wave(
                    self.gridy - y,
                    self.gridy / (1 + self.cs1[n]),
                    self.gridy,
                    0,
                )
            }
            // U curves.
            4 => {
                x += self.wave(
                    y,
                    self.cs1[n] * self.csw[n] / 2,
                    (self.gridy as f64 * 2.0 / pi) as i32,
                    0,
                )
            }
            5 => {
                x -= self.wave(
                    y,
                    self.cs1[n] * self.csw[n] / 2,
                    (self.gridy as f64 * 2.0 / pi) as i32,
                    0,
                )
            }
            _ => {}
        }
        match self.wsy[0] {
            0 => y += ox / (1 + self.cs1[n]),
            1 => y += (self.gridx - ox) / (1 + self.cs1[n]),
            2 => y += self.wave(ox, self.gridx / (1 + self.cs1[n]), self.gridx, 0),
            3 => {
                y += self.wave(
                    self.gridx - ox,
                    self.gridx / (1 + self.cs1[n]),
                    self.gridx,
                    0,
                )
            }
            4 => {
                y += self.wave(
                    ox,
                    self.cs1[n] * self.csw[n] / 2,
                    (self.gridy as f64 * 2.0 / pi) as i32,
                    0,
                )
            }
            5 => {
                y -= self.wave(
                    ox,
                    self.cs1[n] * self.csw[n] / 2,
                    (self.gridy as f64 * 2.0 / pi) as i32,
                    0,
                )
            }
            _ => {}
        }

        let csw = self.csw[n].max(1);
        let (cs1, cs2, cs3, cs4) = (self.cs1[n], self.cs2[n], self.cs3[n], self.cs4[n]);
        let mut v = match self.pattern[n] {
            0 => y,
            1 => x,
            2 => x + (y * cs1 / cs2),
            3 => x - (y * cs1 / cs2),
            // Checkerboard.
            4 => (y / csw * 3 + x / csw) * csw,
            // Diagonal checkerboard.
            5 => ((x + y) / 2 / csw + (x + self.gridy - y) / 2 / csw * 3) * csw,
            // Crosses.
            6 => self.gridx + (min((x - self.rx3[n]).abs(), (y - self.ry3[n]).abs()) * 2),
            7 => {
                min(
                    min((x - self.rx2[n]).abs(), (y - self.ry2[n]).abs()),
                    min((x - self.rx1[n]).abs(), (y - self.ry1[n]).abs()),
                ) * 2
            }
            8 => {
                self.gridx
                    + (min(
                        (x - self.rx3[n]).abs() * cs1 / cs2 + (y - self.ry2[n]).abs() * cs3 / cs4,
                        (x - self.rx3[n]).abs() * cs1 / cs2 - (y - self.ry3[n]).abs() * cs3 / cs4,
                    ) * 2)
            }
            9 => {
                min(
                    min(
                        (x - self.rx2[n]).abs() + (y - self.ry2[n]).abs(),
                        (x - self.rx2[n]).abs() - (y - self.ry2[n]).abs(),
                    ),
                    min(
                        (x - self.rx1[n]).abs() + (y - self.ry1[n]).abs(),
                        (x - self.rx1[n]).abs() - (y - self.ry1[n]).abs(),
                    ),
                ) * 2
            }
            // Stripes with waves.
            10 => self.gridy + (y + self.wave(x, self.waveh[n], self.wavel[n], self.wave[n])),
            11 => self.gridx + (x + self.wave(y, self.waveh[n], self.wavel[n], self.wave[n])),
            12 => {
                self.gridx
                    + (x + (y * cs1 / cs2)
                        + self.wave(x, self.waveh[n], self.wavel[n], self.wave[n]))
            }
            13 => {
                self.gridx
                    + (x - (y * cs1 / cs2)
                        + self.wave(y, self.waveh[n], self.wavel[n], self.wave[n]))
            }
            // Spikey waves.
            14 => {
                y + (csw * cs4 / cs3)
                    + self.wave(
                        x + ((y / cs3) * self.edir),
                        csw / 2 * cs1 / cs2,
                        csw / 2 * cs2 / cs1,
                        self.wave[n],
                    )
            }
            15 => {
                x + (csw * cs1 / cs2)
                    + self.wave(
                        y + ((x / cs3) * self.edir),
                        csw / 2 * cs1 / cs2,
                        csw / 2 * cs3 / cs4,
                        self.wave[n],
                    )
            }
            // Big slanted waves.
            16 => {
                self.gridy - y - (x * cs1 / cs3)
                    + (csw * cs1 * cs2)
                    + self.wave(x, csw / 3 * cs1 * cs2, csw / 3 * cs3 * cs2, self.wave[n])
            }
            17 => {
                x - (y * cs1 / cs3)
                    + (csw * cs1 * cs2)
                    + self.wave(y, csw / 3 * cs1 * cs2, csw / 3 * cs3 * cs2, self.wave[n])
            }
            // Double waves.
            18 => {
                y + (y + csw * cs3)
                    + self.wave(x, csw / 3 * cs3, csw / 3 * cs2, self.wave[n])
                    + self.wave(x, csw / 3 * cs4, csw / 3 * cs1 * 3 / 2, self.wave[n])
            }
            19 => {
                x + (x + csw * cs1)
                    + self.wave(y, csw / 3 * cs1, csw / 3 * cs3, self.wave[n])
                    + self.wave(y, csw / 3 * cs2, csw / 3 * cs4 * 3 / 2, self.wave[n])
            }
            // One shape.
            20..=22 => self.shape(x, y, self.rx3[n], self.ry3[n], n),
            // Two shapes.
            23..=25 => min(
                self.shape(x, y, self.rx1[n], self.ry1[n], n),
                self.shape(x, y, self.rx2[n], self.ry2[n], n),
            ),
            // Two shapes, opposite.
            26 | 27 => min(
                self.shape(x, y, self.rx2[n], self.ry2[n], n),
                self.shape(x, y, self.gridx - self.rx2[n], self.gridy - self.rx2[n], n),
            ),
            // Two shapes as a checkerboard.
            28 | 29 => {
                ((self.shape(x, y, self.rx1[n], self.ry1[n], n) / csw)
                    + (self.shape(x, y, self.rx2[n], self.ry2[n], n) / csw))
                    * csw
            }
            // Two shapes blended.
            30 | 31 => {
                (self.shape(x, y, self.rx1[n], self.ry1[n], n)
                    + self.shape(x, y, self.rx2[n], self.ry2[n], n))
                    / 2
            }
            32 | 33 => {
                (self.shape(x, y, self.rx1[n], self.ry1[n], n)
                    + self.shape(self.gridx - x, self.gridy - y, self.rx1[n], self.ry1[n], n))
                    / 2
            }
            // Three shapes.
            34 | 35 => min(
                self.shape(x, y, self.rx3[n], self.ry3[n], n),
                min(
                    self.shape(x, y, self.rx1[n], self.ry1[n], n),
                    self.shape(x, y, self.rx2[n], self.ry2[n], n),
                ),
            ),
            36 | 37 => {
                (self.shape(x, y, self.rx1[n], self.ry1[n], n)
                    + self.shape(x, y, self.rx2[n], self.ry2[n], n)
                    + self.shape(x, y, self.rx3[n], self.ry3[n], n))
                    / 3
            }
            // Four shapes. Upstream's comma expression throws the first pair
            // away, so only the second is used; kept as it stands.
            38 => min(
                self.shape(x, y, self.gridx - self.rx2[n], self.ry2[n], n),
                self.shape(x, y, self.rx2[n], self.gridy - self.ry2[n], n),
            ),
            // Four rainbows, the same way.
            _ => min(
                self.shape(x, y, self.rx2[n] / 2, self.gridy - self.csw[n], n),
                self.shape(
                    x,
                    y,
                    self.gridx - self.csw[n],
                    self.gridy - (self.ry2[n] / 2),
                    n,
                ),
            ),
        };

        // Stretch or contract the stripe.
        match self.sec[n] {
            0 => {
                v = ((((v.abs() as f64) * self.gridx as f64).sqrt() as i32 as f64
                    * self.gridx as f64)
                    .sqrt()) as i32
            }
            1 => v = ((v as f64).powi(2) as i32) / self.gridx.max(1),
            _ => {}
        }
        v.abs()
    }

    /// The colour a new line starting here takes, out of the layered pattern.
    fn getcolor(&self, x: i32, y: i32) -> i32 {
        let mut cv0 = 0;
        for n in 0..self.layers as usize {
            let cvn = self.pattern(x, y, n);
            let cswn = self.csw[n].max(1);
            let csw0 = self.csw[0].max(1);
            cv0 = if n == 0 {
                cvn / csw0
            } else if self.mix[n] < 5 {
                (cv0 * csw0 + cvn) / cswn
            } else if self.mix[n] < 12 {
                cv0 + (cvn / cswn * self.ncolors / 2)
            } else if self.mix[n] < 16 {
                cv0 + (cvn / cswn)
            } else if self.mix[n] < 18 {
                cv0 - (cvn / cswn)
            } else if self.mix[n] == 18 {
                ((cv0 * x) + (cvn * (self.gridx - x) / cswn)) / self.gridx.max(1)
            } else {
                ((cv0 * y) + (cvn * (self.gridy - y) / cswn)) / self.gridy.max(1)
            };
        }
        cv0
    }
}

impl Abstractile {
    // ---- laying lines -----------------------------------------------------

    /// How far the line can run from here, and what stops it.
    fn findopen(&mut self, x: i32, y: i32, z: usize) -> i32 {
        let g = self.grid[z];
        if (g.hl != 0 || g.hr != 0) && (g.vu != 0 || g.vd != 0) {
            return DIR_NONE;
        }
        let mut od = [0i32; 4];
        let mut no = 0;
        let gridx = self.gridx as usize;
        if z > gridx && g.hl == 0 && g.hr == 0 && self.grid[z - gridx].line == 0 {
            od[no] = DIR_UP;
            no += 1;
        }
        if z < self.gridn - gridx && g.hl == 0 && g.hr == 0 && self.grid[z + gridx].line == 0 {
            od[no] = DIR_DOWN;
            no += 1;
        }
        if x != 0 && g.hl == 0 && g.hr == 0 && self.grid[z - 1].line == 0 {
            od[no] = DIR_LEFT;
            no += 1;
        }
        if !(z + 1).is_multiple_of(gridx) && g.hl == 0 && g.hr == 0 && self.grid[z + 1].line == 0 {
            od[no] = DIR_RIGHT;
            no += 1;
        }
        if no == 0 {
            return DIR_NONE;
        }
        let dir = od[rnd(no as i32) as usize];
        self.olen = 0;
        self.bln = 0;
        while self.olen <= self.maxlen && self.bln == 0 {
            self.olen += 1;
            let o = self.olen;
            self.bln = match dir {
                DIR_UP => {
                    if y - o < 0 {
                        -1
                    } else {
                        self.grid[z - (o as usize * gridx)].line
                    }
                }
                DIR_DOWN => {
                    if y + o >= self.gridy {
                        -1
                    } else {
                        self.grid[z + (o as usize * gridx)].line
                    }
                }
                DIR_LEFT => {
                    if x - o < 0 {
                        -1
                    } else {
                        self.grid[z - o as usize].line
                    }
                }
                _ => {
                    if x + o >= self.gridx {
                        -1
                    } else {
                        self.grid[z + o as usize].line
                    }
                }
            };
        }
        self.olen -= 1;
        dir
    }

    fn fillgrid(&mut self) {
        let li = self.li;
        let line = self.dline[li as usize];
        let mut gridc = (self.gridx * line.y + line.x) as usize;
        let add = if line.hv { 1 } else { self.gridx as usize };
        for n in 0..=line.len {
            if n != 0 {
                gridc += add;
            }
            if gridc >= self.grid.len() {
                return;
            }
            if self.grid[gridc].line == 0 {
                self.fi += 1;
                self.grid[gridc].line = li;
            }
            if line.hv {
                if n != 0 {
                    self.grid[gridc].hr = li;
                }
                if n < line.len {
                    self.grid[gridc].hl = li;
                }
            } else {
                if n != 0 {
                    self.grid[gridc].vd = li;
                }
                if n < line.len {
                    self.grid[gridc].vu = li;
                }
            }
            if self.fi >= self.gridn as i32 {
                self.grid_full = true;
                return;
            }
        }
    }

    fn newline(&mut self) {
        let z = self.zlist[self.zi];
        let x = (z % self.gridx as usize) as i32;
        let y = (z / self.gridx as usize) as i32;
        self.zi += 1;
        let mut dir = self.findopen(x, y, z);
        let mut bl = 0;
        let is_new;

        if self.grid[z].line == 0 {
            // An empty space: a new line, unless nothing around it is open.
            if dir == DIR_NONE {
                // Nothing is open, so force a length-one branch any way that
                // stays on the grid.
                is_new = false;
                let mut guard = 0;
                while dir == DIR_NONE
                    || (dir == DIR_UP && y == 0)
                    || (dir == DIR_DOWN && y + 1 == self.gridy)
                    || (dir == DIR_LEFT && x == 0)
                    || (dir == DIR_RIGHT && x + 1 == self.gridx)
                {
                    dir = rnd(4);
                    guard += 1;
                    if guard > 1000 {
                        return;
                    }
                }
                let bz = match dir {
                    DIR_UP => z - self.gridx as usize,
                    DIR_DOWN => z + self.gridx as usize,
                    DIR_LEFT => z - 1,
                    _ => z + 1,
                };
                bl = self.grid[bz].line;
                self.li += 1;
                self.dline[self.li as usize] = Line {
                    len: 1,
                    ..Line::default()
                };
            } else if self.bnratio > 1
                && self.bln > 0
                && self.olen < self.maxlen
                && rnd(self.bnratio) != 0
            {
                // Run into the line that blocked this one, welding them.
                is_new = false;
                bl = self.bln;
                self.li += 1;
                self.dline[self.li as usize] = Line {
                    len: self.olen + 1,
                    ..Line::default()
                };
            } else {
                // A new line, and a new object.
                is_new = true;
                self.oi += 1;
                self.li += 1;
                let len = if self.forcemax == 0 {
                    self.olen
                } else {
                    1 + rnd(self.olen)
                };
                self.dline[self.li as usize] = Line {
                    len,
                    ..Line::default()
                };
            }
        } else {
            // A filled space: branch out of the line already here.
            if dir == DIR_NONE {
                return;
            }
            is_new = false;
            bl = self.grid[z].line;
            self.li += 1;
            let len = if self.forcemax == 0 {
                self.olen
            } else {
                1 + rnd(self.olen)
            };
            self.dline[self.li as usize] = Line {
                len,
                ..Line::default()
            };
        }

        let li = self.li as usize;
        self.dline[li].x = if dir == DIR_LEFT {
            x - self.dline[li].len
        } else {
            x
        };
        self.dline[li].y = if dir == DIR_UP {
            y - self.dline[li].len
        } else {
            y
        };
        self.dline[li].hv = dir == DIR_LEFT || dir == DIR_RIGHT;
        self.dline[li].obj = if is_new {
            self.oi
        } else {
            self.dline[bl.max(0) as usize].obj
        };
        self.dline[li].color = if is_new {
            self.getcolor(x, y).rem_euclid(self.ncolors.max(1))
        } else {
            self.dline[bl.max(0) as usize].color
        };
        let dmap = self.dmap;
        let dvar = self.dvar.max(1);
        self.dline[li].deo = (self.getdeo(x, y, dmap, 1) + rnd(dvar) + rnd(dvar)) * self.ddir;
        self.dline[li].ndol = 0;
        self.fillgrid();
    }

    // ---- the screen -------------------------------------------------------

    fn init_screen(&mut self) {
        if self.resized {
            self.narray = ((self.width + 1) as usize * (self.height + 1) as usize) / 4 + 1;
            self.dline = vec![Line::default(); self.narray];
            self.eline = vec![Line::default(); self.narray];
            self.grid = vec![Cell::default(); self.narray];
            self.zlist = vec![0; self.narray];
            self.fdol = vec![0; self.narray];
            self.dialog = if self.width < 500 { 1 } else { 0 };
            self.resized = false;
        }

        if self.ii != 0 {
            // Swap the two line arrays: what was drawn is what gets erased.
            std::mem::swap(&mut self.dline, &mut self.eline);
            self.eli = self.li;
            self.elwid = self.lwid;
            self.elpu = self.lpu;
            self.egridx = self.gridx;
            self.egridy = self.gridy;

            let (emap, evar, edir) = (self.emap, self.evar.max(1), self.edir);
            for li in 1..=self.eli {
                self.li = li;
                let (x, y) = (self.eline[li as usize].x, self.eline[li as usize].y);
                self.eline[li as usize].deo =
                    (self.getdeo(x, y, emap, 0) + rnd(evar) + rnd(evar)) * edir;
            }
            let n = self.eli as usize + 1;
            self.eline[..n].sort_by_key(|l| l.deo);
        }
        self.ii += 1;

        self.di = 0;
        self.fi = 0;
        self.li = 0;
        self.oi = 0;
        self.zi = 0;
        self.grid_full = false;
        self.dline[0] = Line {
            // Kept first by the sort, so the draw index is never null.
            deo: -999_999_999,
            ..Line::default()
        };

        self.lwid = if self.ii == 1 { 3 } else { 2 + ((rnd(6)) % 4) };
        self.d3d = if self.tile == TILE_FLAT || self.tile == TILE_THIN || self.tile == TILE_OUTLINE
        {
            D3D_NONE
        } else if self.tile == TILE_BLOCK {
            D3D_BLOCK
        } else if self.tile == TILE_NEON {
            D3D_NEON
        } else if self.tile == TILE_TILED {
            D3D_TILED
        } else if self.ii == 1 && !self.newcols {
            // Force the tiled style on the first screen so every shade is
            // loaded.
            D3D_TILED
        } else {
            (rnd(5)) % 4
        };
        self.outline = if self.tile == TILE_OUTLINE {
            1
        } else if self.tile != TILE_RANDOM || rnd(5) != 0 {
            0
        } else {
            1
        };
        self.round = if self.d3d == D3D_NEON {
            1
        } else if self.d3d == D3D_BLOCK || self.outline != 0 || rnd(6) != 0 {
            0
        } else {
            1
        };
        if self.d3d != D3D_NONE || self.outline != 0 || self.round != 0 {
            self.lwid += 2;
        }
        if self.d3d == D3D_NONE && self.round == 0 && self.outline == 0 && self.lwid > 3 {
            self.lwid -= 2;
        }
        if self.d3d == D3D_TILED {
            self.lwid += 1;
        }
        if self.tile == TILE_THIN {
            self.lwid = 2;
        }
        if self.width > 2560 || self.height > 2560 {
            // Retina displays.
            self.lwid *= 3;
        }

        self.init_zlist();

        self.maxlen = if self.lwid > 6 {
            2 + rnd(4)
        } else if self.lwid > 4 {
            2 + (rnd(8)) % 6
        } else if self.lwid > 2 {
            2 + (rnd(12)) % 8
        } else {
            2 + (rnd(15)) % 10
        };
        self.bnratio = 4 + rnd(4) + rnd(4);
        self.forcemax = if rnd(6) != 0 { 0 } else { 1 };

        if self.ii == 1 || self.newcols {
            self.init_colors();
        }

        // Upstream computes a draw order from the erase order and then throws
        // it away on the next line; only the second assignment survives.
        self.dmap = 20 + rnd(20);
        self.dvar = if self.dmap > 22 {
            100
        } else {
            10 + (self.csw[0] * rnd(5))
        };
        self.ddir = if rnd(2) != 0 { 1 } else { -1 };

        self.emap = (self.dmap + 10 + rnd(10)) % 20;
        self.evar = if self.emap > 22 {
            100
        } else {
            10 + (self.csw[0] * rnd(5))
        };
        self.edir = if rnd(2) != 0 { 1 } else { -1 };

        self.layers = if rnd(2) != 0 {
            2
        } else if rnd(2) != 0 {
            1
        } else if rnd(2) != 0 {
            3
        } else {
            4
        };
        self.cmap = (self.cmap + 5 + rnd(10)) % COLORMAPS;

        for x in 0..LAYERS {
            self.pattern[x] = rnd(PATTERNS);
            self.shape[x] = rnd(SHAPES);
            self.mix[x] = rnd(20);
            let nstr = match self.lwid {
                2 => 20 + rnd(12),
                3 => 16 + rnd(8),
                4 => 12 + rnd(6),
                5 => 10 + rnd(5),
                6 => 8 + rnd(4),
                _ => 5 + rnd(5),
            };
            self.csw[x] = max(5, self.gridy / nstr.max(1));
            self.wsx[x] = (self.wsx[x] + 3 + rnd(3)) % STRETCHES;
            self.wsy[x] = (self.wsy[x] + 3 + rnd(3)) % STRETCHES;
            self.sec[x] = rnd(5);
            if self.dialog == 0 && self.sec[x] < 2 {
                self.csw[x] /= 2;
            }
            let spread = |dialog: i32| {
                if dialog != 0 { 1 + rnd(3) } else { 2 + rnd(5) }
            };
            self.cs1[x] = spread(self.dialog);
            self.cs2[x] = spread(self.dialog);
            self.cs3[x] = spread(self.dialog);
            self.cs4[x] = spread(self.dialog);
            self.wave[x] = rnd(WAVES);
            self.wavel[x] = self.csw[x] * (2 + rnd(6));
            self.waveh[x] = self.csw[x] * (1 + rnd(3));
            self.rx1[x] = self.gridx / 10 + rnd(self.gridx * 8 / 10);
            self.ry1[x] = self.gridy / 10 + rnd(self.gridy * 8 / 10);
            self.rx2[x] = self.gridx * 2 / 10 + rnd(self.gridx * 6 / 10);
            self.ry2[x] = self.gridy * 2 / 10 + rnd(self.gridy * 6 / 10);
            self.rx3[x] = self.gridx * 3 / 10 + rnd(self.gridx * 4 / 10);
            self.ry3[x] = self.gridy * 3 / 10 + rnd(self.gridy * 4 / 10);
        }
    }

    fn create_screen(&mut self) {
        while !self.grid_full && self.zi < self.gridn {
            self.newline();
        }
        let n = self.li as usize + 1;
        self.dline[..n].sort_by_key(|l| l.deo);
        // A two-hundredth of the lines per frame, so the picture assembles at
        // the same rate whatever it is made of.
        self.lpu = if self.dialog != 0 {
            self.li / 50
        } else {
            self.li / 200
        };
        if self.lpu == 0 {
            self.lpu = 1;
        }
        self.bi = 1;
        self.mode = Mode::Erase;
    }

    // ---- drawing ----------------------------------------------------------

    fn fill_outline(&mut self, d: &mut Dpy, di: i32) {
        if di == 0 {
            return;
        }
        let line = self.dline[di as usize];
        let x = line.x * self.lwid + 1;
        let y = line.y * self.lwid + 1;
        let (w, h) = if line.hv {
            ((line.len + 1) * self.lwid - 3, self.lwid - 3)
        } else {
            (self.lwid - 3, (line.len + 1) * self.lwid - 3)
        };
        d.win().fill_rectangle(&self.bgc, x, y, w, h);
    }

    fn fill_line(&mut self, d: &mut Dpy, di: i32, adj: i32) {
        let line = self.dline[di as usize];
        let mut x = line.x * self.lwid;
        let mut y = line.y * self.lwid;
        let (mut w, mut h) = if line.hv {
            ((line.len + 1) * self.lwid - 1, self.lwid - 1)
        } else {
            (self.lwid - 1, (line.len + 1) * self.lwid - 1)
        };
        match self.d3d {
            D3D_NEON => {
                x += adj;
                y += adj;
                w -= adj * 2;
                h -= adj * 2;
            }
            D3D_BLOCK => {
                x += adj;
                y += adj;
                w -= self.lwid / 2 - 1;
                h -= self.lwid / 2 - 1;
            }
            _ => {}
        }
        if self.round == 0 {
            d.win().fill_rectangle(&self.fgc, x, y, w, h);
        } else if h < self.lwid {
            // Rounded ends, horizontal.
            let a = (h - 1) / 2;
            for b in 0..=a {
                d.win()
                    .fill_rectangle(&self.fgc, x + b, y + a - b, w - b * 2, h - ((a - b) * 2));
            }
        } else {
            let a = (w - 1) / 2;
            for b in 0..=a {
                d.win()
                    .fill_rectangle(&self.fgc, x + a - b, y + b, w - ((a - b) * 2), h - b * 2);
            }
        }
    }

    fn poly(&mut self, d: &mut Dpy, color: usize, pts: &[XPoint]) {
        self.fgc
            .set_foreground(self.colors[color.min(self.colors.len() - 1)]);
        d.win().fill_polygon(&self.fgc, pts);
    }

    fn tri(&mut self, d: &mut Dpy, c: usize, p: [(i32, i32); 3]) {
        let pts: Vec<XPoint> = p.iter().map(|&(x, y)| XPoint { x, y }).collect();
        self.poly(d, c, &pts);
    }

    fn quad(&mut self, d: &mut Dpy, c: usize, p: [(i32, i32); 4]) {
        let pts: Vec<XPoint> = p.iter().map(|&(x, y)| XPoint { x, y }).collect();
        self.poly(d, c, &pts);
    }

    /// The tile-floor style: each cell is drawn as a piece with mitred
    /// corners, picked out of sixteen cases by which sides carry a line.
    fn draw_tiled(&mut self, d: &mut Dpy, color: usize) {
        let line = self.dline[self.di as usize];
        let a = if line.hv { 1 } else { self.gridx as usize };
        let mut z = (line.y * self.gridx + line.x) as usize;
        let m1 = (self.lwid - 1) / 2;
        let m2 = self.lwid / 2;
        let lr = self.lwid - 1;
        let nl = self.lwid;

        for c in 0..=line.len {
            if z >= self.grid.len() {
                return;
            }
            let (x, y) = if line.hv {
                let p = ((line.x + c) * self.lwid, line.y * self.lwid);
                if c != 0 {
                    self.grid[z].dhr = self.di;
                }
                if c < line.len {
                    self.grid[z].dhl = self.di;
                }
                p
            } else {
                let p = (line.x * self.lwid, (line.y + c) * self.lwid);
                if c != 0 {
                    self.grid[z].dvd = self.di;
                }
                if c < line.len {
                    self.grid[z].dvu = self.di;
                }
                p
            };

            let g = self.grid[z];
            let mut dd = 0;
            if g.dhl != 0 {
                dd += 8;
            }
            if g.dhr != 0 {
                dd += 4;
            }
            if g.dvu != 0 {
                dd += 2;
            }
            if g.dvd != 0 {
                dd += 1;
            }

            // The base of the line: two shades side by side.
            match dd {
                1 | 2 | 3 | 5 | 6 | 7 | 11 | 15 => {
                    let h = if dd == 1 || dd == 5 { lr } else { nl };
                    self.fgc.set_foreground(self.colors[color]);
                    d.win().fill_rectangle(&self.fgc, x, y, m2, h);
                    self.fgc.set_foreground(self.colors[color + 3]);
                    d.win().fill_rectangle(&self.fgc, x + m2, y, m1, h);
                }
                4 | 8 | 9 | 10 | 12 | 13 | 14 => {
                    let w = if dd == 4 { lr } else { nl };
                    self.fgc.set_foreground(self.colors[color + 1]);
                    d.win().fill_rectangle(&self.fgc, x, y, w, m2);
                    self.fgc.set_foreground(self.colors[color + 2]);
                    d.win().fill_rectangle(&self.fgc, x, y + m2, w, m1);
                }
                _ => {}
            }

            // The mitred ends and corners.
            match dd {
                1 => self.tri(
                    d,
                    color + 2,
                    [(x, y + lr), (x + lr, y + lr), (x + m2, y + m2)],
                ),
                2 => self.tri(d, color + 1, [(x, y), (x + lr, y), (x + m2, y + m2)]),
                4 => self.tri(
                    d,
                    color + 3,
                    [(x + lr, y), (x + lr, y + lr), (x + m2, y + m2)],
                ),
                5 => {
                    self.tri(d, color + 1, [(x, y + m2), (x + m2, y + m2), (x, y)]);
                    self.quad(
                        d,
                        color + 2,
                        [(x, y + m2), (x + m2, y + m2), (x + lr, y + lr), (x, y + lr)],
                    );
                }
                6 => {
                    self.quad(
                        d,
                        color + 1,
                        [(x, y + m2), (x + m2, y + m2), (x + lr, y), (x, y)],
                    );
                    self.tri(d, color + 2, [(x, y + m2), (x + m2, y + m2), (x, y + lr)]);
                }
                7 => {
                    self.tri(d, color + 1, [(x, y + m2), (x + m2, y + m2), (x, y)]);
                    self.tri(d, color + 2, [(x, y + m2), (x + m2, y + m2), (x, y + lr)]);
                }
                8 => self.tri(d, color, [(x, y), (x, y + lr), (x + m2, y + m2)]),
                9 => {
                    self.quad(
                        d,
                        color,
                        [(x + m2, y), (x + m2, y + m2), (x, y + lr), (x, y)],
                    );
                    self.tri(d, color + 3, [(x + m2, y), (x + m2, y + m2), (x + lr, y)]);
                }
                10 => {
                    self.quad(
                        d,
                        color,
                        [(x + m2, y + nl), (x + m2, y + m2), (x, y), (x, y + nl)],
                    );
                    self.quad(
                        d,
                        color + 3,
                        [
                            (x + m2, y + nl),
                            (x + m2, y + m2),
                            (x + lr, y + lr),
                            (x + lr, y + nl),
                        ],
                    );
                }
                11 => {
                    self.quad(
                        d,
                        color + 1,
                        [(x + nl, y + m2), (x + m2, y + m2), (x + lr, y), (x + nl, y)],
                    );
                    self.quad(
                        d,
                        color + 2,
                        [
                            (x + nl, y + m2),
                            (x + m2, y + m2),
                            (x + lr, y + lr),
                            (x + nl, y + lr),
                        ],
                    );
                }
                13 => {
                    self.tri(d, color, [(x + m2, y), (x + m2, y + m2), (x, y)]);
                    self.tri(d, color + 3, [(x + m2, y), (x + m2, y + m2), (x + lr, y)]);
                }
                14 => {
                    self.quad(
                        d,
                        color,
                        [(x + m2, y + nl), (x + m2, y + m2), (x, y + lr), (x, y + nl)],
                    );
                    self.quad(
                        d,
                        color + 3,
                        [
                            (x + m2, y + nl),
                            (x + m2, y + m2),
                            (x + lr, y + lr),
                            (x + lr, y + nl),
                        ],
                    );
                }
                15 => {
                    self.tri(d, color + 1, [(x, y + m2), (x + m2, y + m2), (x, y)]);
                    self.tri(d, color + 2, [(x, y + m2), (x + m2, y + m2), (x, y + lr)]);
                    self.quad(
                        d,
                        color + 1,
                        [(x + nl, y + m2), (x + m2, y + m2), (x + lr, y), (x + nl, y)],
                    );
                    self.quad(
                        d,
                        color + 2,
                        [
                            (x + nl, y + m2),
                            (x + m2, y + m2),
                            (x + lr, y + lr),
                            (x + nl, y + lr),
                        ],
                    );
                }
                _ => {}
            }
            z += a;
        }
    }

    fn draw_lines(&mut self, d: &mut Dpy) {
        if self.bi == 1 {
            for a in 0..=self.oi.max(0) as usize {
                if a < self.fdol.len() {
                    self.fdol[a] = 0;
                }
            }
        }

        self.di = self.bi;
        while self.di < min(self.li + 1, self.bi + self.lpu) {
            let color = (self.dline[self.di as usize]
                .color
                .rem_euclid(self.ncolors.max(1))
                * self.shades) as usize;
            self.fgc
                .set_foreground(self.colors[color.min(self.colors.len() - 1)]);

            match self.d3d {
                D3D_NEON | D3D_BLOCK => {
                    // Every line of the object is redrawn a pixel further in
                    // and a shade along, which is what makes the bevel.
                    let obj = self.dline[self.di as usize].obj.max(0) as usize;
                    self.dline[self.di as usize].ndol = self.fdol[obj.min(self.fdol.len() - 1)];
                    if obj < self.fdol.len() {
                        self.fdol[obj] = self.di;
                    }
                    for sh in 0..self.lwid / 2 {
                        let c = if self.d3d == D3D_NEON {
                            color + sh as usize
                        } else {
                            color + (self.lwid / 2 - sh - 1).max(0) as usize
                        };
                        self.fgc
                            .set_foreground(self.colors[c.min(self.colors.len() - 1)]);
                        let mut di = self.di;
                        while di > 0 {
                            self.fill_line(d, di, sh);
                            di = self.dline[di as usize].ndol;
                        }
                    }
                }
                D3D_TILED => self.draw_tiled(d, color),
                _ => {
                    self.fill_line(d, self.di, 0);
                    if self.outline != 0 {
                        self.fill_outline(d, self.di);
                        let line = self.dline[self.di as usize];
                        let mut z = (line.y * self.gridx + line.x) as usize;
                        let a = if line.hv { 1 } else { self.gridx as usize };
                        for n in 0..=line.len {
                            if z >= self.grid.len() {
                                break;
                            }
                            let g = self.grid[z];
                            self.fill_outline(d, g.dhl);
                            self.fill_outline(d, g.dhr);
                            self.fill_outline(d, g.dvu);
                            self.fill_outline(d, g.dvd);
                            if line.hv {
                                if n != 0 {
                                    self.grid[z].dhr = self.di;
                                }
                                if n < line.len {
                                    self.grid[z].dhl = self.di;
                                }
                            } else {
                                if n != 0 {
                                    self.grid[z].dvd = self.di;
                                }
                                if n < line.len {
                                    self.grid[z].dvu = self.di;
                                }
                            }
                            z += a;
                        }
                    }
                }
            }
            self.di += 1;
        }

        if self.di > self.li {
            self.bi = 1;
            self.mode = Mode::Create;
        } else {
            self.bi += self.lpu;
        }
    }

    fn erase_lines(&mut self, d: &mut Dpy) {
        if self.ii == 0 {
            return;
        }
        self.di = self.bi;
        while self.di < min(self.eli + 1, self.bi + self.elpu) {
            let line = self.eline[self.di as usize];
            if line.hv {
                d.win().fill_rectangle(
                    &self.bgc,
                    line.x * self.elwid,
                    line.y * self.elwid,
                    (line.len + 1) * self.elwid,
                    self.elwid,
                );
            } else {
                d.win().fill_rectangle(
                    &self.bgc,
                    line.x * self.elwid,
                    line.y * self.elwid,
                    self.elwid,
                    (line.len + 1) * self.elwid,
                );
            }
            if self.di == self.eli {
                // Clear, just in case.
                let (w, h) = (self.width, self.height);
                d.win().fill_rectangle(&self.bgc, 0, 0, w, h);
            }
            self.di += 1;
        }
        if self.di > self.eli {
            self.bi = 1;
            self.mode = if self.resized {
                Mode::Create
            } else {
                Mode::Draw
            };
        } else {
            self.bi += self.elpu;
        }
    }
}

impl Screenhack for Abstractile {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // If the window is too small, do nothing, sorry!
        if self.width > 20 && self.height > 20 {
            match self.mode {
                Mode::Create => {
                    self.init_screen();
                    self.create_screen();
                }
                Mode::Erase => self.erase_lines(d),
                Mode::Draw => self.draw_lines(d),
            }
        }
        // Upstream subtracts the time the frame took; there is nothing to
        // measure here and the correction is small.
        if self.ii == 0 && self.mode == Mode::Create {
            0
        } else if self.mode == Mode::Create {
            (self.sleep * 1_000_000).max(0) as u32
        } else {
            ((5 - self.speed) * (2 - self.dialog) * 100_000 / self.lpu.max(1)).max(0) as u32
        }
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        if (width * height) as usize > self.narray * 4 {
            self.resized = true;
        }
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.mode = Mode::Create;
            return true;
        }
        false
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(Abstractile::new(d))
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*sleep: 3",
    "*speed: 3",
    "*tile: random",
    "*ignoreRotation: True",
];

const TILES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random tile layout",
    },
    SelectItem {
        value: "flat",
        label: "Flat tiles",
    },
    SelectItem {
        value: "thin",
        label: "Thin tiles",
    },
    SelectItem {
        value: "outline",
        label: "Outline tiles",
    },
    SelectItem {
        value: "block",
        label: "Block tiles",
    },
    SelectItem {
        value: "neon",
        label: "Neon tiles",
    },
    SelectItem {
        value: "tiled",
        label: "Tiled tiles",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("speed", "Speed", 0.0, 5.0, 1.0, 0, "3"),
    Opt::slider("sleep", "Linger", 0.0, 60.0, 1.0, 0, "3"),
    Opt::select("tile", "Tile style", TILES, "random"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "abstractile",
    label: "Abstractile",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Steve Sundstrom",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=NgSetBY6VP4"),
        blurb: "Mosaic patterns of interlocking tiles.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
