//! Port of `hacks/glx/crackberg.c`.
//!
//! ```text
//! crackberg; Matus Telgarsky [ catachresis@cmu.edu ] 2005
//! ```
//!
//! A flight over an endless fractal mountain range, generated a triangle at a
//! time as the camera comes at it and taken apart again once it is behind.
//!
//! The ground is a lattice of unit triangles, `trile`s, half of them pointing
//! up and half down. Each one is subdivided by midpoint displacement, so its
//! height field is random but its three edges are exactly the edges of its
//! neighbours: a new trile copies whichever edges already exist and only
//! invents the ones that do not. That is the whole trick, and it is why the
//! range can go on forever without ever being stored.
//!
//! What is on screen is worked out on the ground plane rather than by clipping:
//! the view frustum is flattened to a quadrilateral, the quadrilateral is
//! scanned row by row, and every trile a row touches is marked visible. One
//! that stops being marked starts dying, and how a trile arrives and leaves is
//! its `morph`: it grows out of the floor, falls from the sky, or spins up out
//! of nothing.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};
use std::collections::BTreeMap;

/// The circumradius of a unit triangle, three quarters of the root of seven
/// over four: how far outside the frustum a trile's centre can be and still
/// have a corner inside it.
const M_RAD7_4: f64 = 0.661_437_827_766_148;
const M_SQRT3_2: f64 = 0.866_025_403_784_439;
const M_PI_180: f64 = 0.017_453_292_519_943_3;
const M_180_PI: f64 = 57.295_779_513_082_3;
const MSPEED_SCALE: f64 = 1.1;
const MAX_ZDELTA: f64 = 0.35;

/// The midpoint displacement itself: the mean of two heights, pushed about by
/// an amount that halves with every subdivision.
fn displace(h: f64, d: u32) -> f64 {
    h + (frand(1.0) - 0.5) * 2.0 * MAX_ZDELTA / f64::from(1u32 << d)
}

fn mean(x: f64, y: f64) -> f64 {
    (x + y) / 2.0
}

fn ave3(a: f64, b: f64, c: f64) -> f64 {
    (a + b + c) / 3.0
}

/// Fill in an edge from its two ends, halving the displacement each round.
fn subdivide(edge: &mut [f64], nsubdivs: u32) {
    let mut i = (1usize << nsubdivs) >> 1;
    let mut k = 1;
    while i > 0 {
        let mut j = i;
        while j < edge.len() {
            edge[j] = displace(mean(edge[j - i], edge[j + i]), k);
            j += i * 2;
        }
        i >>= 1;
        k += 1;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    New,
    Init,
    Stable,
    Dying,
    Delete,
}

/// How a trile arrives and how it leaves. Each carries one number, which is
/// how far through its arrival it is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Morph {
    /// Rises out of the floor.
    Grow,
    /// Drops out of the sky.
    Fall,
    /// Spins up out of its own centre.
    Yeast,
    /// Is simply there.
    Identity,
}

/// One triangle of the lattice. Only its three edges are kept: everything
/// inside is generated once, baked into a display list, and thrown away.
struct Trile {
    x: i32,
    y: i32,
    state: State,
    visible: bool,
    l: Vec<f64>,
    r: Vec<f64>,
    v: Vec<f64>,
    list: u32,
    morph: Morph,
    data: f64,
}

impl Trile {
    fn morph_init(&mut self) {
        match self.morph {
            // Not zero: a flat trile makes a mess of the normals.
            Morph::Grow | Morph::Yeast => self.data = 0.02,
            Morph::Fall => self.data = 0.0,
            Morph::Identity => self.state = State::Stable,
        }
    }

    fn morph_init_iter(&mut self, elapsed: f64) {
        self.data += elapsed;
        let done = match self.morph {
            Morph::Grow | Morph::Yeast => self.data >= 1.0,
            Morph::Fall => self.data >= 0.5,
            Morph::Identity => return,
        };
        if done {
            self.state = State::Stable;
        }
    }

    fn morph_dying_iter(&mut self, elapsed: f64) {
        if self.morph == Morph::Identity {
            self.state = State::Delete;
            return;
        }
        self.data -= elapsed;
        let gone = match self.morph {
            Morph::Fall => self.data <= 0.0,
            _ => self.data <= 0.02,
        };
        if gone {
            self.state = State::Delete;
        }
    }
}

/// Which way the ground is coloured. Every scheme is two ramps: one for what
/// is above the waterline and one for what is below it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Plain,
    Ice,
    Magma,
    Vomit,
}

impl Scheme {
    /// What to clear the screen to. `Vomit` picks its own, at random.
    fn bg(self) -> [f32; 4] {
        match self {
            Scheme::Plain | Scheme::Ice => [0.0, 0.0, 0.0, 1.0],
            // No error!
            Scheme::Magma | Scheme::Vomit => [0.3, 0.3, 0.0, 1.0],
        }
    }
}

/// The keys and the mouse both feed the same set of bits.
const MOTION_MANUAL: i32 = 1;
const MOTION_LROT: i32 = 2;
const MOTION_RROT: i32 = 4;
const MOTION_FORW: i32 = 8;
const MOTION_BACK: i32 = 16;
const MOTION_DEC: i32 = 32;
const MOTION_INC: i32 = 64;
const MOTION_LEFT: i32 = 128;
const MOTION_RIGHT: i32 = 256;
const MOTION_AUTO: i32 = 0;

/// One edge of the flattened frustum, as a line to be crossed at some height.
struct Ls {
    min: f64,
    max: f64,
    start: f64,
    dx: f64,
}

/// One vertex of a row, ready to be handed out as a triangle corner.
#[derive(Clone, Copy, Default)]
struct Vert {
    pos: [f32; 3],
    color: [f32; 3],
    normal: [f32; 3],
}

struct Crackberg {
    triles: BTreeMap<(i32, i32), Trile>,
    /// Display list names of triles that have died, to be handed out again.
    free_lists: Vec<u32>,

    x: f64,
    y: f64,
    z: f64,
    yaw: f64,
    dx: f64,
    dy: f64,
    dyaw: f64,
    elapsed: f64,
    prev_frame: f64,

    motion_state: i32,
    /// What the keyboard asked for this frame. The runtime has no key-release
    /// event, so a key acts for one frame and the browser's auto-repeat does
    /// the holding down.
    keys: i32,
    mspeed: f64,

    fovy: f64,
    aspect: f64,
    z_near: f64,
    z_far: f64,

    scheme: Scheme,
    /// `Vomit`'s two ramps: solid from and to, then fluid from and to.
    vomit: [f64; 12],

    /// Points along one edge of a trile, `1 + 2^nsubdivs`.
    epoints: usize,
    /// The heights of the trile being built, and its normals. Scratch: only
    /// the edges survive into the trile itself.
    heights: Vec<f64>,
    norms: Vec<f64>,

    draw_elapsed: f64,
    dx0: f64,

    button_down: bool,
    mouse: (i32, i32),
    /// After a mouse-up, do not go back into auto-motion for a second, so that
    /// repeated click-and-drag gestures do not fight with it.
    paused: f64,

    nsubdivs: u32,
    crack: bool,
    boring: bool,
    do_water: bool,
    flat: bool,
    lit: bool,
    visibility: f64,
    letterbox: bool,
    wire: bool,
}

impl Crackberg {
    /// Where `(x, y)` of the trile being built lives in `heights`. A trile is
    /// a triangle, so its rows get shorter as they go up.
    fn tindex(&self, x: usize, y: usize) -> usize {
        self.epoints * y - y.saturating_sub(1) * y / 2 + x
    }

    fn tcoord(&self, x: usize, y: usize) -> f64 {
        self.heights[self.tindex(x, y)]
    }

    fn set_tcoord(&mut self, x: usize, y: usize, v: f64) {
        let i = self.tindex(x, y);
        self.heights[i] = v;
    }

    /// Where the flat normal of face `w` of cell `(x, y)` lives. Only defined
    /// for `x >= 1`.
    fn findex(&self, x: usize, y: usize, w: usize) -> usize {
        let e = self.epoints as isize;
        let (x, y, w) = (x as isize, y as isize, w as isize);
        (3 * (2 * y * e - (y + 1) * (y + 1) + 1 + 2 * (x - 1) + w)) as usize
    }

    /* Building one trile */

    /// `trile_calc_sides`: the three edges. Each is copied from the neighbour
    /// that already owns it, or from whichever corner of a nearby trile shares
    /// its end, and only then invented.
    fn calc_sides(&self, x: i32, y: i32) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let e = self.epoints;
        let dv = if (x + y).rem_euclid(2) != 0 { 1 } else { -1 };
        let root = !self.triles.is_empty();
        let find = |a: i32, b: i32| if root { self.triles.get(&(a, b)) } else { None };

        let l = find(x - 1, y);
        let r = find(x + 1, y);
        let v = find(x, y + dv);

        let mut nv = vec![0.0; e];
        let mut nl = vec![0.0; e];
        let mut nr = vec![0.0; e];

        if let Some(v) = v {
            nv.copy_from_slice(&v.v);
        } else {
            nv[0] = if let Some(l) = l {
                l.l[0]
            } else if !root {
                displace(0.0, 0)
            } else if let Some(t) = find(x - 1, y + dv) {
                t.l[0]
            } else if let Some(t) = find(x - 2, y) {
                t.r[0]
            } else if let Some(t) = find(x - 2, y + dv) {
                t.r[0]
            } else {
                displace(0.0, 0)
            };
            nv[e - 1] = if let Some(r) = r {
                r.l[0]
            } else if !root {
                displace(0.0, 0)
            } else if let Some(t) = find(x + 1, y + dv) {
                t.l[0]
            } else if let Some(t) = find(x + 2, y) {
                t.v[0]
            } else if let Some(t) = find(x + 2, y + dv) {
                t.v[0]
            } else {
                displace(0.0, 0)
            };
            subdivide(&mut nv, self.nsubdivs);
        }

        if let Some(l) = l {
            nl.copy_from_slice(&l.r);
        } else {
            nl[0] = if let Some(r) = r {
                r.v[0]
            } else if !root {
                displace(0.0, 0)
            } else if let Some(t) = find(x - 1, y - dv) {
                t.r[0]
            } else if let Some(t) = find(x + 1, y - dv) {
                t.v[0]
            } else if let Some(t) = find(x, y - dv) {
                t.l[0]
            } else {
                displace(0.0, 0)
            };
            nl[e - 1] = nv[0];
            subdivide(&mut nl, self.nsubdivs);
        }

        if let Some(r) = r {
            nr.copy_from_slice(&r.l);
        } else {
            nr[0] = nv[e - 1];
            nr[e - 1] = nl[0];
            subdivide(&mut nr, self.nsubdivs);
        }

        (nl, nr, nv)
    }

    /// `trile_calc_heights`: the edges go round the outside, then the inside
    /// is displaced into being from them, one subdivision at a time.
    fn calc_heights(&mut self, l: &[f64], r: &[f64], v: &[f64]) {
        let e = self.epoints;
        for i in 0..e - 1 {
            self.set_tcoord(i, 0, v[i]);
            self.set_tcoord(e - 1 - i, i, r[i]);
            self.set_tcoord(0, e - 1 - i, l[i]);
        }

        let mut i = (1usize << self.nsubdivs) >> 2;
        let mut k = 1;
        while i > 0 {
            for j in 1..(1usize << k) {
                for h in 1..=(1usize << k) - j {
                    // Rights, lefts and verts: the three new points of each
                    // triangle being split.
                    let right = mean(
                        self.tcoord(i * (2 * h - 2), i * (2 * j)),
                        self.tcoord(i * (2 * h), i * (2 * j - 2)),
                    );
                    self.set_tcoord(i * (2 * h - 1), i * (2 * j - 1), displace(right, k));

                    let left = mean(
                        self.tcoord(i * (2 * h), i * (2 * j - 2)),
                        self.tcoord(i * (2 * h), i * (2 * j)),
                    );
                    self.set_tcoord(i * (2 * h), i * (2 * j - 1), displace(left, k));

                    let vert = mean(
                        self.tcoord(i * (2 * h - 2), i * (2 * j)),
                        self.tcoord(i * (2 * h), i * (2 * j)),
                    );
                    self.set_tcoord(i * (2 * h - 1), i * (2 * j), displace(vert, k));
                }
            }
            i >>= 1;
            k += 1;
        }
    }

    /// `trile_calc_flat_norms`: one normal per face, from the cross product of
    /// its two edges written out by hand.
    fn calc_flat_norms(&mut self, tx: i32, ty: i32) {
        let e = self.epoints;
        let down = if (tx + ty).rem_euclid(2) != 0 {
            -1.0
        } else {
            1.0
        };
        let dy = down * M_SQRT3_2 / f64::from(1u32 << self.nsubdivs);
        let dx = self.dx0;

        for y in 0..e - 1 {
            let mut a = self.tcoord(0, y);
            let mut b = self.tcoord(0, y + 1);
            let xend = e - 1 - y;
            for x in 1..xend {
                let c = self.tcoord(x, y);
                let d = self.tcoord(x, y + 1);

                let i = self.findex(x, y, 0);
                self.norms[i] = down * dy * (a - c);
                self.norms[i + 1] = down * ((a - c) * dx / 2.0 - dx * (b - c));
                self.norms[i + 2] = down * dx * dy;

                let i = self.findex(x, y, 1);
                self.norms[i] = down * dy * (b - d);
                self.norms[i + 1] = down * ((c - d) * dx - dx * (b - d) / 2.0);
                self.norms[i + 2] = down * dx * dy;

                a = c;
                b = d;
            }
            // The last face of the row has no partner above it.
            let x = xend.max(1);
            let c = self.tcoord(x, y);
            let i = self.findex(x, y, 0);
            self.norms[i] = down * dy * (a - c);
            self.norms[i + 1] = down * ((a - c) * dx / 2.0 - dx * (b - c));
            self.norms[i + 2] = down * dx * dy;
        }
    }

    /// One vertex normal, averaged from the six heights around it.
    fn set_snorm(&mut self, x: usize, y: usize, down: bool, p: [f64; 6]) {
        let [a, b, c, d, e5, f6] = p;
        let i = 3 * self.tindex(x, y);
        self.norms[i] = ave3(a - d, 0.5 * (b - e5), -0.5 * (c - f6));
        self.norms[i + 1] =
            (if down { -1.0 } else { 1.0 }) * ave3(0.0, M_SQRT3_2 * (b - e5), M_SQRT3_2 * (c - f6));
        self.norms[i + 2] = 2.0 * self.dx0;
    }

    /// `trile_calc_smooth_norms`: one normal per point, from its neighbours.
    /// The corners assume level ground, which upstream marks with a "bah".
    fn calc_smooth_norms(&mut self, tx: i32, ty: i32) {
        let e = self.epoints;
        let down = (tx + ty).rem_euclid(2) != 0;

        // Corners.
        let cur = self.tcoord(0, 0);
        let (c01, c10) = (self.tcoord(0, 1), self.tcoord(1, 0));
        self.set_snorm(0, 0, down, [cur, cur, c01, c10, cur, cur]);
        let cur = self.tcoord(e - 1, 0);
        let (a, b) = (self.tcoord(e - 2, 0), self.tcoord(e - 2, 1));
        self.set_snorm(e - 1, 0, down, [a, b, cur, cur, cur, cur]);
        let cur = self.tcoord(0, e - 1);
        let (e5, f6) = (self.tcoord(1, e - 2), self.tcoord(0, e - 2));
        self.set_snorm(0, e - 1, down, [cur, cur, cur, cur, e5, f6]);

        // The vertical side.
        let mut prev = self.tcoord(0, 0);
        let mut cur = self.tcoord(1, 0);
        for i in 1..e - 1 {
            let next = self.tcoord(i + 1, 0);
            let (b, c) = (self.tcoord(i - 1, 1), self.tcoord(i, 1));
            self.set_snorm(i, 0, down, [prev, b, c, next, cur, cur]);
            prev = cur;
            cur = next;
        }

        // The right side.
        let mut prev = self.tcoord(e - 1, 0);
        let mut cur = self.tcoord(e - 2, 0);
        for i in 1..e - 1 {
            let next = self.tcoord(e - i - 2, i + 1);
            let a = self.tcoord(e - i - 2, i);
            let f6 = self.tcoord(e - i - 1, i - 1);
            self.set_snorm(e - i - 1, i, down, [a, next, cur, cur, prev, f6]);
            prev = cur;
            cur = next;
        }

        // The left side.
        let mut prev = self.tcoord(0, 0);
        let mut cur = self.tcoord(0, 1);
        for i in 1..e - 1 {
            let next = self.tcoord(0, i + 1);
            let (d, e5) = (self.tcoord(1, i), self.tcoord(1, i - 1));
            self.set_snorm(0, i, down, [cur, cur, next, d, e5, prev]);
            prev = cur;
            cur = next;
        }

        // And the inside.
        for i in 1..e.saturating_sub(2) {
            let mut prev = self.tcoord(0, i);
            let mut cur = self.tcoord(1, i);
            for j in 1..e - i - 1 {
                let next = self.tcoord(j + 1, i);
                let b = self.tcoord(j - 1, i + 1);
                let c = self.tcoord(j, i + 1);
                let e5 = self.tcoord(j + 1, i - 1);
                let f6 = self.tcoord(j, i - 1);
                self.set_snorm(j, i, down, [prev, b, c, next, e5, f6]);
                prev = cur;
                cur = next;
            }
        }
    }

    /* Colour */

    fn land_color(&self, z: f64) -> [f32; 3] {
        let z = z as f32;
        match self.scheme {
            Scheme::Plain => [(z / 0.35).powi(4), z / 0.35, (z / 0.35).powi(4)],
            Scheme::Ice => [(0.35 - z) / 0.35, (0.35 - z) / 0.35, 1.0],
            Scheme::Magma => [z / 0.35, z / 0.2, 0.0],
            Scheme::Vomit => {
                let n = (z / 0.35).abs() as f64;
                let v = &self.vomit;
                [
                    ((1.0 - n) * v[0] + n * v[3]) as f32,
                    ((1.0 - n) * v[1] + n * v[4]) as f32,
                    ((1.0 - n) * v[2] + n * v[5]) as f32,
                ]
            }
        }
    }

    fn water_color(&self, z: f64) -> [f32; 3] {
        let zf = z as f32;
        match self.scheme {
            Scheme::Plain | Scheme::Ice => [0.0, (zf + 0.35) * 1.6, 0.8],
            Scheme::Magma => [(zf + 0.35) * 1.6, zf + 0.35, 0.0],
            Scheme::Vomit => {
                let n = z / -0.35;
                let v = &self.vomit;
                [
                    ((1.0 - n) * v[6] + n * v[9]) as f32,
                    ((1.0 - n) * v[7] + n * v[10]) as f32,
                    ((1.0 - n) * v[8] + n * v[11]) as f32,
                ]
            }
        }
    }

    /* Drawing one trile */

    /// `trile_light`: which normal a vertex gets. The flat case has none for
    /// the leftmost column, so it borrows the one next to it; upstream reports
    /// Mesa errors and "bizarre glitches" without that.
    fn light_normal(&self, x: usize, y: usize, which: usize) -> [f32; 3] {
        let i = if self.flat {
            if x > 0 {
                self.findex(x, y, which)
            } else {
                self.findex(1, y, 0)
            }
        } else {
            3 * self.tindex(x, y + which)
        };
        [
            self.norms[i] as f32,
            self.norms[i + 1] as f32,
            self.norms[i + 2] as f32,
        ]
    }

    /// `trile_draw_vertex`: where a point of the mesh goes and what colour it
    /// is. Anything at or below the waterline is flattened onto it.
    fn vertex(&self, x: usize, y: usize, which: usize, px: f64, py: f64, z: f64) -> Vert {
        if self.do_water && z <= 0.0 {
            Vert {
                pos: [px as f32, py as f32, 0.0],
                color: self.water_color(z),
                normal: [0.0, 0.0, 1.0],
            }
        } else {
            Vert {
                pos: [px as f32, py as f32, z as f32],
                color: self.land_color(z),
                normal: if self.lit {
                    self.light_normal(x, y, which)
                } else {
                    [0.0, 0.0, 1.0]
                },
            }
        }
    }

    /// `trile_render`: bake one trile into a display list.
    ///
    /// Upstream draws each row as a triangle strip under `GL_FLAT`, where the
    /// whole of a face takes the colour and normal of its last vertex. There
    /// is no flat shading here, so the strips are cut into separate triangles
    /// and each face is given its provoking vertex's colour and normal three
    /// times over, which is the same picture. It also means every trile in the
    /// scene merges into one batch, since nothing changes between them.
    fn render(&self, g: &mut Gl, tx: i32, ty: i32, list: u32) {
        let e = self.epoints;
        let down = (tx + ty).rem_euclid(2) != 0;
        let mut cornerx = 0.5 * f64::from(tx) - 0.5;
        let mut cornery = if down {
            (f64::from(ty) + 0.5) * M_SQRT3_2
        } else {
            (f64::from(ty) - 0.5) * M_SQRT3_2
        };
        let mut dy = M_SQRT3_2 / f64::from(1u32 << self.nsubdivs);
        if down {
            dy = -dy;
        }
        let dx = self.dx0;

        let mut strip: Vec<Vert> = Vec::with_capacity(2 * e);
        g.glx.new_list(list);
        g.glx.begin(if self.wire {
            Shape::Lines
        } else {
            Shape::Triangles
        });
        for y in 0..e - 1 {
            strip.clear();
            for k in 0..2 * (e - y) - 1 {
                let (x, which) = (k / 2, k % 2);
                let z = self.tcoord(x, y + which);
                let px = cornerx + (x as f64 + 0.5 * which as f64) * dx;
                let py = cornery + which as f64 * dy;
                strip.push(self.vertex(x, y, which, px, py, z));
            }

            for i in 0..strip.len() - 2 {
                // A strip alternates its winding; the provoking vertex is the
                // last of each face either way.
                let face = if i % 2 == 0 {
                    [i, i + 1, i + 2]
                } else {
                    [i + 1, i, i + 2]
                };
                if self.wire {
                    for j in 0..3 {
                        emit(g, &strip[face[j]], &strip[face[j]], self.lit);
                        emit(
                            g,
                            &strip[face[(j + 1) % 3]],
                            &strip[face[(j + 1) % 3]],
                            self.lit,
                        );
                    }
                } else {
                    for &v in &face {
                        let attr = if self.flat { &strip[i + 2] } else { &strip[v] };
                        emit(g, &strip[v], attr, self.lit);
                    }
                }
            }

            cornerx += dx / 2.0;
            cornery += dy;
        }
        g.glx.end();
        g.glx.end_list();
    }

    /// `select_morph`: how the next trile is to arrive.
    fn select_morph(&self) -> Morph {
        if self.crack {
            match random() % 3 {
                0 => Morph::Grow,
                1 => Morph::Fall,
                _ => Morph::Yeast,
            }
        } else if self.boring {
            Morph::Identity
        } else {
            Morph::Grow
        }
    }

    /// `trile_new`: generate a triangle at `(x, y)` and put it in the lattice.
    fn trile_new(&mut self, g: &mut Gl, x: i32, y: i32) {
        let morph = self.select_morph();
        let (l, r, v) = self.calc_sides(x, y);
        self.calc_heights(&l, &r, &v);
        if self.lit {
            if self.flat {
                self.calc_flat_norms(x, y);
            } else {
                self.calc_smooth_norms(x, y);
            }
        }
        let list = match self.free_lists.pop() {
            Some(list) => list,
            None => g.glx.gen_lists(1),
        };
        self.render(g, x, y, list);

        let mut tr = Trile {
            x,
            y,
            state: State::New,
            visible: true,
            l,
            r,
            v,
            list,
            morph,
            data: 0.0,
        };
        tr.morph_init();
        self.triles.insert((x, y), tr);
    }

    /// `triles_set_visible`: mark one, making it if it is not there yet.
    fn set_visible(&mut self, g: &mut Gl, x: i32, y: i32) {
        match self.triles.get_mut(&(x, y)) {
            Some(tr) => tr.visible = true,
            None => self.trile_new(g, x, y),
        }
    }

    /* What is on screen */

    /// `calc_points`: flatten the view frustum onto the ground as a
    /// quadrilateral, then shrink it towards its own centre by `visibility`.
    fn calc_points(&self) -> [(f64, f64); 4] {
        let halfheight = (self.fovy / 2.0 * M_PI_180).tan() * self.z_near;
        let fovx_2 = (halfheight * self.aspect / self.z_near).atan() * M_180_PI;
        let z_far = self.z_far + M_RAD7_4;
        let fhalfwidth = z_far * (fovx_2 * M_PI_180).tan() + M_RAD7_4 / (fovx_2 * M_PI_180).cos();
        let x_farcenter = self.x + z_far * (self.yaw * M_PI_180).cos();
        let y_farcenter = self.y + z_far * (self.yaw * M_PI_180).sin();
        let side = (self.yaw - 90.0) * M_PI_180;
        let mut p = [
            (
                x_farcenter + fhalfwidth * side.cos(),
                y_farcenter + fhalfwidth * side.sin(),
            ),
            (
                x_farcenter - fhalfwidth * side.cos(),
                y_farcenter - fhalfwidth * side.sin(),
            ),
            (0.0, 0.0),
            (0.0, 0.0),
        ];

        // Where the near plane meets the ground, or the bottom of the frustum
        // if the camera is high enough that it does not.
        let z_near = if self.z - halfheight <= 0.0 {
            self.z_near - M_RAD7_4
        } else {
            self.z / (self.fovy / 2.0 * M_PI_180).tan() - M_RAD7_4
        };
        let nhalfwidth = z_near * (fovx_2 * M_PI_180).tan() + M_RAD7_4 / (fovx_2 * M_PI_180).cos();
        let x_nearcenter = self.x + z_near * (self.yaw * M_PI_180).cos();
        let y_nearcenter = self.y + z_near * (self.yaw * M_PI_180).sin();
        p[2] = (
            x_nearcenter - nhalfwidth * side.cos(),
            y_nearcenter - nhalfwidth * side.sin(),
        );
        p[3] = (
            x_nearcenter + nhalfwidth * side.cos(),
            y_nearcenter + nhalfwidth * side.sin(),
        );

        let x_center = (x_nearcenter + x_farcenter) / 2.0;
        let y_center = (y_nearcenter + y_farcenter) / 2.0;
        for q in &mut p {
            q.0 = self.visibility * q.0 + (1.0 - self.visibility) * x_center;
            q.1 = self.visibility * q.1 + (1.0 - self.visibility) * y_center;
        }
        p
    }

    /// `mark_visible`: scan the quadrilateral row by row and mark every trile
    /// each row crosses.
    fn mark_visible(&mut self, g: &mut Gl) {
        let p = self.calc_points();
        let mut ls: Vec<Ls> = Vec::with_capacity(4);
        for i in 0..4 {
            let (xa, ya) = p[i];
            let (xb, yb) = p[(i + 1) % 4];
            if (ya - yb).abs() <= 0.001 {
                continue;
            }
            let dx = (xb - xa) / (yb - ya);
            if yb > ya {
                ls.push(Ls {
                    min: ya,
                    max: yb,
                    start: xa,
                    dx,
                });
            } else {
                ls.push(Ls {
                    min: yb,
                    max: ya,
                    start: xb,
                    dx,
                });
            }
        }

        let (mut trough, mut peak) = (p[0].1, p[0].1);
        for q in &p[1..] {
            if q.1 > peak {
                peak = q.1;
            } else if q.1 < trough {
                trough = q.1;
            }
        }

        let start = (trough / M_SQRT3_2).ceil() as i32;
        let stop = (peak / M_SQRT3_2).floor() as i32;
        for y in start..=stop {
            let yval = f64::from(y) * M_SQRT3_2;
            let (left, right) = find_bounds(yval, &ls);
            let from = (left * 2.0 - 1.0).ceil() as i32;
            let to = (right * 2.0).floor() as i32;
            for x in from..=to {
                self.set_visible(g, x, y);
            }
        }
    }

    /// `triles_update_state`: everything marked this frame steps towards being
    /// there, everything not marked steps towards being gone. Upstream splices
    /// the dead out of its tree by hand; here they are keys to remove.
    fn update_state(&mut self) {
        let elapsed = self.elapsed;
        let mut dead: Vec<(i32, i32)> = Vec::new();
        for (key, tr) in &mut self.triles {
            if tr.visible {
                match tr.state {
                    State::Init => tr.morph_init_iter(elapsed),
                    State::Dying => {
                        tr.state = State::Init;
                        tr.morph_init_iter(elapsed);
                    }
                    State::New => tr.state = State::Init,
                    _ => {}
                }
                tr.visible = false;
            } else {
                match tr.state {
                    State::Stable => tr.state = State::Dying,
                    State::Init => {
                        tr.state = State::Dying;
                        tr.morph_dying_iter(elapsed);
                    }
                    State::Dying => tr.morph_dying_iter(elapsed),
                    _ => {}
                }
            }
            if tr.state == State::Delete {
                dead.push(*key);
            }
        }
        for key in dead {
            if let Some(tr) = self.triles.remove(&key) {
                self.free_lists.push(tr.list);
            }
        }
    }

    /// `trile_draw`: a settled trile is just its list; one still arriving or
    /// leaving is its list under whatever its morph is doing to it.
    fn draw_trile(g: &mut Gl, tr: &Trile) {
        if tr.state == State::Stable || tr.morph == Morph::Identity {
            g.glx.call_list(tr.list);
            return;
        }
        let z = tr.data as f32;
        g.glx.push_matrix();
        match tr.morph {
            Morph::Grow => g.glx.scale(1.0, 1.0, z),
            Morph::Fall => g.glx.translate(0.0, 0.0, (0.5 - tr.data as f32) * 8.0),
            Morph::Yeast => {
                let x = f64::from(tr.x) as f32 * 0.5;
                let y = f64::from(tr.y) as f32 * M_SQRT3_2 as f32;
                g.glx.translate(x, y, 0.0);
                g.glx.rotate(z * 360.0, 0.0, 0.0, 1.0);
                g.glx.scale(z, z, z);
                g.glx.translate(-x, -y, 0.0);
            }
            Morph::Identity => {}
        }
        g.glx.call_list(tr.list);
        g.glx.pop_matrix();
    }
}

/// One vertex: its own position, another vertex's colour and normal.
fn emit(g: &mut Gl, at: &Vert, attr: &Vert, lit: bool) {
    // Upstream sets black first, "don't ask, my card breaks otherwise", and
    // then overwrites it; only the second colour is ever seen.
    g.glx.color3f(attr.color[0], attr.color[1], attr.color[2]);
    if lit {
        g.glx
            .normal3f(attr.normal[0], attr.normal[1], attr.normal[2]);
    }
    g.glx.vertex3f(at.pos[0], at.pos[1], at.pos[2]);
}

/// `find_bounds`: where a horizontal line at `y` enters and leaves the
/// quadrilateral. An empty span if it somehow blew up.
fn find_bounds(y: f64, ls: &[Ls]) -> (f64, f64) {
    let mut left = 0.0;
    let mut set = false;
    for l in ls {
        if l.min <= y && l.max >= y {
            let x = (y - l.min) * l.dx + l.start;
            if !set {
                left = x;
                set = true;
            } else if (x - left).abs() > 0.001 {
                return if left < x { (left, x) } else { (x, left) };
            }
        }
    }
    (3.0, -3.0)
}

/// `drunken_rando`: a random walk that is pulled back towards the middle the
/// further out it gets, so the flight wanders without ever running away.
fn drunken_rando(cur: f64, max: f64, width: f64) -> f64 {
    let r = frand(2.0);
    if cur > 0.0 {
        if r >= 1.0 {
            cur + (r - 1.0) * width * (1.0 - cur / max)
        } else {
            cur - r * width
        }
    } else if r >= 1.0 {
        cur - (r - 1.0) * width * (1.0 + cur / max)
    } else {
        cur + r * width
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let nsubdivs = (g.res.int("nsubdivs") as u32) % 16;
    let mut visibility = g.res.float("visibility");
    if !(0.2..=1.0).contains(&visibility) {
        visibility = 1.0;
    }
    let flat = g.res.bool("flat");
    let epoints = 1 + (1usize << nsubdivs);
    let tpoints = epoints * (epoints + 1) / 2;
    let ntris = 1usize << (nsubdivs << 1);
    let tnorms = if flat { ntris } else { tpoints };

    let mut this = Crackberg {
        triles: BTreeMap::new(),
        free_lists: Vec::new(),
        x: 0.0,
        y: 0.0,
        z: 0.5,
        yaw: 0.0,
        dx: 0.0,
        dy: 0.0,
        dyaw: 0.0,
        elapsed: 0.0,
        prev_frame: 0.0,
        motion_state: MOTION_AUTO,
        keys: 0,
        mspeed: 1.0,
        fovy: 60.0,
        aspect: 1.0,
        z_near: 0.5,
        z_far: 5.0,
        scheme: Scheme::Plain,
        vomit: [0.0; 12],
        epoints,
        heights: vec![0.0; tpoints],
        norms: vec![0.0; 3 * tnorms],
        draw_elapsed: 1.0,
        dx0: 1.0 / f64::from(1u32 << nsubdivs),
        button_down: false,
        mouse: (0, 0),
        paused: 0.0,
        nsubdivs,
        crack: g.res.bool("crack"),
        boring: g.res.bool("boring"),
        do_water: g.res.bool("water"),
        flat,
        lit: g.res.bool("lit"),
        visibility,
        letterbox: g.res.bool("letterbox"),
        wire: g.res.bool("wireframe"),
    };

    // `select_color`, which also picks what to clear to.
    this.scheme = match g.res.string("color") {
        "plain" => Scheme::Plain,
        "ice" => Scheme::Ice,
        "magma" => Scheme::Magma,
        "vomit" => Scheme::Vomit,
        _ => match random() % 4 {
            0 => Scheme::Plain,
            1 => Scheme::Ice,
            2 => Scheme::Magma,
            _ => Scheme::Vomit,
        },
    };
    if this.scheme == Scheme::Vomit {
        for v in &mut this.vomit {
            *v = frand(1.0);
        }
        g.glx
            .clear_color(frand(1.0) as f32, frand(1.0) as f32, frand(1.0) as f32, 1.0);
    } else {
        let bg = this.scheme.bg();
        g.glx.clear_color(bg[0], bg[1], bg[2], bg[3]);
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Crackberg {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h2 = width * 9 / 16;
        if self.letterbox && h2 < height {
            g.glx.viewport(0, (height - h2) / 2, width, h2);
            self.aspect = f64::from(width) / f64::from(h2);
        } else {
            g.glx.viewport(0, 0, width, height);
            self.aspect = f64::from(width) / f64::from(height);
        }
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        match *event {
            XEvent::KeyPress { key } => {
                let bit = match key {
                    'a' => MOTION_LEFT,
                    'd' => MOTION_RIGHT,
                    's' => MOTION_BACK,
                    'w' => MOTION_FORW,
                    '1' => MOTION_DEC,
                    '2' => MOTION_INC,
                    // The arrow keys arrive as these.
                    '\u{f702}' => MOTION_LROT,
                    '\u{f703}' => MOTION_RROT,
                    '\u{f701}' => MOTION_BACK,
                    '\u{f700}' => MOTION_FORW,
                    ' ' => {
                        if self.motion_state == MOTION_MANUAL {
                            self.motion_state = MOTION_AUTO;
                        }
                        return true;
                    }
                    _ => return false,
                };
                self.keys |= bit;
                self.motion_state |= MOTION_MANUAL;
                true
            }
            XEvent::ButtonPress { x, y, button: 1 } => {
                self.button_down = true;
                self.mouse = (x, y);
                self.motion_state = MOTION_MANUAL;
                self.paused = 0.0;
                true
            }
            XEvent::ButtonRelease { button: 1, .. } => {
                self.button_down = false;
                self.motion_state = MOTION_AUTO;
                self.paused = g.time;
                true
            }
            XEvent::MotionNotify { x, y } if self.button_down => {
                let mut dx = x - self.mouse.0;
                let mut dy = y - self.mouse.1;
                self.mouse = (x, y);
                self.motion_state = MOTION_MANUAL;

                // Take the larger dimension, since the motion bits do not
                // scale.
                if dx > 0 && dx > dy {
                    dy = 0;
                }
                if dx < 0 && dx < dy {
                    dy = 0;
                }
                if dy > 0 && dy > dx {
                    dx = 0;
                }
                if dy < 0 && dy < dx {
                    dx = 0;
                }

                if dx > 0 {
                    self.motion_state |= MOTION_LEFT;
                } else if dx < 0 {
                    self.motion_state |= MOTION_RIGHT;
                } else if dy > 0 {
                    self.motion_state |= MOTION_FORW;
                } else if dy < 0 {
                    self.motion_state |= MOTION_BACK;
                }
                true
            }
            _ => false,
        }
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let now = g.time;
        if self.prev_frame != 0.0 {
            self.elapsed = now - self.prev_frame;
            let state = self.motion_state | self.keys;

            if state == MOTION_AUTO && self.paused + 1.0 <= now {
                self.x += self.dx * self.elapsed;
                self.y += self.dy * self.elapsed;
                self.yaw += self.dyaw * self.elapsed;

                self.draw_elapsed += self.elapsed;
                if self.draw_elapsed >= 0.8 {
                    self.draw_elapsed = 0.0;
                    self.dx = drunken_rando(self.dx, 2.5, 0.8);
                    self.dy = drunken_rando(self.dy, 2.5, 0.8);
                    self.dyaw = drunken_rando(self.dyaw, 40.0, 8.0);
                }
            } else {
                let scale = self.elapsed * self.mspeed;
                let (c, s) = ((self.yaw * M_PI_180).cos(), (self.yaw * M_PI_180).sin());
                if state & MOTION_BACK != 0 {
                    self.x -= c * scale;
                    self.y -= s * scale;
                }
                if state & MOTION_FORW != 0 {
                    self.x += c * scale;
                    self.y += s * scale;
                }
                if state & MOTION_LEFT != 0 {
                    self.x -= s * scale;
                    self.y += c * scale;
                }
                if state & MOTION_RIGHT != 0 {
                    self.x += s * scale;
                    self.y -= c * scale;
                }
                if state & MOTION_LROT != 0 {
                    self.yaw += 45.0 * scale;
                }
                if state & MOTION_RROT != 0 {
                    self.yaw -= 45.0 * scale;
                }
                if state & MOTION_DEC != 0 {
                    self.mspeed /= MSPEED_SCALE.powf(self.draw_elapsed);
                }
                if state & MOTION_INC != 0 {
                    self.mspeed *= MSPEED_SCALE.powf(self.draw_elapsed);
                }
            }
        }
        self.prev_frame = now;
        self.keys = 0;

        self.mark_visible(g);
        self.update_state();

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(
            self.fovy as f32,
            self.aspect as f32,
            self.z_near as f32,
            self.z_far as f32,
        );
        g.glx.matrix_mode_modelview();

        g.glx.clear();
        g.glx.depth_test(true);
        // Nothing is ever culled, so which way a face is wound does not matter
        // here: the shader lights a back face by its own side, and the colour
        // comes from the vertex either way.
        g.glx.cull_face(false);
        g.glx.blend(Blend::Alpha);
        g.glx.lighting(self.lit);
        if self.lit {
            g.glx.color_material(true);
            g.glx.light_enable(0, true);
        }

        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        g.glx.light_position(0, 2.0, 0.0, -0.3, 0.0);
        // The camera sees `yaw` over.
        g.glx.rotate(-self.yaw as f32, 0.0, 0.0, 1.0);
        g.glx
            .translate(-self.x as f32, -self.y as f32, -self.z as f32);

        for tr in self.triles.values() {
            Self::draw_trile(g, tr);
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:      20000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*nsubdivs:   4",
    "*boring:     False",
    "*crack:      True",
    "*water:      True",
    "*flat:       True",
    "*color:      random",
    "*lit:        True",
    "*visibility: 0.6",
    "*letterbox:  False",
];

const COLORS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "random",
        label: "Random coloration",
    },
    crate::runtime::opts::SelectItem {
        value: "plain",
        label: "Earthy coloration",
    },
    crate::runtime::opts::SelectItem {
        value: "ice",
        label: "Icy coloration",
    },
    crate::runtime::opts::SelectItem {
        value: "magma",
        label: "Swampy coloration",
    },
    crate::runtime::opts::SelectItem {
        value: "vomit",
        label: "Vomitous coloration",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("visibility", "Visibility", 0.2, 1.0, 0.05, 2, "0.6"),
    // Upstream's slider goes to nine subdivisions, a quarter of a million
    // triangles in every one of the hundred-odd triles on screen, which its
    // own label calls "hurt me". Measured here at 1280x720: four is 38k
    // vertices a frame and five is 154k, so five is the last one that fits.
    Opt::slider("nsubdivs", "Subdivisions", 2.0, 5.0, 1.0, 0, "4"),
    Opt::select("color", "Coloration", COLORS, "random"),
    Opt::boolean("flat", "Flat shading", "true"),
    Opt::boolean("lit", "Lighting", "true"),
    Opt::boolean("water", "Water", "true"),
    Opt::boolean("crack", "Confused", "true"),
    Opt::boolean("boring", "Immediate", "false"),
    Opt::boolean("letterbox", "Letterbox", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "crackberg",
    label: "Crackberg",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Matus Telgarsky",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=ej1No4EK8Rc"),
        blurb: "Flies through height maps, optionally animating the creation \
                and destruction of generated tiles; tiles `grow' into place.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    /// The whole point of the edge sharing: a lattice point reached from two
    /// different triles has to be at the same height, or there is a crack in
    /// the range. Read off the drawn mesh rather than the state, since that is
    /// where a crack would actually show.
    #[test]
    fn the_range_has_no_cracks() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        for _ in 0..5 {
            r.step();
        }
        let f = r.frame();
        // Points are a sixteenth of a unit apart at four subdivisions, so a
        // ten-thousandth is far below the spacing and far above the drift of
        // adding up the corner offsets in single precision.
        let key = |v: f32| (f64::from(v) * 8192.0).round() as i64;
        let mut seen: HashMap<(i64, i64), f32> = HashMap::new();
        let mut shared = 0;
        for v in &f.vertices {
            let k = (key(v.pos[0]), key(v.pos[1]));
            match seen.get(&k) {
                Some(&z) => {
                    assert!(
                        (z - v.pos[2]).abs() < 1e-4,
                        "a crack at ({}, {}): {z} and {}",
                        v.pos[0],
                        v.pos[1],
                        v.pos[2]
                    );
                    shared += 1;
                }
                None => {
                    seen.insert(k, v.pos[2]);
                }
            }
        }
        assert!(shared > 1000, "only {shared} points were reached twice");
    }

    /// The mesh is a handful of batches: nothing changes between one trile and
    /// the next, so the hundred-odd of them merge into one run.
    #[test]
    fn the_range_is_drawn_in_few_batches() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        for _ in 0..30 {
            r.step();
        }
        let f = r.frame();
        assert!(
            f.batches.len() < 60,
            "{} batches for the mesh",
            f.batches.len()
        );
        assert!(!f.vertices.is_empty(), "nothing was drawn");
    }

    /// Triles that fall out of view are taken apart again, so flying on does
    /// not pile up more and more mesh.
    #[test]
    fn what_goes_behind_is_taken_apart() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        for _ in 0..30 {
            r.step();
        }
        let early = r.frame().vertices.len();
        for _ in 0..300 {
            r.step();
        }
        let late = r.frame().vertices.len();
        assert!(early > 0 && late > 0, "nothing was drawn");
        assert!(
            late < early * 2,
            "the mesh grew from {early} to {late} vertices"
        );
    }

    /// Midpoint displacement leaves the two ends where they were and puts
    /// something between them.
    #[test]
    fn subdividing_keeps_the_ends() {
        let mut edge = vec![0.0; 17];
        edge[0] = 1.0;
        edge[16] = -1.0;
        subdivide(&mut edge, 4);
        assert_eq!(edge[0], 1.0);
        assert_eq!(edge[16], -1.0);
        assert!(edge[1..16].iter().all(|&z| z.abs() < 2.0));
        assert!(edge[8] != 0.0, "the middle was never displaced");
    }

    /// The wander is pulled back the further out it gets, so it never runs
    /// away however long it walks.
    #[test]
    fn the_wander_stays_bounded() {
        let mut v = 0.0;
        for _ in 0..10_000 {
            v = drunken_rando(v, 2.5, 0.8);
            assert!(v.abs() < 3.4, "the wander reached {v}");
        }
    }

    /// A row of the scan crosses the quadrilateral twice, and a row that
    /// misses it gets an empty span rather than a wrong one.
    #[test]
    fn a_row_crosses_the_view_twice() {
        let square = [
            Ls {
                min: 0.0,
                max: 2.0,
                start: -1.0,
                dx: 0.0,
            },
            Ls {
                min: 0.0,
                max: 2.0,
                start: 1.0,
                dx: 0.0,
            },
        ];
        assert_eq!(find_bounds(1.0, &square), (-1.0, 1.0));
        let (left, right) = find_bounds(5.0, &square);
        assert!(left > right, "a row outside the view found {left}..{right}");
    }
}
