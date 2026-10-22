/* cubocteversion --- Shows a cuboctahedron eversion, i.e., a smooth
deformation (homotopy) that turns a cuboctahedron inside out.  During the
eversion, the deformed cuboctahedron is allowed to intersect itself
transversally.  However, no fold edges or non-injective neighborhoods of
vertices are allowed to occur. */

/* Copyright (c) 2023-2026 Carsten Steger <carsten@mirsanmir.org>. */

/*
 * Permission to use, copy, modify, and distribute this software and its
 * documentation for any purpose and without fee is hereby granted,
 * provided that the above copyright notice appear in all copies and that
 * both that copyright notice and this permission notice appear in
 * supporting documentation.
 *
 * This file is provided AS IS with no warranties of any kind.  The author
 * shall have no liability with respect to the infringement of copyrights,
 * trade secrets or any patents by this file or any part thereof.  In no
 * event will the author be liable for any lost revenue or profits or
 * other special, indirect and consequential damages.
 */

//! Port of `hacks/glx/cubocteversion.c`.
//!
//! Turns a cuboctahedron inside out: a smooth deformation (homotopy). It is
//! `sphereeversion` done in straight lines. Where that one bends a smooth
//! surface through itself, this one keeps twelve vertices, thirty edges and
//! twenty flat triangles the whole way, and everts by moving the vertices
//! along straight lines from one polyhedron to the next.
//!
//! So the eversion *is* a table. Richard Denner and Francois Apery each worked
//! out a sequence of polyhedra, forty-five and seven of them, and every frame
//! is the interpolation between two neighbours in one of those sequences,
//! eased so the corners do not show. [`crate::hacks3d::cubocteversion_models`]
//! is those tables.
//!
//! What is not a table is where the surface passes through itself. That is
//! found rather than modelled: every one of the hundred and ninety pairs of
//! non-adjacent triangles is intersected against every other, each frame, by
//! Devillers and Guigue's predicate, and the segments that come back are drawn
//! as orange tubes. The white tubes along the edges are the same machinery.
//!
//! Upstream draws it two ways, and unlike `timetunnel` its fixed-function path
//! is the whole saver, so that is what is ported. Two things live only in the
//! shader path and are noted where they arise: the earth colouring, and the
//! transparency knob, which chose between two depth-peeling schemes that the
//! fixed-function path has no use for.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Mat4, Shape};
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

use super::cubocteversion_models::{
    EDGE, FACE, NUM_EDGE, NUM_FACE, NUM_VERT, VERT_APERY, VERT_DENNER,
};

/// Number of frames the finished eversion is swiped off the screen over.
const NUM_SWIPE: i32 = 100;

/// Number of subdivisions for spheres and cylinders.
const NUMU: usize = 16;
const NUMV: usize = 16;

/// Radii of the edge and self-intersection tubes.
const RADIUS_TUBE: f32 = 0.020;
const RADIUS_SELF: f32 = 0.019;

/// Minimum and maximum FOV angle for perspective projection.
const MIN_FOV_ANGLE: f32 = 20.0;
const MAX_FOV_ANGLE: f32 = 60.0;

/// Minimum and maximum FOV for orthographic projection.
const MIN_FOV_ORTHO: f32 = 2.0;
const MAX_FOV_ORTHO: f32 = 6.0;

/// Minimum and maximum opacity of transparent faces.
const MIN_OPACITY: f32 = 0.3;
const MAX_OPACITY: f32 = 0.7;

/// Minimum and maximum time for the deformation.
const TIME_MIN: f32 = -56.0;
const TIME_MAX: f32 = 56.0;

/// Upstream's `M_SQRT2_F`. Its literal and this constant are the same number
/// once rounded to a `f32`, which is why clippy asks for the constant.
const M_SQRT2_F: f32 = std::f32::consts::SQRT_2;
const M_SQRT3_F: f32 = 1.732_050_8;
const M_SQRT6_F: f32 = 2.449_489_8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Apery,
    MorinDenner,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayMode {
    Surface,
    Transparent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Colors {
    TwoSided,
    Face,
    /// Upstream wraps the earth around it in a fragment shader. Its own
    /// fixed-function path leaves the faces white, and so does this.
    Earth,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Projection {
    Perspective,
    Orthographic,
}

/// Where in the loop the animation is: deforming, or being swiped off the
/// screen and back on again between eversions.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimState {
    Deform,
    SwipeDown,
    SwipeUp,
}

/* -------------------------------------------------------------------------
 * Triangle against triangle
 *
 * Devillers and Guigue's predicate, as upstream carries it: whether two
 * triangles in space meet, and if so along which segment.
 * ---------------------------------------------------------------------- */

fn sub(v1: [f64; 3], v2: [f64; 3]) -> [f64; 3] {
    [v1[0] - v2[0], v1[1] - v2[1], v1[2] - v2[2]]
}

fn cross(v1: [f64; 3], v2: [f64; 3]) -> [f64; 3] {
    [
        v1[1] * v2[2] - v1[2] * v2[1],
        v1[2] * v2[0] - v1[0] * v2[2],
        v1[0] * v2[1] - v1[1] * v2[0],
    ]
}

fn dot(v1: [f64; 3], v2: [f64; 3]) -> f64 {
    v1[0] * v2[0] + v1[1] * v2[1] + v1[2] * v2[2]
}

fn scalar(alpha: f64, v: [f64; 3]) -> [f64; 3] {
    [alpha * v[0], alpha * v[1], alpha * v[2]]
}

/// The segment two triangles meet along, once it is known that they do.
#[allow(clippy::too_many_arguments)]
fn construct_intersection(
    p1: [f64; 3],
    q1: [f64; 3],
    r1: [f64; 3],
    p2: [f64; 3],
    q2: [f64; 3],
    r2: [f64; 3],
    n1: [f64; 3],
    n2: [f64; 3],
) -> Option<([f64; 3], [f64; 3])> {
    // `along(a, b, c, n)` is the point where the segment from `a` towards `c`
    // crosses the plane with normal `n` through `b`: upstream writes the same
    // four lines out eight times.
    let along = |a: [f64; 3], b: [f64; 3], c: [f64; 3], n: [f64; 3]| {
        let v1 = sub(a, b);
        let v2 = sub(a, c);
        sub(a, scalar(dot(v1, n) / dot(v2, n), v2))
    };

    let mut v1 = sub(q1, p1);
    let v2 = sub(r2, p1);
    let mut n = cross(v1, v2);
    let v = sub(p2, p1);
    if dot(v, n) > 0.0 {
        v1 = sub(r1, p1);
        n = cross(v1, v2);
        if dot(v, n) > 0.0 {
            return None;
        }
        let v2 = sub(q2, p1);
        n = cross(v1, v2);
        if dot(v, n) > 0.0 {
            Some((along(p1, p2, r1, n2), along(p2, p1, r2, n1)))
        } else {
            Some((along(p2, p1, q2, n1), along(p2, p1, r2, n1)))
        }
    } else {
        let v2 = sub(q2, p1);
        n = cross(v1, v2);
        if dot(v, n) < 0.0 {
            return None;
        }
        let v1 = sub(r1, p1);
        n = cross(v1, v2);
        if dot(v, n) >= 0.0 {
            Some((along(p1, p2, r1, n2), along(p1, p2, q1, n2)))
        } else {
            Some((along(p2, p1, q2, n1), along(p1, p2, q1, n2)))
        }
    }
}

/// Which of the six permutations of the second triangle to hand on, from the
/// signs of its vertices against the first triangle's plane.
#[allow(clippy::too_many_arguments)]
fn tri_tri_inter_3d(
    p1: [f64; 3],
    q1: [f64; 3],
    r1: [f64; 3],
    p2: [f64; 3],
    q2: [f64; 3],
    r2: [f64; 3],
    dp2: f64,
    dq2: f64,
    dr2: f64,
    n1: [f64; 3],
    n2: [f64; 3],
) -> Option<([f64; 3], [f64; 3])> {
    let go = |a, b, c, d, e, f| construct_intersection(a, b, c, d, e, f, n1, n2);
    if dp2 > 0.0 {
        if dq2 > 0.0 {
            go(p1, r1, q1, r2, p2, q2)
        } else if dr2 > 0.0 {
            go(p1, r1, q1, q2, r2, p2)
        } else {
            go(p1, q1, r1, p2, q2, r2)
        }
    } else if dp2 < 0.0 {
        if dq2 < 0.0 {
            go(p1, q1, r1, r2, p2, q2)
        } else if dr2 < 0.0 {
            go(p1, q1, r1, q2, r2, p2)
        } else {
            go(p1, r1, q1, p2, q2, r2)
        }
    } else if dq2 < 0.0 {
        if dr2 >= 0.0 {
            go(p1, r1, q1, q2, r2, p2)
        } else {
            go(p1, q1, r1, p2, q2, r2)
        }
    } else if dq2 > 0.0 {
        if dr2 > 0.0 {
            go(p1, r1, q1, p2, q2, r2)
        } else {
            go(p1, q1, r1, q2, r2, p2)
        }
    } else if dr2 > 0.0 {
        go(p1, q1, r1, r2, p2, q2)
    } else if dr2 < 0.0 {
        go(p1, r1, q1, r2, p2, q2)
    } else {
        None
    }
}

/// Where two triangles in space meet, or nothing if they do not.
fn tri_tri_intersection_3d(
    p1: [f64; 3],
    q1: [f64; 3],
    r1: [f64; 3],
    p2: [f64; 3],
    q2: [f64; 3],
    r2: [f64; 3],
) -> Option<([f64; 3], [f64; 3])> {
    let n2 = cross(sub(p2, r2), sub(q2, r2));
    let dp1 = dot(sub(p1, r2), n2);
    let dq1 = dot(sub(q1, r2), n2);
    let dr1 = dot(sub(r1, r2), n2);
    if dp1 * dq1 > 0.0 && dp1 * dr1 > 0.0 {
        return None;
    }

    let n1 = cross(sub(q1, p1), sub(r1, p1));
    let dp2 = dot(sub(p2, r1), n1);
    let dq2 = dot(sub(q2, r1), n1);
    let dr2 = dot(sub(r2, r1), n1);
    if dp2 * dq2 > 0.0 && dp2 * dr2 > 0.0 {
        return None;
    }

    let go = |a, b, c, d, e, f, x, y, z| tri_tri_inter_3d(a, b, c, d, e, f, x, y, z, n1, n2);
    if dp1 > 0.0 {
        if dq1 > 0.0 {
            go(r1, p1, q1, p2, r2, q2, dp2, dr2, dq2)
        } else if dr1 > 0.0 {
            go(q1, r1, p1, p2, r2, q2, dp2, dr2, dq2)
        } else {
            go(p1, q1, r1, p2, q2, r2, dp2, dq2, dr2)
        }
    } else if dp1 < 0.0 {
        if dq1 < 0.0 {
            go(r1, p1, q1, p2, q2, r2, dp2, dq2, dr2)
        } else if dr1 < 0.0 {
            go(q1, r1, p1, p2, q2, r2, dp2, dq2, dr2)
        } else {
            go(p1, q1, r1, p2, r2, q2, dp2, dr2, dq2)
        }
    } else if dq1 < 0.0 {
        if dr1 >= 0.0 {
            go(q1, r1, p1, p2, r2, q2, dp2, dr2, dq2)
        } else {
            go(p1, q1, r1, p2, q2, r2, dp2, dq2, dr2)
        }
    } else if dq1 > 0.0 {
        if dr1 > 0.0 {
            go(p1, q1, r1, p2, r2, q2, dp2, dr2, dq2)
        } else {
            go(q1, r1, p1, p2, q2, r2, dp2, dq2, dr2)
        }
    } else if dr1 > 0.0 {
        go(r1, p1, q1, p2, q2, r2, dp2, dq2, dr2)
    } else if dr1 < 0.0 {
        go(r1, p1, q1, p2, r2, q2, dp2, dr2, dq2)
    } else {
        None
    }
}

/* -------------------------------------------------------------------------
 * Tubes
 * ---------------------------------------------------------------------- */

/// Several triangle strips joined into one.
///
/// Upstream keeps an index buffer and an offset and a length per strip, and
/// draws each with its own call: two hundred and twenty two of them for the
/// edges alone, and up to a thousand more for the self-intersections. A strip
/// cannot merge with the strip beside it here, so they are joined instead,
/// with the usual pair of repeated vertices between them. The two triangles
/// that makes have no area and raster to nothing, and every strip is an even
/// number of vertices long, so the winding of what follows a join is
/// unchanged.
#[derive(Default)]
struct Strip {
    verts: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    /// A join is owed: the next vertex pushed goes in twice.
    doubling: bool,
}

impl Strip {
    fn clear(&mut self) {
        self.verts.clear();
        self.normals.clear();
        self.doubling = false;
    }

    /// Begin another strip inside this one.
    fn begin(&mut self) {
        if let (Some(&v), Some(&n)) = (self.verts.last(), self.normals.last()) {
            self.verts.push(v);
            self.normals.push(n);
            self.doubling = true;
        }
    }

    fn push(&mut self, v: [f32; 3], n: [f32; 3]) {
        if self.doubling {
            self.verts.push(v);
            self.normals.push(n);
            self.doubling = false;
        }
        self.verts.push(v);
        self.normals.push(n);
    }
}

/// `gen_sphere`: a ball at a vertex, as `num_u` strips around it.
fn gen_sphere(c: [f32; 3], r: f32, strip: &mut Strip) {
    let mut ring = vec![[0.0f32; 3]; (NUMV + 1) * NUMU];
    for (i, chunk) in ring.chunks_exact_mut(NUMV + 1).enumerate() {
        let u = i as f32 * std::f32::consts::TAU / NUMU as f32;
        let (su, cu) = u.sin_cos();
        for (j, p) in chunk.iter_mut().enumerate() {
            let v = j as f32 * std::f32::consts::PI / NUMV as f32;
            let (sv, cv) = v.sin_cos();
            *p = [sv * cu, sv * su, cv];
        }
    }
    for i in 0..NUMU {
        let k = (i + 1) % NUMU;
        strip.begin();
        for j in 0..=NUMV {
            for m in [i, k] {
                let p = ring[m * (NUMV + 1) + j];
                strip.push(
                    [c[0] + r * p[0], c[1] + r * p[1], c[2] + r * p[2]],
                    [-p[0], -p[1], -p[2]],
                );
            }
        }
    }
}

/// `gen_cylinder`: a tube along an edge. Its ends are left open, because a
/// ball sits at each of them.
fn gen_cylinder(c: [f32; 3], a: [f32; 3], length: f32, r: f32, strip: &mut Strip) {
    let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if len < 10.0 * f32::EPSILON || length < 10.0 * f32::EPSILON {
        return;
    }
    let a = [a[0] / len, a[1] / len, a[2] / len];
    if a[2].abs() >= 1.0 {
        /* The case az = +/-1 does not occur in this program. */
        return;
    }
    let l = 1.0 / (a[1] * a[1] + a[0] * a[0]).sqrt();
    let e = [a[1] * l, -a[0] * l, 0.0];
    let f = [
        a[1] * e[2] - a[2] * e[1],
        a[2] * e[0] - a[0] * e[2],
        a[0] * e[1] - a[1] * e[0],
    ];

    strip.begin();
    for j in 0..=NUMV {
        let v = j as f32 * std::f32::consts::TAU / NUMV as f32;
        let (sv, cv) = v.sin_cos();
        let n = [
            e[0] * cv + f[0] * sv,
            e[1] * cv + f[1] * sv,
            e[2] * cv + f[2] * sv,
        ];
        for i in 0..=1 {
            let u = (2 * i - 1) as f32;
            strip.push(
                [
                    c[0] + u * length * a[0] + r * n[0],
                    c[1] + u * length * a[1] + r * n[1],
                    c[2] + u * length * a[2] + r * n[2],
                ],
                [-n[0], -n[1], -n[2]],
            );
        }
    }
}

/* -------------------------------------------------------------------------
 * The saver
 * ---------------------------------------------------------------------- */

fn barycenter(x: [f32; 3], y: [f32; 3], z: [f32; 3]) -> [f32; 3] {
    [
        (x[0] + y[0] + z[0]) / 3.0,
        (x[1] + y[1] + z[1]) / 3.0,
        (x[2] + y[2] + z[2]) / 3.0,
    ]
}

fn unit_normal(x: [f32; 3], y: [f32; 3], z: [f32; 3]) -> [f32; 3] {
    let a = [y[0] - x[0], y[1] - x[1], y[2] - x[2]];
    let b = [z[0] - x[0], z[1] - x[1], z[2] - x[2]];
    let mut n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l > 0.0 {
        for v in &mut n {
            *v /= l;
        }
        n
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Add a rotation around the x-axis to the matrix m.
fn rotatex(m: &mut [[f32; 3]; 3], phi: f32) {
    let (s, c) = phi.to_radians().sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[1], row[2]);
        row[1] = c * u + s * v;
        row[2] = -s * u + c * v;
    }
}

/// Add a rotation around the y-axis to the matrix m.
fn rotatey(m: &mut [[f32; 3]; 3], phi: f32) {
    let (s, c) = phi.to_radians().sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[0], row[2]);
        row[0] = c * u - s * v;
        row[2] = s * u + c * v;
    }
}

/// Add a rotation around the z-axis to the matrix m.
fn rotatez(m: &mut [[f32; 3]; 3], phi: f32) {
    let (s, c) = phi.to_radians().sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[0], row[1]);
        row[0] = c * u + s * v;
        row[1] = -s * u + c * v;
    }
}

/// Compute the rotation matrix from the rotation angles.
fn rotateall(al: f32, be: f32, de: f32) -> [[f32; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    rotatex(&mut m, al);
    rotatey(&mut m, be);
    rotatez(&mut m, de);
    m
}

/// Compute a rotation matrix from an xscreensaver unit quaternion. Note that
/// xscreensaver has a different convention for unit quaternions than the one
/// that is used in this hack.
fn quat_to_rotmat(p: [f32; 4]) -> Mat4 {
    let r00 = 1.0 - 2.0 * (p[1] * p[1] + p[2] * p[2]);
    let r01 = 2.0 * (p[0] * p[1] + p[2] * p[3]);
    let r02 = 2.0 * (p[2] * p[0] - p[1] * p[3]);
    let r12 = 2.0 * (p[1] * p[2] + p[0] * p[3]);
    let r22 = 1.0 - 2.0 * (p[1] * p[1] + p[0] * p[0]);
    let al = (-r12).atan2(r22).to_degrees();
    let be = r02.atan2((r00 * r00 + r01 * r01).sqrt()).to_degrees();
    let de = (-r01).atan2(r00).to_degrees();
    let m = rotateall(al, be, de);
    // Column major, and upstream's `m[i][j]` is row i column j.
    Mat4([
        m[0][0], m[1][0], m[2][0], 0.0, //
        m[0][1], m[1][1], m[2][1], 0.0, //
        m[0][2], m[1][2], m[2][2], 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ])
}

const LIGHT_MODEL_AMBIENT: [f32; 4] = [0.2, 0.2, 0.2, 1.0];
const MAT_DIFF_MAGENTA: [f32; 4] = [1.0, 0.0, 0.5, 1.0];
const MAT_DIFF_CYAN: [f32; 4] = [0.0, 0.5, 1.0, 1.0];
const MAT_DIFF_TUBE: [f32; 4] = [0.7, 0.7, 0.7, 1.0];
const MAT_DIFF_SELF: [f32; 4] = [1.0, 0.5, 0.0, 1.0];

struct CubocteversionState {
    method: Method,
    random_method: bool,
    display_mode: DisplayMode,
    random_display_mode: bool,
    edge_tubes: bool,
    random_edge_tubes: bool,
    self_tubes: bool,
    random_self_tubes: bool,
    colors: Colors,
    random_colors: bool,
    projection: Projection,
    random_projection: bool,

    /* 3D rotation angles */
    alpha: f32,
    beta: f32,
    delta: f32,
    anim_state: AnimState,
    time: f32,
    defdir: f32,
    swipe_step: i32,
    swipe_y: f32,
    offset3d: [f32; 3],
    fov_angle: f32,
    fov_ortho: f32,
    opacity: f32,

    /* The coordinates and normals of the faces */
    tt: Vec<[f32; 3]>,
    tn: Vec<[f32; 3]>,
    /* The coordinates of the edges and vertices */
    te: Vec<[f32; 3]>,
    tv: Vec<[f32; 3]>,
    /* The face colours */
    tc: Vec<[f32; 4]>,
    /* The self-intersection segments and their endpoints */
    se: Vec<([f32; 3], [f32; 3])>,
    sv: Vec<[f32; 3]>,

    tube: Strip,
    self_tube: Strip,

    aspect: f32,
    trackball: Trackball,
    speed_scale: f32,
    speed_x: f32,
    speed_y: f32,
    speed_z: f32,
    deform_speed: f32,
}

impl CubocteversionState {
    /// `setup_colors`: one colour per face, from where its middle sits on the
    /// undeformed cuboctahedron, so the colouring names the face rather than
    /// following the deformation.
    fn setup_colors(&mut self) {
        const MATC: [[f32; 3]; 3] = [
            [2.0 / M_SQRT6_F, 0.0, 1.0 / M_SQRT3_F],
            [-1.0 / M_SQRT6_F, 1.0 / M_SQRT2_F, 1.0 / M_SQRT3_F],
            [-1.0 / M_SQRT6_F, -1.0 / M_SQRT2_F, 1.0 / M_SQRT3_F],
        ];
        let alpha = if self.display_mode == DisplayMode::Transparent {
            self.opacity
        } else {
            1.0
        };
        if self.colors != Colors::Face {
            self.tc.fill([1.0, 1.0, 1.0, alpha]);
            return;
        }
        let v = &VERT_APERY[0];
        for i in 0..NUM_FACE {
            let mut b = barycenter(v[FACE[i][0]], v[FACE[i][1]], v[FACE[i][2]]);
            b[0] /= 2.0 * M_SQRT6_F;
            b[1] /= 2.0 * M_SQRT6_F;
            b[2] = (b[2] + (M_SQRT6_F - M_SQRT2_F)) * (M_SQRT3_F / (2.0 * M_SQRT6_F));
            let mut c = [0.0f32; 4];
            for (j, cj) in c.iter_mut().take(3).enumerate() {
                for k in 0..3 {
                    *cj += MATC[j][k] * b[k];
                }
            }
            c[3] = alpha;
            for j in 0..3 {
                self.tc[3 * i + j] = c;
            }
        }
    }

    /// `generate_geometry`: where every vertex is at this instant, and where
    /// the surface passes through itself.
    fn generate_geometry(&mut self) {
        // Which two of the tabulated polyhedra this instant lies between, and
        // how far. The middle of the deformation runs at one rate and the two
        // ends, which are only there to show the finished shape, at another.
        let (models, t, self_inter_possible): (&[[[f32; 3]; NUM_VERT]], f32, bool) =
            if self.method == Method::Apery {
                let (t, s) = if self.time.abs() <= 28.0 {
                    (self.time / 28.0, true)
                } else if self.time.abs() <= 35.0 {
                    (
                        if self.time <= 0.0 {
                            (self.time + 21.0) / 7.0
                        } else {
                            (self.time - 21.0) / 7.0
                        },
                        false,
                    )
                } else if self.time <= 0.0 {
                    ((self.time - 7.0) / 21.0, false)
                } else {
                    ((self.time + 7.0) / 21.0, false)
                };
                (&VERT_APERY, t, s)
            } else {
                let (t, s) = if self.time.abs() <= 24.0 {
                    (self.time / 4.0, true)
                } else if self.time <= 0.0 {
                    ((self.time + 12.0) / 2.0, false)
                } else {
                    ((self.time - 12.0) / 2.0, false)
                };
                (&VERT_DENNER, t, s)
            };
        let (limit, base) = if self.method == Method::Apery {
            (2.0, 3.0)
        } else {
            (21.0, 22.0)
        };
        let tf = t.floor().min(limit);
        let t1 = ease(tf + 1.0 - t);
        let t2 = 1.0 - t1;
        let m = (tf + base) as usize;
        let (v1, v2) = (&models[m], &models[m + 1]);

        let mut v = [[0.0f32; 3]; NUM_VERT];
        for i in 0..NUM_VERT {
            for j in 0..3 {
                v[i][j] = t1 * v1[i][j] + t2 * v2[i][j];
            }
        }

        for i in 0..NUM_FACE {
            let n = unit_normal(v[FACE[i][0]], v[FACE[i][1]], v[FACE[i][2]]);
            for j in 0..3 {
                self.tt[3 * i + j] = v[FACE[i][j]];
                self.tn[3 * i + j] = n;
            }
        }
        for i in 0..NUM_EDGE {
            for j in 0..2 {
                self.te[2 * i + j] = v[EDGE[i][j]];
            }
        }
        self.tv[..NUM_VERT].copy_from_slice(&v);

        self.se.clear();
        self.sv.clear();
        if !(self_inter_possible && self.self_tubes) {
            return;
        }
        for (i, f1) in FACE.iter().enumerate() {
            for f2 in FACE.iter().skip(i + 1) {
                /* Check if the two triangles have at least one point in
                common. */
                if f1.iter().any(|a| f2.contains(a)) {
                    continue;
                }
                let p = |f: &[usize; 3], k: usize| {
                    let x = v[f[k]];
                    [f64::from(x[0]), f64::from(x[1]), f64::from(x[2])]
                };
                let Some((s1, s2)) = tri_tri_intersection_3d(
                    p(f1, 0),
                    p(f1, 1),
                    p(f1, 2),
                    p(f2, 0),
                    p(f2, 1),
                    p(f2, 2),
                ) else {
                    continue;
                };
                let a = [s1[0] as f32, s1[1] as f32, s1[2] as f32];
                let b = [s2[0] as f32, s2[1] as f32, s2[2] as f32];
                self.se.push((a, b));
                // A vertex where three or more sheets meet comes back from
                // several pairs of triangles, and only wants one ball on it.
                for q1 in [a, b] {
                    let present = self.sv.iter().any(|q2| {
                        let e = [q2[0] - q1[0], q2[1] - q1[1], q2[2] - q1[2]];
                        (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt() <= 100.0 * f32::EPSILON
                    });
                    if !present {
                        self.sv.push(q1);
                    }
                }
            }
        }
    }

    /// `gen_vertex_edge_tube` and `gen_self_intersection_tube`.
    fn generate_tubes(&mut self) {
        self.tube.clear();
        if self.edge_tubes {
            for i in 0..NUM_VERT {
                gen_sphere(self.tv[i], RADIUS_TUBE, &mut self.tube);
            }
            for i in 0..NUM_EDGE {
                let (a, b) = (self.te[2 * i], self.te[2 * i + 1]);
                tube_between(a, b, RADIUS_TUBE, &mut self.tube);
            }
        }
        self.self_tube.clear();
        if self.self_tubes {
            for i in 0..self.sv.len() {
                gen_sphere(self.sv[i], RADIUS_SELF, &mut self.self_tube);
            }
            for i in 0..self.se.len() {
                let (a, b) = self.se[i];
                tube_between(a, b, RADIUS_SELF, &mut self.self_tube);
            }
        }
    }

    /// `display_cubocteversion`: move the animation on one step.
    fn animate(&mut self) {
        if self.trackball.button_down() {
            return;
        }
        match self.anim_state {
            AnimState::Deform => {
                self.time += self.defdir * self.deform_speed * 0.005;
                if self.time < TIME_MIN {
                    self.time = TIME_MIN;
                    self.defdir = -self.defdir;
                    self.anim_state = AnimState::SwipeDown;
                }
                if self.time > TIME_MAX {
                    self.time = TIME_MAX;
                    self.defdir = -self.defdir;
                    self.anim_state = AnimState::SwipeDown;
                }
                if self.anim_state == AnimState::SwipeDown {
                    self.swipe_step = 0;
                    self.swipe_y = self.fov_extent() + 2.5;
                }
            }
            AnimState::SwipeDown => {
                self.swipe_step += 1;
                let t = 2.0 * ease(0.5 * self.swipe_step as f32 / NUM_SWIPE as f32);
                self.offset3d[1] = -t * self.swipe_y;
                if self.swipe_step > NUM_SWIPE {
                    self.anim_state = AnimState::SwipeUp;
                }
            }
            AnimState::SwipeUp => {
                if self.swipe_step > NUM_SWIPE {
                    // The next eversion is a fresh draw of every knob left on
                    // random, chosen while it is off screen.
                    self.alpha = frand(120.0) as f32 - 60.0;
                    self.beta = frand(120.0) as f32 - 60.0;
                    self.delta = frand(360.0) as f32;
                    self.fov_angle =
                        frand(f64::from(MAX_FOV_ANGLE - MIN_FOV_ANGLE)) as f32 + MIN_FOV_ANGLE;
                    self.fov_ortho =
                        frand(f64::from(MAX_FOV_ORTHO - MIN_FOV_ORTHO)) as f32 + MIN_FOV_ORTHO;
                    self.opacity = frand(f64::from(MAX_OPACITY - MIN_OPACITY)) as f32 + MIN_OPACITY;
                    if self.random_method {
                        self.method = if random().is_multiple_of(2) {
                            Method::Apery
                        } else {
                            Method::MorinDenner
                        };
                    }
                    if self.random_display_mode {
                        self.display_mode = if random().is_multiple_of(2) {
                            DisplayMode::Surface
                        } else {
                            DisplayMode::Transparent
                        };
                    }
                    if self.random_edge_tubes {
                        self.edge_tubes = random() & 1 != 0;
                    }
                    if self.random_self_tubes {
                        self.self_tubes = random() & 1 != 0;
                    }
                    if self.random_colors {
                        // Upstream draws only two of the three colourings at
                        // random without a shader, because the third is the
                        // earth and there is no earth to put on it.
                        self.colors = if random().is_multiple_of(2) {
                            Colors::TwoSided
                        } else {
                            Colors::Face
                        };
                    }
                    if self.random_projection {
                        self.projection = if random().is_multiple_of(2) {
                            Projection::Perspective
                        } else {
                            Projection::Orthographic
                        };
                    }
                    self.swipe_y = self.fov_extent() + 3.0;
                    self.setup_colors();
                }
                self.swipe_step -= 1;
                let t = 2.0 * ease(0.5 * self.swipe_step as f32 / NUM_SWIPE as f32);
                self.offset3d[1] = t * self.swipe_y;
                if self.swipe_step < 0 {
                    self.anim_state = AnimState::Deform;
                }
            }
        }

        self.alpha += self.speed_x * self.speed_scale;
        if self.alpha >= 360.0 {
            self.alpha -= 360.0;
        }
        self.beta += self.speed_y * self.speed_scale;
        if self.beta >= 360.0 {
            self.beta -= 360.0;
        }
        self.delta += self.speed_z * self.speed_scale;
        if self.delta >= 360.0 {
            self.delta -= 360.0;
        }
    }

    /// How far off the middle of the screen the shape has to go to be gone.
    fn fov_extent(&self) -> f32 {
        if self.projection == Projection::Perspective {
            0.1 * self.fov_angle
        } else {
            self.fov_ortho
        }
    }

    fn draw_strip(&self, g: &mut Gl, strip: &Strip, color: [f32; 4]) {
        if strip.verts.is_empty() {
            return;
        }
        g.glx.material_ambient_diffuse(color);
        g.glx.begin(Shape::TriangleStrip);
        for (v, n) in strip.verts.iter().zip(&strip.normals) {
            g.glx.normal3f(n[0], n[1], n[2]);
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();
    }
}

/// A cylinder from one point to another.
fn tube_between(a: [f32; 3], b: [f32; 3], r: f32, strip: &mut Strip) {
    let c = [
        0.5 * (a[0] + b[0]),
        0.5 * (a[1] + b[1]),
        0.5 * (a[2] + b[2]),
    ];
    let ax = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len = 0.5 * (ax[0] * ax[0] + ax[1] * ax[1] + ax[2] * ax[2]).sqrt();
    gen_cylinder(c, ax, len, r, strip);
}

impl Hack3d for CubocteversionState {
    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = width as f32 / height.max(1) as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.animate();
        self.generate_geometry();
        self.generate_tubes();

        g.glx.clear_color(0.0, 0.0, 0.0, 0.0);
        g.glx.clear();

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        let z_min = (-self.offset3d[2] - 9.5).max(0.1);
        let z_max = -self.offset3d[2] + 9.5;
        if self.projection == Projection::Orthographic {
            g.glx.ortho(
                -self.fov_ortho * self.aspect,
                self.fov_ortho * self.aspect,
                -self.fov_ortho,
                self.fov_ortho,
                z_min,
                z_max,
            );
        } else {
            g.glx.perspective(self.fov_angle, self.aspect, z_min, z_max);
        }

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.color_material(false);
        g.glx.light_model_ambient(LIGHT_MODEL_AMBIENT);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(0, -0.3, 0.3, 1.0, 0.0);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(50.0);

        let q = self.trackball.quaternion();
        let qr = quat_to_rotmat([q.x as f32, q.y as f32, q.z as f32, q.w as f32]);
        g.glx
            .translate(self.offset3d[0], self.offset3d[1], self.offset3d[2]);
        g.glx.mult_matrix(qr);
        g.glx.rotate(self.alpha, 1.0, 0.0, 0.0);
        g.glx.rotate(self.beta, 0.0, 1.0, 0.0);
        g.glx.rotate(self.delta, 0.0, 0.0, 1.0);
        // The two families of models are centred differently.
        if self.method == Method::Apery {
            g.glx.translate(0.0, 0.0, -M_SQRT2_F);
        } else {
            g.glx.translate(0.0, 0.0, -1.0);
        }

        g.glx.cull_face(false);
        g.glx.depth_test(true);
        g.glx.depth_mask(true);
        g.glx.blend(Blend::Off);

        // The tubes are always solid, whatever the faces are doing.
        self.draw_strip(g, &self.tube, MAT_DIFF_TUBE);
        self.draw_strip(g, &self.self_tube, MAT_DIFF_SELF);

        if self.display_mode == DisplayMode::Surface {
            g.glx.depth_mask(true);
            g.glx.blend(Blend::Off);
        } else {
            g.glx.depth_mask(false);
            g.glx.blend(Blend::AlphaAdd);
        }

        if self.colors == Colors::TwoSided {
            let alpha = if self.display_mode == DisplayMode::Transparent {
                self.opacity
            } else {
                1.0
            };
            let mut front = MAT_DIFF_MAGENTA;
            let mut back = MAT_DIFF_CYAN;
            front[3] = alpha;
            back[3] = alpha;
            g.glx.material_ambient_diffuse(front);
            g.glx.material_back_ambient_diffuse(back);
            g.glx.begin(Shape::Triangles);
            for i in 0..3 * NUM_FACE {
                let (n, t) = (self.tn[i], self.tt[i]);
                g.glx.normal3f(n[0], n[1], n[2]);
                g.glx.vertex3f(t[0], t[1], t[2]);
            }
            g.glx.end();
        } else {
            // A face colour is the same on both sides, so it can ride on the
            // vertices and the whole shape is one block.
            g.glx.color_material(true);
            g.glx.begin(Shape::Triangles);
            for i in 0..3 * NUM_FACE {
                let (n, t, c) = (self.tn[i], self.tt[i], self.tc[i]);
                g.glx.color4f(c[0], c[1], c[2], c[3]);
                g.glx.normal3f(n[0], n[1], n[2]);
                g.glx.vertex3f(t[0], t[1], t[2]);
            }
            g.glx.end();
            g.glx.color_material(false);
        }

        g.glx.depth_mask(true);
        g.glx.blend(Blend::Off);

        g.res.int("delay").max(0) as u32
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let em = g.res.string("eversionMethod").to_string();
    let (method, random_method) = match em.as_str() {
        "morin-denner" => (Method::MorinDenner, false),
        "apery" => (Method::Apery, false),
        _ => (
            if random().is_multiple_of(2) {
                Method::Apery
            } else {
                Method::MorinDenner
            },
            true,
        ),
    };

    let mode = g.res.string("mode").to_string();
    let (display_mode, random_display_mode) = match mode.as_str() {
        "surface" => (DisplayMode::Surface, false),
        "transparent" => (DisplayMode::Transparent, false),
        _ => (
            if random().is_multiple_of(2) {
                DisplayMode::Surface
            } else {
                DisplayMode::Transparent
            },
            true,
        ),
    };

    let e = g.res.string("edges").to_string();
    let (edge_tubes, random_edge_tubes) = match e.as_str() {
        "on" => (true, false),
        "off" => (false, false),
        _ => (random() & 1 != 0, true),
    };

    let s = g.res.string("selfIntersections").to_string();
    let (self_tubes, random_self_tubes) = match s.as_str() {
        "on" => (true, false),
        "off" => (false, false),
        _ => (random() & 1 != 0, true),
    };

    let c = g.res.string("colors").to_string();
    let (colors, random_colors) = match c.as_str() {
        "two-sided" => (Colors::TwoSided, false),
        "face" => (Colors::Face, false),
        "earth" => (Colors::Earth, false),
        _ => (
            if random().is_multiple_of(2) {
                Colors::TwoSided
            } else {
                Colors::Face
            },
            true,
        ),
    };

    let p = g.res.string("projection").to_string();
    let (projection, random_projection) = match p.as_str() {
        "perspective" => (Projection::Perspective, false),
        "orthographic" => (Projection::Orthographic, false),
        _ => (
            if random().is_multiple_of(2) {
                Projection::Perspective
            } else {
                Projection::Orthographic
            },
            true,
        ),
    };

    let mut deform_speed = g.res.float("deformSpeed") as f32;
    if deform_speed == 0.0 {
        deform_speed = 20.0;
    }

    let mut st = CubocteversionState {
        method,
        random_method,
        display_mode,
        random_display_mode,
        edge_tubes,
        random_edge_tubes,
        self_tubes,
        random_self_tubes,
        colors,
        random_colors,
        projection,
        random_projection,

        alpha: frand(120.0) as f32 - 60.0,
        beta: frand(120.0) as f32 - 60.0,
        delta: frand(360.0) as f32,
        anim_state: AnimState::Deform,
        time: TIME_MIN,
        defdir: 1.0,
        swipe_step: 0,
        swipe_y: 0.0,
        offset3d: [0.0, 0.0, -10.0],
        fov_angle: frand(f64::from(MAX_FOV_ANGLE - MIN_FOV_ANGLE)) as f32 + MIN_FOV_ANGLE,
        fov_ortho: frand(f64::from(MAX_FOV_ORTHO - MIN_FOV_ORTHO)) as f32 + MIN_FOV_ORTHO,
        opacity: frand(f64::from(MAX_OPACITY - MIN_OPACITY)) as f32 + MIN_OPACITY,

        tt: vec![[0.0; 3]; 3 * NUM_FACE],
        tn: vec![[0.0; 3]; 3 * NUM_FACE],
        te: vec![[0.0; 3]; 2 * NUM_EDGE],
        tv: vec![[0.0; 3]; NUM_VERT],
        tc: vec![[1.0; 4]; 3 * NUM_FACE],
        se: Vec::new(),
        sv: Vec::new(),
        tube: Strip::default(),
        self_tube: Strip::default(),

        aspect: 1.0,
        trackball: Trackball::new(),
        /* Make multiple screens rotate at slightly different rates. */
        speed_scale: 0.9 + frand(0.3) as f32,
        speed_x: g.res.float("speedx") as f32,
        speed_y: g.res.float("speedy") as f32,
        speed_z: g.res.float("speedz") as f32,
        deform_speed,
    };
    st.setup_colors();

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:            10000",
    "*showFPS:          False",
    "*eversionMethod:   random",
    "*mode:             random",
    "*edges:            random",
    "*selfIntersections: random",
    "*colors:           random",
    "*projection:       random",
    "*speedx:           0.0",
    "*speedy:           0.0",
    "*speedz:           0.0",
    "*deformSpeed:      20.0",
];

const METHODS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random deformation",
    },
    SelectItem {
        value: "morin-denner",
        label: "Morin-Denner",
    },
    SelectItem {
        value: "apery",
        label: "Apéry",
    },
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random surface",
    },
    SelectItem {
        value: "surface",
        label: "Solid surface",
    },
    SelectItem {
        value: "transparent",
        label: "Transparent surface",
    },
];

const EDGES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random edge tubes",
    },
    SelectItem {
        value: "on",
        label: "With edge tubes",
    },
    SelectItem {
        value: "off",
        label: "Without edge tubes",
    },
];

const SELF_TUBES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random self-intersection tubes",
    },
    SelectItem {
        value: "on",
        label: "With self-intersection tubes",
    },
    SelectItem {
        value: "off",
        label: "Without self-intersection tubes",
    },
];

const COLORINGS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random coloration",
    },
    SelectItem {
        value: "two-sided",
        label: "Two-sided",
    },
    SelectItem {
        value: "face",
        label: "Face colors",
    },
    SelectItem {
        value: "earth",
        label: "Earth colors",
    },
];

const PROJECTIONS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random projection",
    },
    SelectItem {
        value: "perspective",
        label: "Perspective",
    },
    SelectItem {
        value: "orthographic",
        label: "Orthographic",
    },
];

/// Upstream's transparency knob is not here: it chose between two
/// depth-peeling schemes, which are a way of drawing correct transparency in a
/// fragment shader. The fixed-function path has none of that and neither has
/// this, so the knob would have had nothing to select.
const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider(
        "deformSpeed",
        "Deformation speed",
        1.0,
        100.0,
        1.0,
        1,
        "20.0",
    ),
    Opt::select("eversionMethod", "Deformation", METHODS, "random"),
    Opt::select("mode", "Surface", MODES, "random"),
    Opt::select("edges", "Edge tubes", EDGES, "random"),
    Opt::select(
        "selfIntersections",
        "Self-intersections",
        SELF_TUBES,
        "random",
    ),
    Opt::select("colors", "Coloration", COLORINGS, "random"),
    Opt::select("projection", "Projection", PROJECTIONS, "random"),
    Opt::slider("speedx", "X rotation speed", -4.0, 4.0, 0.1, 1, "0.0"),
    Opt::slider("speedy", "Y rotation speed", -4.0, 4.0, 0.1, 1, "0.0"),
    Opt::slider("speedz", "Z rotation speed", -4.0, 4.0, 0.1, 1, "0.0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cubocteversion",
    label: "Cuboctahedron Eversion",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Steger",
        year: "2023",
        video: Some("https://www.youtube.com/watch?v=Yrxf9CNop20"),
        blurb: "Turns a cuboctahedron inside out: a smooth deformation (homotopy).",
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
    use crate::runtime::ya_rand_init;

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// The tabulated polyhedra really are cuboctahedra at both ends: twelve
    /// vertices, and every one of the twenty-four short edges the same length.
    ///
    /// A cuboctahedron has 24 edges, and the models carry 30, because the six
    /// square faces are cut into two triangles each and the cut is an edge
    /// like any other. So six of the thirty are diagonals and are longer.
    #[test]
    fn the_models_begin_and_end_as_cuboctahedra() {
        for models in [&VERT_APERY[..], &VERT_DENNER[..]] {
            for v in [&models[0], &models[models.len() - 1]] {
                let mut lengths: Vec<f32> = EDGE
                    .iter()
                    .map(|e| {
                        let (a, b) = (v[e[0]], v[e[1]]);
                        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2))
                            .sqrt()
                    })
                    .collect();
                lengths.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let short = &lengths[..24];
                let long = &lengths[24..];
                assert!(
                    short[23] - short[0] < 0.001,
                    "the edges ran from {} to {}",
                    short[0],
                    short[23]
                );
                assert!(
                    long[0] > short[23] * 1.3,
                    "the six diagonals were not longer: {long:?} against {}",
                    short[23]
                );
            }
        }
    }

    /// Every face is a real triangle at every instant of both eversions, so
    /// every normal is a unit vector rather than the fallback for a collapsed
    /// one.
    #[test]
    fn no_face_ever_collapses() {
        ya_rand_init(20260812);
        for method in [Method::Apery, Method::MorinDenner] {
            let mut r = start(StartArgs::new(640, 480, "", 20260812));
            r.step();
            let mut st = state(method);
            let mut steps = 0;
            let mut time = TIME_MIN;
            while time <= TIME_MAX {
                st.time = time;
                st.generate_geometry();
                for n in &st.tn {
                    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    assert!(
                        (l - 1.0).abs() < 0.001,
                        "a normal of length {l} at time {time}"
                    );
                }
                time += 0.5;
                steps += 1;
            }
            assert!(steps > 200);
        }
    }

    fn state(method: Method) -> CubocteversionState {
        CubocteversionState {
            method,
            random_method: false,
            display_mode: DisplayMode::Surface,
            random_display_mode: false,
            edge_tubes: true,
            random_edge_tubes: false,
            self_tubes: true,
            random_self_tubes: false,
            colors: Colors::TwoSided,
            random_colors: false,
            projection: Projection::Perspective,
            random_projection: false,
            alpha: 0.0,
            beta: 0.0,
            delta: 0.0,
            anim_state: AnimState::Deform,
            time: TIME_MIN,
            defdir: 1.0,
            swipe_step: 0,
            swipe_y: 0.0,
            offset3d: [0.0, 0.0, -10.0],
            fov_angle: 40.0,
            fov_ortho: 4.0,
            opacity: 0.5,
            tt: vec![[0.0; 3]; 3 * NUM_FACE],
            tn: vec![[0.0; 3]; 3 * NUM_FACE],
            te: vec![[0.0; 3]; 2 * NUM_EDGE],
            tv: vec![[0.0; 3]; NUM_VERT],
            tc: vec![[1.0; 4]; 3 * NUM_FACE],
            se: Vec::new(),
            sv: Vec::new(),
            tube: Strip::default(),
            self_tube: Strip::default(),
            aspect: 1.0,
            trackball: Trackball::new(),
            speed_scale: 1.0,
            speed_x: 0.0,
            speed_y: 0.0,
            speed_z: 0.0,
            deform_speed: 20.0,
        }
    }

    /// The surface passes through itself in the middle of the eversion and not
    /// at either end. That is the whole difficulty of an eversion, and it is
    /// found by intersecting triangles rather than tabulated, so it is worth
    /// checking that it is found.
    #[test]
    fn the_surface_passes_through_itself_only_in_the_middle() {
        ya_rand_init(20260812);
        for method in [Method::Apery, Method::MorinDenner] {
            let mut st = state(method);
            st.time = TIME_MIN;
            st.generate_geometry();
            assert_eq!(st.se.len(), 0, "it crossed itself at the start");
            st.time = TIME_MAX;
            st.generate_geometry();
            assert_eq!(st.se.len(), 0, "it crossed itself at the end");

            let mut worst = 0;
            let mut time = -20.0;
            while time <= 20.0 {
                st.time = time;
                st.generate_geometry();
                worst = worst.max(st.se.len());
                // Every segment ends at a recorded vertex, and the vertices
                // are deduplicated, so there are never more of them than ends.
                assert!(st.sv.len() <= 2 * st.se.len());
                time += 1.0;
            }
            assert!(worst > 4, "it only ever crossed itself {worst} times");
        }
    }

    /// Every self-intersection segment really does lie in both of the
    /// triangles it came from.
    ///
    /// This is the test that says the intersection predicate was transcribed
    /// right. It is six nested cases deep with six permutations of its
    /// arguments at each, so a swapped pair would still return plausible
    /// segments in plausible places, and the only thing that catches it is
    /// checking the answer against its own definition.
    #[test]
    fn a_self_intersection_lies_in_both_triangles() {
        ya_rand_init(20260812);
        // How far off a triangle's plane, and outside its edges, a point may
        // be: the vertices are order one and the arithmetic is in doubles.
        let tol = 1.0e-4f32;

        // Is `p` in the triangle `a b c`, allowing `tol`?
        let inside = |p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]| {
            let sub = |x: [f32; 3], y: [f32; 3]| [x[0] - y[0], x[1] - y[1], x[2] - y[2]];
            let cr = |x: [f32; 3], y: [f32; 3]| {
                [
                    x[1] * y[2] - x[2] * y[1],
                    x[2] * y[0] - x[0] * y[2],
                    x[0] * y[1] - x[1] * y[0],
                ]
            };
            let dt = |x: [f32; 3], y: [f32; 3]| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
            let n = cr(sub(b, a), sub(c, a));
            let area = dt(n, n).sqrt();
            if area <= 0.0 {
                return false;
            }
            // Off the plane?
            if (dt(n, sub(p, a)) / area).abs() > tol {
                return false;
            }
            // The three barycentric coordinates must all be non-negative.
            for (x, y) in [(a, b), (b, c), (c, a)] {
                if dt(cr(sub(y, x), sub(p, x)), n) / area < -tol * area {
                    return false;
                }
            }
            true
        };

        for method in [Method::Apery, Method::MorinDenner] {
            let mut st = state(method);
            let mut checked = 0;
            let mut time = -25.0;
            while time <= 25.0 {
                st.time = time;
                st.generate_geometry();
                // Redo the pairing to know which triangles each segment came
                // from, in the same order `generate_geometry` walks them.
                let v: Vec<[f32; 3]> = (0..NUM_VERT).map(|i| st.tv[i]).collect();
                let mut k = 0;
                for i in 0..NUM_FACE {
                    for j in i + 1..NUM_FACE {
                        if FACE[i].iter().any(|a| FACE[j].contains(a)) {
                            continue;
                        }
                        let (t1, t2) = (FACE[i], FACE[j]);
                        // Only pairs that produced a segment advance `k`, and
                        // they do so in this order, so re-running the
                        // predicate tells us which is which.
                        let p = |f: [usize; 3], m: usize| {
                            let x = v[f[m]];
                            [f64::from(x[0]), f64::from(x[1]), f64::from(x[2])]
                        };
                        if tri_tri_intersection_3d(
                            p(t1, 0),
                            p(t1, 1),
                            p(t1, 2),
                            p(t2, 0),
                            p(t2, 1),
                            p(t2, 2),
                        )
                        .is_none()
                        {
                            continue;
                        }
                        let (a, b) = st.se[k];
                        k += 1;
                        for end in [a, b] {
                            assert!(
                                inside(end, v[t1[0]], v[t1[1]], v[t1[2]]),
                                "at time {time} an end was outside triangle {i}"
                            );
                            assert!(
                                inside(end, v[t2[0]], v[t2[1]], v[t2[2]]),
                                "at time {time} an end was outside triangle {j}"
                            );
                            checked += 1;
                        }
                    }
                }
                assert_eq!(k, st.se.len());
                time += 1.0;
            }
            assert!(checked > 100, "only {checked} ends checked");
        }
    }

    /// The tubes are one draw call each, however many balls and cylinders they
    /// are made of, because the strips are joined.
    #[test]
    fn the_tubes_are_one_draw_call_each() {
        let r = run("edges=on&selfIntersections=on&colors=face", 300);
        let f = r.frame();
        assert!(
            f.batches.len() <= 4,
            "{} batches: {:?}",
            f.batches.len(),
            f.batches.iter().map(|b| b.count).collect::<Vec<_>>()
        );
        // Twelve balls of sixteen strips and thirty cylinders, joined, plus
        // whatever the self-intersections come to at this instant.
        assert!(
            f.vertices.len() > 7_000,
            "only {} vertices",
            f.vertices.len()
        );
        assert!(f.vertices.len() < 120_000, "{} vertices", f.vertices.len());
    }

    /// Each of the two eversions runs end to end and then swipes the finished
    /// shape off the screen and a new one on.
    #[test]
    fn the_shape_is_swiped_away_between_eversions() {
        let mut r = start(StartArgs::new(640, 480, "deformSpeed=100", 20260812));
        let mut lowest = 0.0f32;
        for _ in 0..1500 {
            r.step();
            let m = r.frame().batches[0].modelview;
            lowest = lowest.min(m.0[13]);
        }
        assert!(lowest < -2.0, "the shape never swiped away: {lowest}");
    }

    /// The tubes can be turned off, and turning them off leaves the faces.
    #[test]
    fn the_tubes_can_be_turned_off() {
        let with = run("edges=on&selfIntersections=off&colors=face", 2)
            .frame()
            .vertices
            .len();
        let without = run("edges=off&selfIntersections=off&colors=face", 2)
            .frame()
            .vertices
            .len();
        assert_eq!(
            without,
            3 * NUM_FACE,
            "the faces should be all that is left"
        );
        assert!(with > without + 5000);
    }
}
