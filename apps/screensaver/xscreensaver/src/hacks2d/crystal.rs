//! Port of `hacks/crystal.c`.
//!
//! ```text
//! crystal --- polygons moving according to plane group rules
//!
//! Copyright (c) 1997 by Jouk Jansen <joukj@crys.chem.uva.nl>
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
//! The author should like to be notified if changes have been made to the
//! routine.  Response will only be guaranteed when a VMS version of the
//! program is available.
//!
//! A moving polygon-mode. The polygons obey 2D-planegroup symmetry.
//!
//! The groupings of the cells fall in 3 categories:
//!   oblique groups 1 and 2 where the angle gamma ranges from 60 to 120 degrees
//!   square groups 3 through 11 where the angle gamma is 90 degrees
//!   hexagonal groups 12 through 17 where the angle gamma is 120 degrees
//! ```
//!
//! A few polygons wander about inside one cell of a lattice, and everything
//! else on screen is those same polygons seen through the symmetry of one of
//! the seventeen wallpaper groups. Every group is a complete list of the ways a
//! pattern can repeat across a plane, so this is the whole catalogue: pick one,
//! and the rules say where the copies go.
//!
//! The rules are three tables. One holds a run of matrices, each a two by two
//! integer matrix and a pair of half-cell translations, and a second says which
//! run belongs to which group. Applying one to a polygon rotates or reflects it
//! and shifts it by half a cell; the result is wrapped back into the cell it
//! came from. Two more tables say whether the group also has a centre of
//! inversion, which adds a copy of everything reflected through the far corner,
//! and whether its cell is primitive, which if it is not adds another copy
//! offset by half a cell in both directions. Then the whole cell is tiled
//! across the screen. So four polygons in a group with nine operations, drawn
//! on a three by three lattice, is well over a hundred polygons a frame, and
//! moving one moves all of its images at once.
//!
//! The cell is not always square. Oblique groups get a random angle between
//! sixty and a hundred and twenty degrees, hexagonal groups get a hundred and
//! twenty, and the drawing shears every coordinate on the way out and back to
//! account for it, which is what makes those groups look like they were drawn
//! on a diamond grid. Since a sixty degree cell is a hundred and twenty degree
//! cell seen upside down, the screen is flipped vertically at random rather
//! than the angle being extended.
//!
//! Nothing is ever erased. Everything is drawn exclusive-or, so a polygon drawn
//! a second time in the same place removes itself, which is how the previous
//! frame is cleaned up, and why polygons that overlap show the colour of the
//! difference rather than of either one.
//!
//! Three things here differ from the C, and the first is upstream's own doing:
//! its driver forces full-random mode on every hack, and in that mode crystal
//! ignores the two switches for the unit cell and its grid and flips a coin
//! for each instead, so those two knobs in the config XML cannot do anything.
//! Here the coin is flipped only for a knob the panel has not set.
//!
//! Colour cycling is gone, and it is gone upstream too on any display of the
//! last twenty years: the hack asks whether the colormap has writable cells
//! before it will cycle, and a true-colour display says no.
//!
//! Lastly the shear is computed once per restart rather than by calling `sin`
//! and `cos` on the same constant angle inside the loop over every point of
//! every copy of every polygon, which at these polygon counts is millions of
//! calls a frame.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{
    Pixel, XColor, make_random_colormap, make_smooth_colormap, make_uniform_colormap,
};
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, GXFunc, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

const PI_RAD: f64 = std::f64::consts::PI / 180.0;
/// The smallest cell the random sizing will pick, screen permitting.
const MIN_CELL: i32 = 200;
const DEF_NUM_ATOM: i32 = 10;
const DEF_SIZ_ATOM: i32 = 10;

/// Whether each plane group has a centre of inversion.
const CENTRO: [bool; 17] = [
    false, true, false, false, false, true, true, true, true, true, true, true, false, false,
    false, true, true,
];

/// Whether each plane group's cell is primitive. A centred cell carries a
/// second copy of everything, offset by half the cell in both directions.
const PRIMITIVE: [bool; 17] = [
    true, true, true, true, false, true, true, true, false, true, true, true, true, true, true,
    true, true,
];

/// Where each plane group's symmetry operations sit in `OPERATION`, as an
/// exclusive end and a start.
const NUMOPS: [i32; 34] = [
    1, 0, 1, 0, 9, 7, 2, 0, 9, 7, 9, 7, 4, 2, 5, 3, 9, 7, 8, 6, 10, 6, 8, 4, 16, 13, 19, 13, 16,
    10, 19, 13, 19, 13,
];

/// The symmetry operations: a two by two matrix and two half-cell shifts each.
const OPERATION: [i32; 114] = [
    1, 0, 0, 1, 0, 0, //
    -1, 0, 0, 1, 0, 1, //
    -1, 0, 0, 1, 1, 0, //
    1, 0, 0, 1, 0, 0, //
    -1, 0, 0, 1, 1, 1, //
    1, 0, 0, 1, 1, 1, //
    0, -1, 1, 0, 0, 0, //
    1, 0, 0, 1, 0, 0, //
    -1, 0, 0, 1, 0, 0, //
    0, 1, 1, 0, 0, 0, //
    -1, 0, -1, 1, 0, 0, //
    1, -1, 0, -1, 0, 0, //
    0, 1, 1, 0, 0, 0, //
    0, -1, 1, -1, 0, 0, //
    -1, 1, -1, 0, 0, 0, //
    1, 0, 0, 1, 0, 0, //
    0, -1, -1, 0, 0, 0, //
    -1, 1, 0, 1, 0, 0, //
    1, 0, 1, -1, 0, 0, //
];

/// The shear the cell angle imposes on every coordinate.
#[derive(Clone, Copy, Default)]
struct Shear {
    sin: f64,
    cos: f64,
}

impl Shear {
    fn new(gamma: f32) -> Self {
        let t = (gamma as f64 - 90.0) * PI_RAD;
        Self {
            sin: t.sin(),
            cos: t.cos(),
        }
    }
}

/// `trans_coor`: out of the sheared frame the polygon was built in.
fn trans_coor(xyp: &[XPoint], out: &mut [XPoint], num_points: usize, s: Shear) {
    for i in 0..=num_points {
        out[i].x = xyp[i].x + (xyp[i].y as f64 * s.sin) as i32;
        out[i].y = (xyp[i].y as f64 / s.cos) as i32;
    }
}

/// `trans_coor_back`: into screen coordinates, with the vertical flip that
/// stands in for the cell angles below ninety degrees.
#[allow(clippy::too_many_arguments)]
fn trans_coor_back(
    xyp: &[XPoint],
    out: &mut [XPoint],
    num_points: usize,
    s: Shear,
    offset_w: i32,
    offset_h: i32,
    win_height: i32,
    invert: bool,
) {
    for i in 0..=num_points {
        out[i].y = (xyp[i].y as f64 * s.cos) as i32 + offset_h;
        out[i].x = xyp[i].x - (xyp[i].y as f64 * s.sin) as i32 + offset_w;
        if invert {
            out[i].y = win_height - out[i].y;
        }
    }
}

#[derive(Clone, Copy)]
struct Atom {
    /// An index into the hack's own colormap, or a pixel value when there is
    /// no colormap, exactly as upstream overloads the same field.
    colour: u32,
    x0: i32,
    y0: i32,
    velocity: [i32; 2],
    angle: f32,
    velocity_a: f32,
    num_point: usize,
    at_type: i32,
    size_at: i32,
    xy: [XPoint; 5],
}

impl Default for Atom {
    fn default() -> Self {
        Self {
            colour: 0,
            x0: 0,
            y0: 0,
            velocity: [0; 2],
            angle: 0.0,
            velocity_a: 0.0,
            num_point: 0,
            at_type: 0,
            size_at: 0,
            xy: [XPoint { x: 0, y: 0 }; 5],
        }
    }
}

/// `crystal_setupatom`: build the polygon for one atom at its current angle.
fn setup_atom(atom: &mut Atom, s: Shear) {
    let mut xy = [XPoint { x: 0, y: 0 }; 5];
    let y0 = (atom.y0 as f64 * s.cos) as i32;
    let x0 = atom.x0 - (atom.y0 as f64 * s.sin) as i32;
    let (sa, ca) = ((atom.angle as f64).sin(), (atom.angle as f64).cos());
    let sz = atom.size_at as f64;

    match atom.at_type {
        // Rectangles.
        0 => {
            let (long_c, long_s) = ((2.0 * sz * ca) as i32, (2.0 * sz * sa) as i32);
            let (short_c, short_s) = ((sz * ca) as i32, (sz * sa) as i32);
            xy[0] = XPoint {
                x: x0 + long_c + short_s,
                y: y0 + short_c - long_s,
            };
            xy[1] = XPoint {
                x: x0 + long_c - short_s,
                y: y0 - short_c - long_s,
            };
            xy[2] = XPoint {
                x: x0 - long_c - short_s,
                y: y0 - short_c + long_s,
            };
            xy[3] = XPoint {
                x: x0 - long_c + short_s,
                y: y0 + short_c + long_s,
            };
            xy[4] = xy[0];
            trans_coor(&xy, &mut atom.xy, 4, s);
        }
        // Squares.
        1 => {
            let (c, sn) = ((1.5 * sz * ca) as i32, (1.5 * sz * sa) as i32);
            xy[0] = XPoint {
                x: x0 + c + sn,
                y: y0 + c - sn,
            };
            xy[1] = XPoint {
                x: x0 + c - sn,
                y: y0 - c - sn,
            };
            xy[2] = XPoint {
                x: x0 - c - sn,
                y: y0 - c + sn,
            };
            xy[3] = XPoint {
                x: x0 - c + sn,
                y: y0 + c + sn,
            };
            xy[4] = xy[0];
            trans_coor(&xy, &mut atom.xy, 4, s);
        }
        // Triangles.
        _ => {
            let (c, sn) = ((1.5 * sz * ca) as i32, (1.5 * sz * sa) as i32);
            xy[0] = XPoint {
                x: x0 + sn,
                y: y0 + c,
            };
            xy[1] = XPoint {
                x: x0 + c - sn,
                y: y0 - c - sn,
            };
            xy[2] = XPoint {
                x: x0 - c - sn,
                y: y0 - c + sn,
            };
            xy[3] = xy[0];
            trans_coor(&xy, &mut atom.xy, 3, s);
        }
    }
}

struct Crystal {
    mi: ModeInfo,
    painted: bool,
    win_width: i32,
    win_height: i32,
    num_atom: usize,
    planegroup: usize,
    a: i32,
    b: i32,
    offset_w: i32,
    offset_h: i32,
    nx: i32,
    ny: i32,
    gamma: f32,
    shear: Shear,
    atoms: Vec<Atom>,
    unit_cell: bool,
    grid_cell: bool,
    /// The hack's own colormap, which is not the one `ModeInfo` built.
    colors: Vec<XColor>,
    ncolors: usize,
    mono_p: bool,
    /// True when the atoms index [`Crystal::colors`] rather than carrying a
    /// pixel value of their own.
    install: bool,
    invert: bool,
    grid_pixel: Pixel,
    /// Which cell of the lattice gets drawn when only one is wanted.
    inx: i32,
    iny: i32,
}

impl Crystal {
    fn new(d: &mut Dpy) -> Self {
        let mi = ModeInfo::new(d, ColorScheme::Random);
        let mut st = Self {
            win_width: mi.width,
            win_height: mi.height,
            mi,
            painted: false,
            num_atom: 0,
            planegroup: 0,
            a: 0,
            b: 0,
            offset_w: 0,
            offset_h: 0,
            nx: 1,
            ny: 1,
            gamma: 90.0,
            shear: Shear::default(),
            atoms: Vec::new(),
            unit_cell: false,
            grid_cell: false,
            colors: Vec::new(),
            ncolors: 0,
            mono_p: false,
            install: false,
            invert: false,
            grid_pixel: 0,
            inx: 0,
            iny: 0,
        };
        st.restart(d);
        st
    }

    /// `init_crystal`: pick a plane group, a cell, and a set of atoms.
    fn restart(&mut self, d: &mut Dpy) {
        self.mi.clear_window(d);
        self.painted = false;

        // Upstream's driver forces full-random mode, in which these two are
        // coin flips and the switches for them do nothing. Defer to the panel
        // when it has actually set one.
        self.unit_cell = if d.res.is_overridden("cell") {
            d.res.bool("cell")
        } else {
            lrand() & 1 != 0
        };
        if self.unit_cell {
            self.grid_cell = if d.res.is_overridden("grid") {
                d.res.bool("grid")
            } else {
                lrand() & 1 != 0
            };
        }
        let centre = d.res.bool("centre");
        let maxsize = d.res.bool("maxsize");

        self.win_width = self.mi.width;
        self.win_height = self.mi.height;
        let cell_min = (self.win_width / 2 + 1)
            .min(MIN_CELL)
            .min(self.win_height / 2 + 1);

        self.planegroup = nrand(17) as usize;
        self.invert = nrand(2) != 0;
        self.gamma = if self.planegroup > 11 {
            120.0
        } else if self.planegroup < 2 {
            60.0 + nrand(60) as f32
        } else {
            90.0
        };

        // How many copies of one atom the symmetry will produce, which is what
        // the requested count is divided between.
        let mut neqv = NUMOPS[2 * self.planegroup] - NUMOPS[2 * self.planegroup + 1];
        if CENTRO[self.planegroup] {
            neqv *= 2;
        }
        if !PRIMITIVE[self.planegroup] {
            neqv *= 2;
        }

        let nx = d.res.int("nx");
        let ny = d.res.int("ny");
        self.nx = match nx.cmp(&0) {
            std::cmp::Ordering::Greater => nx,
            std::cmp::Ordering::Less => nrand(-nx) + 1,
            std::cmp::Ordering::Equal => 1,
        };
        self.ny = if self.planegroup > 8 {
            self.nx
        } else {
            match ny.cmp(&0) {
                std::cmp::Ordering::Greater => ny,
                std::cmp::Ordering::Less => nrand(-ny) + 1,
                std::cmp::Ordering::Equal => 1,
            }
        };
        neqv *= self.nx * self.ny;

        let count = self.mi.count;
        let (mut num_atom, max_atoms) = if count == 0 {
            (DEF_NUM_ATOM, DEF_NUM_ATOM)
        } else if count < 0 {
            (nrand(-count) + 1, -count)
        } else {
            (count, count)
        };
        if neqv > 1 {
            num_atom = num_atom / neqv + 1;
        }
        self.num_atom = num_atom.max(0) as usize;
        self.atoms = vec![Atom::default(); self.num_atom.max(max_atoms.max(0) as usize)];

        if maxsize {
            // One cell, as large as the screen will take.
            if self.planegroup < 13 {
                self.gamma = 90.0;
                self.offset_w = 0;
                self.offset_h = 0;
                if self.planegroup < 10 {
                    self.b = self.win_height;
                    self.a = self.win_width;
                } else {
                    self.b = self.win_height.min(self.win_width);
                    self.a = self.b;
                }
            } else {
                self.gamma = 120.0;
                self.a = (self.win_width as f64 * 2.0 / 3.0) as i32;
                self.b = self.a;
                let s = Shear::new(self.gamma);
                self.offset_h = (self.b as f64 * 0.25 * s.cos) as i32;
                self.offset_w = (self.b as f64 * 0.5) as i32;
            }
        } else {
            let s = Shear::new(self.gamma);
            let mut max_repeat = 10;
            self.offset_w = -1;
            while max_repeat > 0
                && (self.offset_w < 4
                    || ((self.offset_w as f64 - self.b as f64 * s.sin) as i32) < 4)
            {
                max_repeat -= 1;
                self.b = nrand((self.win_height as f64 / s.cos) as i32 - cell_min) + cell_min;
                self.a = if self.planegroup > 8 {
                    self.b
                } else {
                    nrand(self.win_width - cell_min) + cell_min
                };
                self.offset_w = ((self.win_width as f64 - (self.a as f64 - self.b as f64 * s.sin))
                    / 2.0) as i32;
            }
            self.offset_h = ((self.win_height as f64 - self.b as f64 * s.cos) / 2.0) as i32;
            if !centre {
                let n2 = 2 * self.offset_h;
                if self.offset_h > 0 {
                    self.offset_h = nrand(n2);
                }
                self.offset_w =
                    (self.win_width as f64 - self.a as f64 - self.b as f64 * s.sin.abs()) as i32;
                if self.gamma > 90.0 {
                    self.offset_w = if self.offset_w > 0 {
                        nrand(self.offset_w) + (self.b as f64 * s.sin) as i32
                    } else {
                        (self.b as f64 * s.sin) as i32
                    };
                } else if self.offset_w > 0 {
                    self.offset_w = nrand(self.offset_w);
                } else {
                    self.offset_w = 0;
                }
            }
        }
        self.shear = Shear::new(self.gamma);

        let mut size_atom = (self.a as f32 / 40.0) as i32 + 1;
        size_atom = size_atom.min((self.b as f32 / 40.0) as i32 + 1);
        if self.mi.size < size_atom {
            size_atom = if self.mi.size < -size_atom {
                -size_atom
            } else {
                self.mi.size
            };
        }
        self.a /= self.nx;
        self.b /= self.ny;

        // The hack builds its own colormap rather than using the one xlockmore
        // handed it, and picks how to build it at random.
        self.install = self.mi.npixels() > 2;
        if self.install {
            self.ncolors = self.mi.npixels().max(2) as usize;
            self.mono_p = self.ncolors <= 2;
            self.colors = if self.mono_p {
                Vec::new()
            } else if lrand().is_multiple_of(10) {
                make_random_colormap(self.ncolors, true)
            } else if lrand().is_multiple_of(2) {
                make_uniform_colormap(self.ncolors)
            } else {
                make_smooth_colormap(self.ncolors)
            };
        }

        let shear = self.shear;
        let (a, b, ncolors, install) = (self.a, self.b, self.ncolors, self.install);
        let npixels = self.mi.npixels();
        for i in 0..self.num_atom {
            let colour = if install {
                if ncolors > 2 {
                    (nrand(ncolors as i32 - 2) + 2) as u32
                } else {
                    1
                }
            } else if npixels > 2 {
                self.mi.pixel(nrand(npixels) as usize)
            } else {
                1
            };
            let atom = &mut self.atoms[i];
            atom.colour = colour;
            atom.x0 = nrand(a);
            atom.y0 = nrand(b);
            atom.velocity[0] = nrand(7) - 3;
            atom.velocity[1] = nrand(7) - 3;
            atom.velocity_a = ((nrand(7) - 3) as f64 * PI_RAD) as f32;
            atom.angle = (nrand(90) as f64 * PI_RAD) as f32;
            atom.at_type = nrand(3);
            atom.size_at = match size_atom {
                0 => DEF_SIZ_ATOM,
                n if n > 0 => n,
                n => nrand(-n) + 1,
            };
            atom.size_at += 1;
            atom.num_point = if atom.at_type == 2 { 3 } else { 4 };
            setup_atom(atom, shear);
        }

        self.grid_pixel = if npixels > 2 {
            self.mi.pixel(nrand(npixels) as usize)
        } else {
            self.mi.white
        };
        self.inx = nrand(self.nx);
        self.iny = nrand(self.ny);
    }

    /// The vertical flip that turns a hundred and twenty degree cell into a
    /// sixty degree one.
    fn flip(&self, y: i32) -> i32 {
        if self.invert { self.win_height - y } else { y }
    }

    /// `crystal_drawatom`: one atom, and every copy of it the plane group and
    /// the lattice call for.
    fn draw_atom(&self, d: &mut Dpy, index: usize) {
        let atom = &self.atoms[index];
        let (a, b) = (self.a, self.b);
        let np = atom.num_point;
        let mut xy = [XPoint { x: 0, y: 0 }; 5];

        for j in NUMOPS[2 * self.planegroup + 1]..NUMOPS[2 * self.planegroup] {
            let o = &OPERATION[j as usize * 6..j as usize * 6 + 6];

            // Where this operation puts the atom's origin, and how far the
            // result has to be shifted to land back inside the cell.
            let mut xtrans =
                o[0] * atom.x0 + o[1] * atom.y0 + (o[4] as f64 * a as f64 / 2.0) as i32;
            let mut ytrans =
                o[2] * atom.x0 + o[3] * atom.y0 + (o[5] as f64 * b as f64 / 2.0) as i32;
            if xtrans < 0 {
                xtrans = if xtrans < -a { 2 * a } else { a };
            } else if xtrans >= a {
                xtrans = -a;
            } else {
                xtrans = 0;
            }
            if ytrans < 0 {
                ytrans = b;
            } else if ytrans >= b {
                ytrans = -b;
            } else {
                ytrans = 0;
            }

            for (out, src) in xy.iter_mut().zip(atom.xy.iter()).take(np) {
                out.x =
                    o[0] * src.x + o[1] * src.y + (o[4] as f64 * a as f64 / 2.0) as i32 + xtrans;
                out.y =
                    o[2] * src.x + o[3] * src.y + (o[5] as f64 * b as f64 / 2.0) as i32 + ytrans;
            }
            xy[np] = xy[0];
            self.tile(d, &xy, np);

            if CENTRO[self.planegroup] {
                // A centre of inversion: the same shape through the far corner.
                for p in xy.iter_mut().take(np + 1) {
                    p.x = a - p.x;
                    p.y = b - p.y;
                }
                self.tile(d, &xy, np);
            }

            if !PRIMITIVE[self.planegroup] {
                // A centred cell: everything again, half a cell along both axes.
                let xt = if xy[np].x >= (a as f64 / 2.0) as i32 {
                    (-a as f64 / 2.0) as i32
                } else {
                    (a as f64 / 2.0) as i32
                };
                let yt = if xy[np].y >= (b as f64 / 2.0) as i32 {
                    (-b as f64 / 2.0) as i32
                } else {
                    (b as f64 / 2.0) as i32
                };
                for p in xy.iter_mut().take(np + 1) {
                    p.x += xt;
                    p.y += yt;
                }
                self.tile(d, &xy, np);

                if CENTRO[self.planegroup] {
                    let mut xy1 = [XPoint { x: 0, y: 0 }; 5];
                    for k in 0..=np {
                        xy1[k].x = a - xy[k].x;
                        xy1[k].y = b - xy[k].y;
                    }
                    self.tile(d, &xy1, np);
                }
            }
        }
    }

    /// Draw one polygon once per cell of the lattice.
    fn tile(&self, d: &mut Dpy, xy: &[XPoint; 5], np: usize) {
        let mut xy_1 = [XPoint { x: 0, y: 0 }; 5];
        let mut new_xy = [XPoint { x: 0, y: 0 }; 5];
        for l in 0..self.nx {
            for m in 0..self.ny {
                for k in 0..=np {
                    xy_1[k].x = xy[k].x + l * self.a;
                    xy_1[k].y = xy[k].y + m * self.b;
                }
                trans_coor_back(
                    &xy_1,
                    &mut new_xy,
                    np,
                    self.shear,
                    self.offset_w,
                    self.offset_h,
                    self.win_height,
                    self.invert,
                );
                d.win().fill_polygon(&self.mi.gc, &new_xy[..np]);
            }
        }
    }

    /// The unit cell, drawn once per restart: either the whole lattice or one
    /// randomly chosen cell of it.
    fn draw_cell(&mut self, d: &mut Dpy) {
        let s = self.shear;
        let (a, b, ow, oh) = (self.a, self.b, self.offset_w, self.offset_h);
        self.mi.gc.set_foreground(self.grid_pixel);

        if self.grid_cell {
            let y = self.flip(oh);
            d.win().draw_line(&self.mi.gc, ow, y, ow + self.nx * a, y);

            let y2 = self.flip(((self.ny * b) as f64 * s.cos) as i32 + oh);
            d.win().draw_line(
                &self.mi.gc,
                ow,
                self.flip(oh),
                (ow as f64 - (self.ny * b) as f64 * s.sin) as i32,
                y2,
            );

            let inx = self.nx;
            for iny in 1..=self.ny {
                let y = self.flip(((iny * b) as f64 * s.cos) as i32 + oh);
                d.win().draw_line(
                    &self.mi.gc,
                    ow + inx * a - ((iny * b) as f64 * s.sin) as i32,
                    y,
                    (ow as f64 - (iny * b) as f64 * s.sin) as i32,
                    y,
                );
            }
            let iny = self.ny;
            for inx in 1..=self.nx {
                let y1 = self.flip(((iny * b) as f64 * s.cos) as i32 + oh);
                let y2 = self.flip(oh);
                d.win().draw_line(
                    &self.mi.gc,
                    ow + inx * a - ((iny * b) as f64 * s.sin) as i32,
                    y1,
                    ow + inx * a,
                    y2,
                );
            }
        } else {
            let (inx, iny) = (self.inx, self.iny);
            let y1 = self.flip(((iny * b) as f64 * s.cos) as i32 + oh);
            let y2 = self.flip((((iny + 1) * b) as f64 * s.cos) as i32 + oh);
            let x00 = ow + inx * a - ((iny * b) as f64 * s.sin) as i32;
            let x10 = ow + (inx + 1) * a - ((iny * b) as f64 * s.sin) as i32;
            let x01 = ow + inx * a - (((iny + 1) * b) as f64 * s.sin) as i32;
            let x11 = ow + (inx + 1) * a - (((iny + 1) * b) as f64 * s.sin) as i32;
            d.win().draw_line(&self.mi.gc, x00, y1, x10, y1);
            d.win().draw_line(&self.mi.gc, x00, y1, x01, y2);
            d.win().draw_line(&self.mi.gc, x10, y1, x11, y2);
            d.win().draw_line(&self.mi.gc, x01, y2, x11, y2);
        }
    }
}

impl Screenhack for Crystal {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.unit_cell && !self.painted {
            self.draw_cell(d);
        }

        // Everything from here is exclusive-or, which is how the last frame
        // gets rubbed out: each atom is drawn where it was before being moved.
        self.mi.gc.set_function(GXFunc::Xor);

        let shear = self.shear;
        let (a, b) = (self.a, self.b);
        for i in 0..self.num_atom {
            let pixel = if self.install && !self.colors.is_empty() {
                self.colors[(self.atoms[i].colour as usize).min(self.colors.len() - 1)].pixel
            } else {
                self.atoms[i].colour
            };
            self.mi.gc.set_foreground(pixel);

            if self.painted {
                self.draw_atom(d, i);
            }

            let atom = &mut self.atoms[i];
            atom.velocity[0] = (atom.velocity[0] + nrand(3) - 1).clamp(-20, 20);
            atom.velocity[1] = (atom.velocity[1] + nrand(3) - 1).clamp(-20, 20);
            atom.x0 += atom.velocity[0];
            if atom.x0 < 0 {
                atom.x0 += a;
            } else if atom.x0 >= a {
                atom.x0 -= a;
            }
            atom.y0 += atom.velocity[1];
            if atom.y0 < 0 {
                atom.y0 += b;
            } else if atom.y0 >= b {
                atom.y0 -= b;
            }
            atom.velocity_a += (nrand(1001) as f32 - 500.0) / 2000.0;
            atom.angle += atom.velocity_a;
            setup_atom(atom, shear);

            self.draw_atom(d, i);
        }

        self.mi.gc.set_function(GXFunc::Copy);
        self.painted = true;
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.restart(d);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(Crystal::new(d))
}

const DEFAULTS: &[&str] = &[
    "*delay: 60000",
    "*count: -500",
    "*cycles: 200",
    "*size: -15",
    "*ncolors: 100",
    "*fpsSolid: True",
    "*ignoreRotation: True",
    "*nx: -3",
    "*ny: -3",
    "*centre: False",
    "*maxsize: False",
    "*cell: True",
    "*grid: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "60000").inverted(),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "100"),
    Opt::spin("count", "Number of crystals", -5000.0, 5000.0, "-500"),
    Opt::spin("nx", "Horizontal symmetries", -10.0, 10.0, "-3"),
    Opt::spin("ny", "Vertical symmetries", -10.0, 10.0, "-3"),
    Opt::boolean("grid", "Draw grid", "false"),
    Opt::boolean("cell", "Draw cell", "true"),
    Opt::boolean("centre", "Center on screen", "false"),
    Opt::boolean("maxsize", "Fill the screen with one cell", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "crystal",
    label: "Crystal",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jouk Jansen",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=M27wWKGXIvw"),
        blurb: "Moving polygons, similar to a kaleidoscope.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
