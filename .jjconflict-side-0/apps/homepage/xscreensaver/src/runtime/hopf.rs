//! The geometry and the choreography of the Hopf fibration.
//!
//! ```text
//! hopffibration --- Displays the Hopf fibration of the 4D hypersphere S³.
//!
//! Copyright (c) 2025-2026 Carsten Steger <carsten@mirsanmir.org>.
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
//! ```
//!
//! The Hopf map sends the 3-sphere in four dimensions onto the ordinary
//! sphere in three, and every point of the ordinary sphere is the image of a
//! whole great circle. Pick a point on the sphere, take the great circle it
//! came from, project that from four dimensions down to three, and you get a
//! closed curve. That curve is a *fiber*, and this is the machinery for
//! working one out and for choosing which points to draw fibers of.
//!
//! Three parts, all of them upstream's, and all of them independent of
//! anything that draws:
//!
//! * The circle itself, in [`HopfCircle`]: the stereographic projection of a
//!   great circle of the 3-sphere, compressed so that the whole of infinite
//!   space fits in a ball, along with the Frenet frame that a tube is swept
//!   along. The curve is turned into a polygon by recursive subdivision
//!   rather than by sampling at fixed steps: a segment is split while the
//!   midpoint of the curve is further than a given distance from the chord.
//!
//! * The sphere the base points sit on, in [`Icosphere`]: an icosahedron
//!   whose faces are cut into `s` by `s` triangles and whose vertices are
//!   then pushed out onto the sphere. It is drawn half transparent, so its
//!   triangles have to be sorted back to front, which [`Icosphere::sort`]
//!   does and remembers.
//!
//! * The choreography, in [`Animations`]: eight configurations the base
//!   points can be in, and a table of sixty-four sets of animations for
//!   getting from any one of them to any other. That table is half a
//!   megabyte of C struct literals upstream, converted by
//!   `apps/homepage/gen-hopfanimations.nu` into an asset this module reads.
//!
//! [`crate::hacks3d::hopffibration`] is what draws it. How much geometry that
//! comes to is the saver's one difficulty and is measured by a test at the
//! bottom of this file: the heaviest of the hundred and eighty-eight
//! animations is two hundred and sixteen fibers, or 767k vertices a frame at
//! the coarsest detail level. The median is a third of that.

use std::f32::consts::PI;

/* ------------------------------------------------------------------ */
/* Vectors and rotations                                              */
/* ------------------------------------------------------------------ */

pub fn norm(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

pub fn normalize(v: &mut [f32; 3]) {
    let n = norm(*v);
    if n != 0.0 {
        for c in v.iter_mut() {
            *c /= n;
        }
    }
}

fn normalize_to_length(v: &mut [f32; 3], r: f32) {
    let n = norm(*v);
    if n != 0.0 {
        for c in v.iter_mut() {
            *c *= r / n;
        }
    }
}

pub fn cross(m: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    [
        m[1] * n[2] - m[2] * n[1],
        m[2] * n[0] - m[0] * n[2],
        m[0] * n[1] - m[1] * n[0],
    ]
}

/// How far `p` is from the line through `q` and `r`.
fn distance_point_line(p: [f32; 3], q: [f32; 3], r: [f32; 3]) -> f32 {
    let v = [r[0] - q[0], r[1] - q[1], r[2] - q[2]];
    let lv = norm(v);
    let w = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let lc = norm(cross(w, v));
    if lv > 0.0 { lc / lv } else { 0.0 }
}

/// A three by three rotation matrix, row major as upstream writes them.
pub type Mat3 = [[f32; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Add a rotation around the x axis to the matrix, in degrees.
pub fn rotatex(m: &mut Mat3, phi: f32) {
    let phi = phi * PI / 180.0;
    let (s, c) = phi.sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[1], row[2]);
        row[1] = c * u + s * v;
        row[2] = -s * u + c * v;
    }
}

/// Add a rotation around the y axis to the matrix, in degrees.
pub fn rotatey(m: &mut Mat3, phi: f32) {
    let phi = phi * PI / 180.0;
    let (s, c) = phi.sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[0], row[2]);
        row[0] = c * u - s * v;
        row[2] = s * u + c * v;
    }
}

/// Add a rotation around the z axis to the matrix, in degrees.
pub fn rotatez(m: &mut Mat3, phi: f32) {
    let phi = phi * PI / 180.0;
    let (s, c) = phi.sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[0], row[1]);
        row[0] = c * u + s * v;
        row[1] = -s * u + c * v;
    }
}

pub fn rotateall(al: f32, be: f32, de: f32) -> Mat3 {
    let mut m = IDENTITY;
    rotatex(&mut m, al);
    rotatey(&mut m, be);
    rotatez(&mut m, de);
    m
}

/// `o = m * n`.
pub fn mult_rotmat(m: &Mat3, n: &Mat3) -> Mat3 {
    let mut o = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| m[i][k] * n[k][j]).sum();
        }
    }
    o
}

/// `o = m * v`.
pub fn mult_rotmat_vec(m: &Mat3, v: [f32; 3]) -> [f32; 3] {
    let mut o = [0.0f32; 3];
    for (i, out) in o.iter_mut().enumerate() {
        *out = (0..3).map(|j| m[i][j] * v[j]).sum();
    }
    o
}

/// The rotation part of `gluLookAt`.
pub fn look_at_rotmat(eye: [f32; 3], centre: [f32; 3], up: [f32; 3]) -> Mat3 {
    let mut forward = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
    normalize(&mut forward);
    let mut side = cross(forward, up);
    normalize(&mut side);
    let up = cross(side, forward);
    [side, up, [-forward[0], -forward[1], -forward[2]]]
}

pub fn quat_to_rotmat(q: [f32; 4]) -> Mat3 {
    [
        [
            q[0] * q[0] + q[1] * q[1] - q[2] * q[2] - q[3] * q[3],
            2.0 * (q[1] * q[2] - q[0] * q[3]),
            2.0 * (q[1] * q[3] + q[0] * q[2]),
        ],
        [
            2.0 * (q[1] * q[2] + q[0] * q[3]),
            q[0] * q[0] - q[1] * q[1] + q[2] * q[2] - q[3] * q[3],
            2.0 * (q[2] * q[3] - q[0] * q[1]),
        ],
        [
            2.0 * (q[1] * q[3] - q[0] * q[2]),
            2.0 * (q[2] * q[3] + q[0] * q[1]),
            q[0] * q[0] - q[1] * q[1] - q[2] * q[2] + q[3] * q[3],
        ],
    ]
}

/// A quaternion from a rotation axis and an angle in radians.
pub fn axis_angle_to_quat(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let (s, c) = (0.5 * angle).sin_cos();
    [c, s * axis[0], s * axis[1], s * axis[2]]
}

/* ------------------------------------------------------------------ */
/* Easing                                                             */
/* ------------------------------------------------------------------ */

pub const EASING_NONE: i32 = 0;
pub const EASING_CUBIC: i32 = 1;
pub const EASING_SIN: i32 = 2;
pub const EASING_COS: i32 = 3;
pub const EASING_LIN: i32 = 4;
pub const EASING_ACCEL: i32 = 5;
pub const EASING_DECEL: i32 = 6;

/// Upstream's easing functions, for `t` between nought and one.
///
/// `EASING_NONE` returns nought whatever `t` is, which is how a parameter
/// that does not move is spelled: everything is interpolated from its start
/// value to its end value by this, and an easing of nought never leaves the
/// start.
pub fn ease(t: f32, easing: i32) -> f32 {
    match easing {
        EASING_CUBIC => t * t * (3.0 - 2.0 * t),
        EASING_SIN => {
            if t < 0.25 {
                0.5 * ease(4.0 * t, EASING_CUBIC) + 0.5
            } else if t > 0.75 {
                0.5 * ease(4.0 * t - 3.0, EASING_CUBIC)
            } else {
                1.0 - ease(2.0 * t - 0.5, EASING_CUBIC)
            }
        }
        EASING_COS => {
            if t < 0.5 {
                1.0 - ease(2.0 * t, EASING_CUBIC)
            } else {
                ease(2.0 * t - 1.0, EASING_CUBIC)
            }
        }
        EASING_LIN => t,
        EASING_ACCEL => t * t * (2.0 - t),
        EASING_DECEL => t * (1.0 + t * (1.0 - t)),
        _ => 0.0,
    }
}

/* ------------------------------------------------------------------ */
/* The fibers                                                         */
/* ------------------------------------------------------------------ */

/// A point on the unit sphere, which one fiber comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BasePoint {
    pub a: f32,
    pub b: f32,
    pub c: f32,
}

impl BasePoint {
    /// The colour of the fiber from here, and of the dot that marks it: the
    /// point's own coordinates, moved from minus one to one into nought to
    /// one.
    pub fn color(&self) -> [f32; 4] {
        [
            0.5 * (1.0 + self.a),
            0.5 * (1.0 + self.b),
            0.5 * (1.0 + self.c),
            1.0,
        ]
    }

    pub fn rotated(&self, m: &Mat3) -> BasePoint {
        let v = mult_rotmat_vec(m, [self.a, self.b, self.c]);
        BasePoint {
            a: v[0],
            b: v[1],
            c: v[2],
        }
    }
}

/// One point of a projected fiber, with the frame a tube is swept along it.
#[derive(Clone, Copy, Debug, Default)]
pub struct CirclePoint {
    pub p: [f32; 3],
    /// Unit tangent.
    pub t: [f32; 3],
    /// Unit normal.
    pub n: [f32; 3],
    /// Unit binormal. The projected fiber lies in a plane, so this is the
    /// same for every point of one fiber.
    pub b: [f32; 3],
    pub phi: f32,
}

/// Sixty-four segments is as deep as the subdivision ever goes.
const MAX_CIRCLE_STACK: usize = 64;
/// And this many points is as many as a fiber ever needs.
pub const MAX_CIRCLE_PNT: usize = 512;

/// The cosine of one degree, to as many digits as an `f32` can tell apart.
/// A base point closer to the north pole than this
/// has a fiber that projects to a straight line rather than to a circle.
const COS_1: f32 = 0.9998477;

/// The parameters of one fiber, worked out from its base point.
#[derive(Clone, Copy, Debug)]
pub struct HopfCircle {
    pub base: BasePoint,
    al: f32,
    be: f32,
    atab: f32,
}

impl HopfCircle {
    pub fn new(base: BasePoint) -> Self {
        HopfCircle {
            base,
            al: (0.5 * (1.0 + base.c)).sqrt(),
            be: (0.5 * (1.0 - base.c)).sqrt(),
            atab: (-base.a).atan2(base.b),
        }
    }

    /// Where the fiber is at parameter `phi`.
    ///
    /// The great circle in four dimensions is projected stereographically to
    /// three, which sends it off to infinity, and the whole of space is then
    /// squeezed into the unit ball by scaling each point by the arc it is
    /// along.
    pub fn point(&self, phi: f32) -> CirclePoint {
        let theta = phi;
        let phase = self.atab - phi;
        let w = self.al * theta.cos();
        let x = -self.be * phase.cos();
        let y = -self.be * phase.sin();
        let z = self.al * theta.sin();
        let r = w.acos() / (PI * (1.0 - w * w).sqrt());
        CirclePoint {
            p: [x * r, y * r, z * r],
            phi,
            ..CirclePoint::default()
        }
    }

    /// The unit tangent at `phi`, differentiated by hand.
    pub fn tangent(&self, phi: f32) -> [f32; 3] {
        let theta = phi;
        let phase = self.atab - phi;

        let w = self.al * theta.cos();
        let x = -self.be * phase.cos();
        let y = -self.be * phase.sin();
        let z = self.al * theta.sin();

        let s = (1.0 - w * w).sqrt();
        let n = w.acos();
        let d = PI * s;
        let r = n / d;

        let dwdp = -self.al * theta.sin();
        let dxdp = -self.be * phase.sin();
        let dydp = self.be * phase.cos();
        let dzdp = self.al * theta.cos();

        let dndp = -dwdp / s;
        let dddp = -PI * w * dwdp / s;
        let drdp = (dndp * d - n * dddp) / (d * d);

        let mut t = [
            dxdp * r + x * drdp,
            dydp * r + y * drdp,
            dzdp * r + z * drdp,
        ];
        normalize(&mut t);
        t
    }

    /// The unit binormal, which is the same all the way round because the
    /// projected fiber is a plane curve.
    pub fn binormal(&self) -> [f32; 3] {
        let (a, b, c) = (self.base.a, self.base.b, self.base.c);
        let sab = (a * a + b * b).sqrt();
        if sab > 0.0 {
            let pisqr8 = 2.0 * std::f32::consts::SQRT_2 * PI;
            let sq1mc = (1.0 - c).sqrt();
            let sq1pc = (1.0 + c).sqrt();
            let ac = (-sq1pc / std::f32::consts::SQRT_2).acos();
            let mut n = [
                -a * sq1pc * ac / (pisqr8 * sab),
                -b * sq1pc * ac / (pisqr8 * sab),
                sq1mc * ac / pisqr8,
            ];
            normalize(&mut n);
            n
        } else if c >= 0.0 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        }
    }

    /// Is this the one fiber that goes through the north pole, which the
    /// projection sends to a straight line rather than to a closed curve?
    pub fn is_line(&self) -> bool {
        self.base.c >= COS_1
    }

    /// The fiber as a polygon, no part of which is further than `max_dist`
    /// from the curve it approximates.
    ///
    /// Segments are split in half while the middle of the curve is too far
    /// from the chord, which puts the points where the curve bends rather
    /// than spreading them evenly.
    pub fn points(&self, max_dist: f32) -> Vec<CirclePoint> {
        if self.is_line() {
            // The fiber through the north pole becomes the whole z axis,
            // squeezed down to a segment of it, and is drawn as a cylinder
            // with a cap at each end. It should be thought of as a circle
            // that closes through infinity.
            let mut lo = CirclePoint {
                p: [0.0, 0.0, 1.0],
                t: [0.0, 0.0, 1.0],
                n: [0.0, 1.0, 0.0],
                b: [1.0, 0.0, 0.0],
                phi: 0.0,
            };
            let mut hi = lo;
            hi.p = [0.0, 0.0, -1.0];
            hi.phi = 2.0 * PI;
            lo.phi = 0.0;
            return vec![lo, hi];
        }

        // Four quarter circles to start with, taken off the stack in the
        // order upstream takes them so the points come out in order.
        let mut stack: Vec<(CirclePoint, CirclePoint)> = Vec::with_capacity(MAX_CIRCLE_STACK);
        for i in (0..4).rev() {
            let s = self.point(i as f32 * PI / 2.0);
            let e = self.point((i + 1) as f32 * PI / 2.0);
            stack.push((s, e));
        }

        let mut out: Vec<CirclePoint> = Vec::with_capacity(64);
        let mut last = stack[0].1;
        while let Some((s, e)) = stack.pop() {
            let phi = 0.5 * (s.phi + e.phi);
            let mid = self.point(phi);
            if distance_point_line(mid.p, s.p, e.p) > max_dist
                && stack.len() < MAX_CIRCLE_STACK - 2
                && out.len() < MAX_CIRCLE_PNT - 2
            {
                stack.push((mid, e));
                stack.push((s, mid));
            } else {
                out.push(s);
                last = e;
            }
        }
        out.push(last);

        // The frame. The binormal is constant, the tangent is worked out per
        // point, and the normal closes the frame.
        let b = self.binormal();
        for p in &mut out {
            p.b = b;
            p.t = self.tangent(p.phi);
            p.n = cross(b, p.t);
        }
        out
    }
}

/* ------------------------------------------------------------------ */
/* Where the base points go                                           */
/* ------------------------------------------------------------------ */

pub const GEN_TORUS: i32 = 0;
pub const GEN_SPIRAL: i32 = 1;

/// `gen_hopf_torus_base`: base points along a closed curve on the sphere.
///
/// With `q` and `r` at nought this is a circle of latitude at `p`; raising
/// them makes it wave up and down `n` times as it goes round, which is what
/// turns the Clifford torus its fibers make into a Hopf torus.
pub fn gen_torus_base(
    out: &mut Vec<BasePoint>,
    p: f32,
    q: f32,
    r: f32,
    n: i32,
    offset: f32,
    sector: f32,
    num: i32,
    rotate: bool,
    quat: [f32; 4],
) {
    let mut num = num.max(1);
    if (p == 0.0 || p == PI) && q == 0.0 {
        num = 1;
    }
    if sector == 0.0 {
        num = 1;
    }
    let m = rotate.then(|| quat_to_rotmat(quat));

    for i in 0..num {
        let t = offset + i as f32 * sector / num as f32;
        let g = if q == 0.0 {
            p
        } else {
            p + q * (n as f32 * t).sin()
        };
        let h = if r == 0.0 {
            t
        } else {
            t + r * (n as f32 * t).cos()
        };
        let bp = [h.cos() * g.sin(), h.sin() * g.sin(), g.cos()];
        let bp = match &m {
            Some(m) => mult_rotmat_vec(m, bp),
            None => bp,
        };
        out.push(BasePoint {
            a: bp[0],
            b: bp[1],
            c: bp[2],
        });
    }
}

/// `gen_hopf_spiral_base`: base points along a spiral that winds from one
/// pole of the sphere towards the other.
pub fn gen_spiral_base(
    out: &mut Vec<BasePoint>,
    p: f32,
    q: f32,
    r: f32,
    offset: f32,
    sector: f32,
    num: i32,
    rotate: bool,
    quat: [f32; 4],
) {
    let mut num = num.max(1);
    if sector == 0.0 {
        num = 1;
    }
    let m = rotate.then(|| quat_to_rotmat(quat));

    for i in 0..num {
        let t = offset + i as f32 * sector / num as f32;
        let u = p + 0.5 * q * t;
        let bp = [(-r * t).cos() * u.cos(), (-r * t).sin() * u.cos(), -u.sin()];
        let bp = match &m {
            Some(m) => mult_rotmat_vec(m, bp),
            None => bp,
        };
        out.push(BasePoint {
            a: bp[0],
            b: bp[1],
            c: bp[2],
        });
    }
}

/* ------------------------------------------------------------------ */
/* The base sphere                                                    */
/* ------------------------------------------------------------------ */

/// The twelve vertices of an icosahedron.
#[rustfmt::skip]
const ICOSA_VERT: [[f32; 3]; 12] = [
    [ 0.0,          0.0,          1.0],
    [ 0.8944272, 0.0,          0.4472136],
    [ 0.2763932, 0.8506508, 0.4472136],
    [-0.7236068, 0.5257311, 0.4472136],
    [-0.7236068,-0.5257311, 0.4472136],
    [ 0.2763932,-0.8506508, 0.4472136],
    [ 0.7236068, 0.5257311,-0.4472136],
    [-0.2763932, 0.8506508,-0.4472136],
    [-0.8944272, 0.0,         -0.4472136],
    [-0.2763932,-0.8506508,-0.4472136],
    [ 0.7236068,-0.5257311,-0.4472136],
    [ 0.0,          0.0,         -1.0],
];

/// And its twenty triangles.
#[rustfmt::skip]
const ICOSA_TRI: [[usize; 3]; 20] = [
    [0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5], [0, 5, 1],
    [1, 6, 2], [2, 7, 3], [3, 8, 4], [4, 9, 5], [5,10, 1],
    [2, 6, 7], [3, 7, 8], [4, 8, 9], [5, 9,10], [1,10, 6],
    [6,11, 7], [7,11, 8], [8,11, 9], [9,11,10], [10,11,6],
];

/// A sphere made by cutting each face of an icosahedron into `s` by `s`
/// triangles and pushing the vertices out onto the sphere.
pub struct Icosphere {
    pub vert: Vec<[f32; 3]>,
    pub norm: Vec<[f32; 3]>,
    pub tri: Vec<[usize; 3]>,
    /// The same triangles, sorted back to front for the last matrix passed
    /// to [`Icosphere::sort`].
    pub stri: Vec<[usize; 3]>,
    smat: Option<Mat3>,
}

impl Icosphere {
    /// `gen_icosphere_data`.
    pub fn new(s: usize, r: f32) -> Self {
        let s = s.max(1);
        let num_vert = s * s * (ICOSA_VERT.len() - 2) + 2;
        let num_tri = s * s * ICOSA_TRI.len();
        let mut vert: Vec<[f32; 3]> = Vec::with_capacity(num_vert);
        let mut tri: Vec<[usize; 3]> = Vec::with_capacity(num_tri);

        // Which vertices each edge of the icosahedron gained, indexed both
        // ways round: walking an edge from the other end gives the same
        // points in reverse.
        let nv = ICOSA_VERT.len();
        let mut edge: Vec<Vec<usize>> = vec![Vec::new(); nv * nv];
        let mut adj = vec![false; nv * nv];
        for t in ICOSA_TRI {
            for j in 0..3 {
                adj[t[j] * nv + t[(j + 1) % 3]] = true;
                adj[t[(j + 1) % 3] * nv + t[j]] = true;
            }
        }

        vert.extend_from_slice(&ICOSA_VERT);

        for i in 0..nv {
            for j in i + 1..nv {
                if !adj[i * nv + j] {
                    continue;
                }
                let mut fwd = Vec::with_capacity(s.saturating_sub(1));
                for m in 1..s {
                    let t = m as f32 / s as f32;
                    let a = ICOSA_VERT[i];
                    let b = ICOSA_VERT[j];
                    fwd.push(vert.len());
                    vert.push([
                        (1.0 - t) * a[0] + t * b[0],
                        (1.0 - t) * a[1] + t * b[1],
                        (1.0 - t) * a[2] + t * b[2],
                    ]);
                }
                let mut back = fwd.clone();
                back.reverse();
                edge[i * nv + j] = fwd;
                edge[j * nv + i] = back;
            }
        }

        // Then the vertices inside each face, laid out as a triangle of rows
        // so that the two triangles of each cell can be read straight off.
        let mut vi: Vec<usize> = Vec::with_capacity((s + 1) * (s + 2) / 2);
        for t in ICOSA_TRI {
            let (i1, i2, i3) = (t[0], t[1], t[2]);
            vi.clear();
            vi.push(i2);
            for j in 1..s {
                let k1 = edge[i2 * nv + i3][j - 1];
                let k2 = edge[i2 * nv + i1][j - 1];
                vi.push(k1);
                for k in 1..j {
                    let t = k as f32 / j as f32;
                    let a = vert[k1];
                    let b = vert[k2];
                    vi.push(vert.len());
                    vert.push([
                        (1.0 - t) * a[0] + t * b[0],
                        (1.0 - t) * a[1] + t * b[1],
                        (1.0 - t) * a[2] + t * b[2],
                    ]);
                }
                vi.push(k2);
            }
            vi.push(i3);
            for k in 1..s {
                vi.push(edge[i3 * nv + i1][k - 1]);
            }
            vi.push(i1);

            for j in 0..s {
                let k1 = j * (j + 1) / 2;
                let k2 = k1 + j + 1;
                for k in 0..j {
                    tri.push([vi[k1 + k], vi[k2 + k], vi[k2 + k + 1]]);
                    tri.push([vi[k1 + k], vi[k2 + k + 1], vi[k1 + k + 1]]);
                }
                tri.push([vi[k1 + j], vi[k2 + j], vi[k2 + j + 1]]);
            }
        }

        let norm: Vec<[f32; 3]> = vert
            .iter()
            .map(|v| {
                let mut n = *v;
                normalize(&mut n);
                n
            })
            .collect();
        for v in &mut vert {
            normalize_to_length(v, r);
        }

        Icosphere {
            stri: tri.clone(),
            vert,
            norm,
            tri,
            smat: None,
        }
    }

    /// Sort the triangles back to front under `mat`, which the base sphere
    /// needs because it is drawn half transparent. Does nothing, and says so,
    /// if the matrix has not moved since last time.
    pub fn sort(&mut self, mat: &Mat3) -> bool {
        if self.smat == Some(*mat) {
            return false;
        }
        let vertz: Vec<f32> = self
            .vert
            .iter()
            .map(|v| mat[2][0] * v[0] + mat[2][1] * v[1] + mat[2][2] * v[2])
            .collect();
        let mut order: Vec<usize> = (0..self.tri.len()).collect();
        order.sort_by(|&a, &b| {
            let za: f32 = self.tri[a].iter().map(|&i| vertz[i]).sum();
            let zb: f32 = self.tri[b].iter().map(|&i| vertz[i]).sum();
            za.total_cmp(&zb)
        });
        for (out, &i) in self.stri.iter_mut().zip(&order) {
            *out = self.tri[i];
        }
        self.smat = Some(*mat);
        true
    }
}

/* ------------------------------------------------------------------ */
/* The choreography                                                   */
/* ------------------------------------------------------------------ */

/// How one animated object moves over one phase: each parameter as a start
/// value, an end value and the easing that gets between them.
#[derive(Clone, Copy, Debug, Default)]
pub struct SingleObj {
    pub generator: i32,
    pub p_start: f32,
    pub p_end: f32,
    pub easing_p: i32,
    pub q_start: f32,
    pub q_end: f32,
    pub easing_q: i32,
    pub r_start: f32,
    pub r_end: f32,
    pub easing_r: i32,
    pub offset_start: f32,
    pub offset_end: f32,
    pub easing_offset: i32,
    pub sector_start: f32,
    pub sector_end: f32,
    pub easing_sector: i32,
    pub n: i32,
    pub num: i32,
    pub rot_axis_base: [f32; 3],
    pub angle_start: f32,
    pub angle_end: f32,
    pub easing_rotate: i32,
}

/// One phase: the objects that move together, and whether the whole
/// projection turns while they do.
#[derive(Clone, Debug, Default)]
pub struct MultiObj {
    pub so: Vec<SingleObj>,
    /// How often this phase also spins the base points about a random axis.
    pub rotate_prob: f32,
    pub easing_rot_rnd: i32,
    pub rot_axis_space: [f32; 3],
    pub angle_start: f32,
    pub angle_end: f32,
    pub easing_rot_space: i32,
    pub num_steps: i32,
}

/// The eight configurations the base points are ever in.
pub const NUM_ANIM_STATES: usize = 8;

/// Every animation there is, and the table that says which of them leads from
/// one configuration to another.
#[derive(Debug, Default)]
pub struct Animations {
    /// The phases, flattened; `phases` and `anims` index into these.
    pub multi: Vec<MultiObj>,
    /// One animation, as the phases it runs through in order.
    pub phases: Vec<Vec<usize>>,
    /// One set of animations to choose between.
    pub anims: Vec<Vec<usize>>,
    /// Which set gets from configuration `i` to configuration `j`.
    pub table: [[usize; NUM_ANIM_STATES]; NUM_ANIM_STATES],
}

/// Look a name up in a list of them, or panic: a name that is not there is a
/// bug in the converter rather than anything a saver could do about it.
fn index_of(names: &[String], want: &str, what: &str) -> usize {
    let found = names.iter().position(|n| n == want);
    assert!(found.is_some(), "the converter names no {what} {want}");
    found.unwrap_or_default()
}

impl Animations {
    /// Read the converted animation tables.
    pub fn parse(text: &str) -> Self {
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("HOPF1"), "not converted animations");

        let mut so_names: Vec<String> = Vec::new();
        let mut so_sets: Vec<Vec<SingleObj>> = Vec::new();
        let mut mo_names: Vec<String> = Vec::new();
        let mut ps_names: Vec<String> = Vec::new();
        let mut anim_names: Vec<String> = Vec::new();
        let mut this = Animations::default();

        while let Some(line) = lines.next() {
            let mut w = line.split_whitespace();
            let Some(kind) = w.next() else { continue };
            let name = w.next().unwrap_or_default().to_string();
            assert!(
                matches!(kind, "so" | "mo" | "ps" | "anims" | "table"),
                "unknown line {kind} in the animations"
            );
            match kind {
                "so" => {
                    let n: usize = w.next().and_then(|s| s.parse().ok()).expect("so count");
                    let mut set = Vec::with_capacity(n);
                    for _ in 0..n {
                        let f: Vec<f32> = lines
                            .next()
                            .expect("so row")
                            .split_whitespace()
                            .map(|s| s.parse().expect("so number"))
                            .collect();
                        assert_eq!(f.len(), 24, "so row is not 24 numbers");
                        set.push(SingleObj {
                            generator: f[0] as i32,
                            p_start: f[1],
                            p_end: f[2],
                            easing_p: f[3] as i32,
                            q_start: f[4],
                            q_end: f[5],
                            easing_q: f[6] as i32,
                            r_start: f[7],
                            r_end: f[8],
                            easing_r: f[9] as i32,
                            offset_start: f[10],
                            offset_end: f[11],
                            easing_offset: f[12] as i32,
                            sector_start: f[13],
                            sector_end: f[14],
                            easing_sector: f[15] as i32,
                            n: f[16] as i32,
                            num: f[17] as i32,
                            rot_axis_base: [f[18], f[19], f[20]],
                            angle_start: f[21],
                            angle_end: f[22],
                            easing_rotate: f[23] as i32,
                        });
                    }
                    so_names.push(name);
                    so_sets.push(set);
                }
                "mo" => {
                    let so = w.next().expect("mo object set");
                    let f: Vec<f32> = w.map(|s| s.parse().expect("mo number")).collect();
                    assert_eq!(f.len(), 9, "mo is not nine numbers");
                    this.multi.push(MultiObj {
                        so: so_sets[index_of(&so_names, so, "object set")].clone(),
                        rotate_prob: f[0],
                        easing_rot_rnd: f[1] as i32,
                        rot_axis_space: [f[2], f[3], f[4]],
                        angle_start: f[5],
                        angle_end: f[6],
                        easing_rot_space: f[7] as i32,
                        num_steps: f[8] as i32,
                    });
                    mo_names.push(name);
                }
                "ps" => {
                    this.phases
                        .push(w.map(|m| index_of(&mo_names, m, "phase")).collect());
                    ps_names.push(name);
                }
                "anims" => {
                    this.anims
                        .push(w.map(|p| index_of(&ps_names, p, "animation")).collect());
                    anim_names.push(name);
                }
                "table" => {
                    let all: Vec<usize> = std::iter::once(name.as_str())
                        .chain(w)
                        .map(|a| index_of(&anim_names, a, "set"))
                        .collect();
                    assert_eq!(all.len(), NUM_ANIM_STATES * NUM_ANIM_STATES, "table size");
                    for (i, row) in this.table.iter_mut().enumerate() {
                        row.copy_from_slice(&all[i * NUM_ANIM_STATES..(i + 1) * NUM_ANIM_STATES]);
                    }
                }
                _ => {}
            }
        }
        this
    }

    /// The most base points any one phase ever asks for, which is how many
    /// fibers have to be drawable at once.
    pub fn max_base_points(&self) -> usize {
        self.multi
            .iter()
            .map(|m| m.so.iter().map(|s| s.num.max(1) as usize).sum())
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANIMATIONS: &str = include_str!("../../data/hopfanimations.txt");

    #[test]
    fn a_fiber_is_a_closed_curve_inside_the_unit_ball() {
        // Every fiber but the one through the north pole projects to a closed
        // curve, and the whole of infinite space has been squeezed into the
        // ball, so nothing can be outside it.
        for base in [
            BasePoint {
                a: 1.0,
                b: 0.0,
                c: 0.0,
            },
            BasePoint {
                a: 0.0,
                b: 1.0,
                c: 0.0,
            },
            BasePoint {
                a: 0.0,
                b: 0.0,
                c: -1.0,
            },
            BasePoint {
                a: 0.6,
                b: 0.0,
                c: 0.8,
            },
            BasePoint {
                a: -0.36,
                b: 0.48,
                c: -0.8,
            },
        ] {
            let c = HopfCircle::new(base);
            assert!(!c.is_line());
            let pts = c.points(0.0005);
            assert!(pts.len() > 8, "{base:?} came out as {} points", pts.len());
            assert!(pts.len() <= MAX_CIRCLE_PNT);

            for p in &pts {
                assert!(norm(p.p) <= 1.0001, "{:?} is outside the ball", p.p);
                // The frame is orthonormal.
                for v in [p.t, p.n, p.b] {
                    assert!((norm(v) - 1.0).abs() < 1e-3, "{v:?} is not a unit vector");
                }
                assert!(dot(p.t, p.b).abs() < 1e-3);
                assert!(dot(p.n, p.b).abs() < 1e-3);
                assert!(dot(p.t, p.n).abs() < 1e-3);
            }
            // It closes: the last point is the first one again.
            let (a, b) = (pts[0].p, pts[pts.len() - 1].p);
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            assert!(d < 1e-3, "{base:?} does not close: {d}");
        }
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn the_subdivision_follows_the_curve_rather_than_the_parameter() {
        // Asking for a closer approximation gives more points, and the points
        // that come back really are within that distance of the curve.
        let c = HopfCircle::new(BasePoint {
            a: 0.6,
            b: 0.0,
            c: 0.8,
        });
        let coarse = c.points(0.01).len();
        let fine = c.points(0.0003).len();
        assert!(fine > coarse * 2, "{coarse} then {fine}");

        let pts = c.points(0.001);
        for w in pts.windows(2) {
            let mid = c.point(0.5 * (w[0].phi + w[1].phi));
            assert!(
                distance_point_line(mid.p, w[0].p, w[1].p) <= 0.001 + 1e-6,
                "a chord strays from the curve"
            );
        }
    }

    #[test]
    fn the_fiber_through_the_north_pole_is_a_straight_line() {
        let c = HopfCircle::new(BasePoint {
            a: 0.0,
            b: 0.0,
            c: 1.0,
        });
        assert!(c.is_line());
        let pts = c.points(0.0005);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].p, [0.0, 0.0, 1.0]);
        assert_eq!(pts[1].p, [0.0, 0.0, -1.0]);
    }

    #[test]
    fn every_fiber_is_linked_with_every_other_one() {
        // Two fibers from different base points form a Hopf link, so neither
        // of them can pass through the other's plane outside its own curve:
        // what that comes to here is that no two fibers ever touch.
        let a = HopfCircle::new(BasePoint {
            a: 1.0,
            b: 0.0,
            c: 0.0,
        })
        .points(0.001);
        let b = HopfCircle::new(BasePoint {
            a: 0.0,
            b: 1.0,
            c: 0.0,
        })
        .points(0.001);
        let mut closest = f32::MAX;
        for p in &a {
            for q in &b {
                let d = ((p.p[0] - q.p[0]).powi(2)
                    + (p.p[1] - q.p[1]).powi(2)
                    + (p.p[2] - q.p[2]).powi(2))
                .sqrt();
                closest = closest.min(d);
            }
        }
        assert!(closest > 0.05, "two fibers came within {closest}");
    }

    #[test]
    fn an_icosphere_is_a_sphere() {
        for s in [1, 2, 3, 12] {
            let sph = Icosphere::new(s, 0.2);
            assert_eq!(sph.vert.len(), s * s * 10 + 2, "s = {s}");
            assert_eq!(sph.tri.len(), s * s * 20, "s = {s}");
            for v in &sph.vert {
                assert!((norm(*v) - 0.2).abs() < 1e-5, "{v:?} is not on the sphere");
            }
            for n in &sph.norm {
                assert!((norm(*n) - 1.0).abs() < 1e-5);
            }
            // Every vertex is used, and no triangle names one twice.
            let mut used = vec![false; sph.vert.len()];
            for t in &sph.tri {
                assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2], "{t:?}");
                for &i in t {
                    used[i] = true;
                }
            }
            assert!(used.iter().all(|&u| u), "s = {s}: a vertex is unreachable");
        }
    }

    #[test]
    fn the_sphere_is_sorted_back_to_front_and_only_when_it_moves() {
        let mut sph = Icosphere::new(3, 0.2);
        let m = rotateall(30.0, 20.0, 10.0);
        assert!(sph.sort(&m), "the first sort has to happen");
        assert!(!sph.sort(&m), "the same matrix must not sort again");
        assert!(sph.sort(&rotateall(31.0, 20.0, 10.0)));

        // Back to front along the third row of the matrix.
        let m = rotateall(31.0, 20.0, 10.0);
        let z = |t: &[usize; 3]| -> f32 {
            t.iter()
                .map(|&i| {
                    let v = sph.vert[i];
                    m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2]
                })
                .sum()
        };
        for w in sph.stri.windows(2) {
            assert!(z(&w[0]) <= z(&w[1]) + 1e-6, "out of order");
        }
        assert_eq!(sph.stri.len(), sph.tri.len());
    }

    #[test]
    fn the_easing_functions_stay_between_nought_and_one() {
        for e in [
            EASING_CUBIC,
            EASING_SIN,
            EASING_COS,
            EASING_LIN,
            EASING_ACCEL,
            EASING_DECEL,
        ] {
            for i in 0..=100 {
                let v = ease(i as f32 / 100.0, e);
                assert!((-0.001..=1.001).contains(&v), "easing {e} reached {v}");
            }
        }

        // Four of them run from one end to the other, and a parameter eased
        // by one of those arrives where the table says it should.
        for e in [EASING_CUBIC, EASING_LIN, EASING_ACCEL, EASING_DECEL] {
            assert!(
                ease(0.0, e).abs() < 1e-6,
                "easing {e} starts at {}",
                ease(0.0, e)
            );
            assert!(
                (ease(1.0, e) - 1.0).abs() < 1e-6,
                "easing {e} ends at {}",
                ease(1.0, e)
            );
        }

        // The other two are waves rather than ramps, which is what a
        // parameter that goes out and comes back is written with: sine
        // starts halfway and cosine starts at the far end.
        for (t, want) in [(0.0, 0.5), (0.25, 1.0), (0.5, 0.5), (0.75, 0.0), (1.0, 0.5)] {
            assert!((ease(t, EASING_SIN) - want).abs() < 1e-5, "sine at {t}");
        }
        for (t, want) in [(0.0, 1.0), (0.5, 0.0), (1.0, 1.0)] {
            assert!((ease(t, EASING_COS) - want).abs() < 1e-5, "cosine at {t}");
        }

        // And the one that means "do not move" never leaves the start.
        for i in 0..=20 {
            assert_eq!(ease(i as f32 / 20.0, EASING_NONE), 0.0);
        }
    }

    #[test]
    fn a_circle_of_latitude_puts_its_points_on_one() {
        let mut out = Vec::new();
        gen_torus_base(
            &mut out,
            PI / 3.0,
            0.0,
            0.0,
            0,
            0.0,
            2.0 * PI,
            12,
            false,
            [1.0, 0.0, 0.0, 0.0],
        );
        assert_eq!(out.len(), 12);
        for p in &out {
            assert!((norm([p.a, p.b, p.c]) - 1.0).abs() < 1e-5);
            // All at the same height, which is what a circle of latitude is.
            assert!((p.c - (PI / 3.0f32).cos()).abs() < 1e-5);
        }
        // A sector of nought is a single point however many were asked for.
        out.clear();
        gen_torus_base(
            &mut out,
            PI / 3.0,
            0.0,
            0.0,
            0,
            0.0,
            0.0,
            12,
            false,
            [1.0, 0.0, 0.0, 0.0],
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_spiral_winds_from_one_pole_towards_the_other() {
        let mut out = Vec::new();
        gen_spiral_base(
            &mut out,
            0.0,
            2.0,
            6.0,
            0.0,
            PI / 2.0,
            40,
            false,
            [1.0, 0.0, 0.0, 0.0],
        );
        assert_eq!(out.len(), 40);
        for p in &out {
            assert!((norm([p.a, p.b, p.c]) - 1.0).abs() < 1e-5);
        }
        // It descends all the way, here from the equator to the south pole.
        for w in out.windows(2) {
            assert!(w[1].c <= w[0].c + 1e-6, "the spiral goes back up");
        }
        assert!(
            out[0].c.abs() < 1e-6 && out[39].c < -0.9,
            "{} to {}",
            out[0].c,
            out[39].c
        );
        // And it winds: the longitude goes round more than once.
        let turns: f32 = out
            .windows(2)
            .map(|w| {
                let a = w[0].b.atan2(w[0].a);
                let b = w[1].b.atan2(w[1].a);
                let d = b - a;
                if d > PI {
                    d - 2.0 * PI
                } else if d < -PI {
                    d + 2.0 * PI
                } else {
                    d
                }
            })
            .sum();
        assert!(turns.abs() > 2.0 * PI, "it only turned {turns} radians");
    }

    #[test]
    fn the_animation_tables_are_whole() {
        let a = Animations::parse(ANIMATIONS);
        assert_eq!(a.multi.len(), 188);
        assert_eq!(a.phases.len(), 133);
        assert_eq!(a.anims.len(), 64);

        // Every index in the table leads somewhere, and every set leads to
        // phases that lead to objects.
        for row in &a.table {
            for &s in row {
                let set = &a.anims[s];
                assert!(!set.is_empty());
                for &p in set {
                    let phases = &a.phases[p];
                    assert!(!phases.is_empty());
                    for &m in phases {
                        let mo = &a.multi[m];
                        assert!(!mo.so.is_empty());
                        assert!(
                            mo.num_steps > 0,
                            "a phase with no steps would divide by nought"
                        );
                    }
                }
            }
        }
        // The diagonal is the animations that stay in one configuration.
        for i in 0..NUM_ANIM_STATES {
            assert!(!a.anims[a.table[i][i]].is_empty());
        }
        assert_eq!(a.max_base_points(), 216);
    }

    #[test]
    fn the_first_animation_is_the_one_upstream_writes_first() {
        // A spot check that the converter kept the numbers: the first entry
        // turns a single point on the equator once around the z axis over a
        // hundred and eighty steps.
        let a = Animations::parse(ANIMATIONS);
        let m = &a.multi[0];
        assert_eq!(m.num_steps, 180);
        assert_eq!(m.so.len(), 1);
        let s = m.so[0];
        assert_eq!(s.generator, GEN_TORUS);
        assert!((s.p_start - PI / 2.0).abs() < 1e-6);
        assert!((s.p_end - PI / 2.0).abs() < 1e-6);
        assert_eq!(s.num, 1);
        assert_eq!(s.rot_axis_base, [0.0, 0.0, 1.0]);
        assert_eq!(s.angle_start, 0.0);
        assert!((s.angle_end - 2.0 * PI).abs() < 1e-5);
        assert_eq!(s.easing_rotate, EASING_CUBIC);
    }
}

#[cfg(test)]
mod cost {
    use super::*;

    const ANIMATIONS: &str = include_str!("../../data/hopfanimations.txt");

    /// How many vertices the heaviest animation is worth at one detail level.
    fn vertices(m: &MultiObj, num_tube: usize, max_dist: f32) -> usize {
        let mut base = Vec::new();
        for s in &m.so {
            if s.generator == GEN_TORUS {
                gen_torus_base(
                    &mut base,
                    s.p_start,
                    s.q_start,
                    s.r_start,
                    s.n,
                    s.offset_start,
                    s.sector_start,
                    s.num,
                    false,
                    [1.0, 0.0, 0.0, 0.0],
                );
            } else {
                gen_spiral_base(
                    &mut base,
                    s.p_start,
                    s.q_start,
                    s.r_start,
                    s.offset_start,
                    s.sector_start,
                    s.num,
                    false,
                    [1.0, 0.0, 0.0, 0.0],
                );
            }
        }
        // A tube is swept along each fiber: one ring of `num_tube` quads
        // between each pair of points along it, and a quad is two triangles.
        let points: usize = base
            .iter()
            .map(|b| HopfCircle::new(*b).points(max_dist).len())
            .sum();
        (points - base.len()) * num_tube * 6
    }

    #[test]
    fn the_heaviest_animation_is_the_one_that_sets_the_detail_default() {
        // The geometry moves every frame, so it is rebuilt every frame,
        // which is what upstream does too. How much of it there is decides
        // which detail level the saver defaults to, and the number that
        // matters is the middle of the distribution rather than the top of
        // it: the heaviest animation is four of a hundred and eighty-eight.
        let a = Animations::parse(ANIMATIONS);
        let heaviest = a
            .multi
            .iter()
            .max_by_key(|m| m.so.iter().map(|s| s.num.max(1)).sum::<i32>())
            .expect("there is at least one animation");
        let points: i32 = heaviest.so.iter().map(|s| s.num.max(1)).sum();
        assert_eq!(points, 216);

        // Coarse, medium and fine, as upstream's knob offers them.
        let coarse = vertices(heaviest, 8, 0.0010);
        let medium = vertices(heaviest, 12, 0.0005);
        let fine = vertices(heaviest, 16, 0.0003);
        assert!(coarse > 700_000, "{coarse}");
        assert!(medium > 1_500_000, "{medium}");
        assert!(fine > 2_500_000, "{fine}");

        // And the middle of the distribution at coarse, which is where the
        // saver actually spends its time: a third of the worst case, and
        // about what the runtime draws elsewhere without trouble.
        let mut all: Vec<usize> = a.multi.iter().map(|m| vertices(m, 8, 0.0010)).collect();
        all.sort_unstable();
        let median = all[all.len() / 2];
        let upper = all[all.len() * 3 / 4];
        assert!(median < 300_000, "the median animation is {median}");
        assert!(upper < 400_000, "the upper quartile is {upper}");
        assert!(coarse < medium && medium < fine);
    }
}
