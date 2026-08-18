//! Port of `hacks/glx/teapot.c`.
//!
//! ```text
//! Copyright (c) Mark J. Kilgard, 1994.
//!
//! (c) Copyright 1993, Silicon Graphics, Inc.
//!
//! ALL RIGHTS RESERVED
//!
//! Permission to use, copy, modify, and distribute this software
//! for any purpose and without fee is hereby granted, provided
//! that the above copyright notice appear in all copies and that
//! both the copyright notice and this permission notice appear in
//! supporting documentation, and that the name of Silicon
//! Graphics, Inc. not be used in advertising or publicity
//! pertaining to distribution of the software without specific,
//! written prior permission.
//!
//! THE MATERIAL EMBODIED ON THIS SOFTWARE IS PROVIDED TO YOU
//! "AS-IS" AND WITHOUT WARRANTY OF ANY KIND, EXPRESS, IMPLIED OR
//! OTHERWISE, INCLUDING WITHOUT LIMITATION, ANY WARRANTY OF
//! MERCHANTABILITY OR FITNESS FOR A PARTICULAR PURPOSE.  IN NO
//! EVENT SHALL SILICON GRAPHICS, INC.  BE LIABLE TO YOU OR ANYONE
//! ELSE FOR ANY DIRECT, SPECIAL, INCIDENTAL, INDIRECT OR
//! CONSEQUENTIAL DAMAGES OF ANY KIND, OR ANY DAMAGES WHATSOEVER,
//! INCLUDING WITHOUT LIMITATION, LOSS OF PROFIT, LOSS OF USE,
//! SAVINGS OR REVENUE, OR THE CLAIMS OF THIRD PARTIES, WHETHER OR
//! NOT SILICON GRAPHICS, INC.  HAS BEEN ADVISED OF THE POSSIBILITY
//! OF SUCH LOSS, HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//! ARISING OUT OF OR IN CONNECTION WITH THE POSSESSION, USE OR
//! PERFORMANCE OF THIS SOFTWARE.
//!
//! OpenGL(TM) is a trademark of Silicon Graphics, Inc.
//! ```
//!
//! The Utah teapot: Martin Newell's 1975 test model, as thirty-two bicubic
//! Bezier patches over a hundred and twenty-seven control points. Ten patches
//! are stored and the rest are got by mirroring, since the pot is symmetric
//! about one plane and the body about two.
//!
//! Upstream hands the control points to `glMap2f` and lets `glEvalMesh2` walk
//! the grid, with `GL_AUTO_NORMAL` taking the normal from the two partial
//! derivatives. There is no evaluator here, so the Bernstein polynomials and
//! their derivatives are worked out directly, which is what an evaluator does.
//! Upstream's fallback for OpenGL ES instead ships a big table of triangles
//! that had been evaluated in advance; evaluating them is smaller than the
//! table and gives a rounder pot.
//!
//! The texture coordinates upstream generates alongside are not here: nothing
//! that draws a teapot turns texturing on.

use super::gl::{Glx, Shape};

/// Which control points make up each patch. Rim, body, lid and bottom are
//  mirrored in both x and y; the handle and spout only across y.
static PATCHDATA: [[usize; 16]; 10] = [
    // Rim.
    [102, 103, 104, 105, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    // Body.
    [
        12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    ],
    [
        24, 25, 26, 27, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    ],
    // Lid.
    [
        96, 96, 96, 96, 97, 98, 99, 100, 101, 101, 101, 101, 0, 1, 2, 3,
    ],
    [
        0, 1, 2, 3, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117,
    ],
    // Bottom.
    [
        118, 118, 118, 118, 124, 122, 119, 121, 123, 126, 125, 120, 40, 39, 38, 37,
    ],
    // Handle.
    [
        41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56,
    ],
    [
        53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 28, 65, 66, 67,
    ],
    // Spout.
    [
        68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83,
    ],
    [
        80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95,
    ],
];

#[rustfmt::skip]
static CPDATA: [[f32; 3]; 127] = [
    [0.2, 0.0, 2.7], [0.2, -0.112, 2.7], [0.112, -0.2, 2.7], [0.0, -0.2, 2.7],
    [1.3375, 0.0, 2.53125], [1.3375, -0.749, 2.53125], [0.749, -1.3375, 2.53125],
    [0.0, -1.3375, 2.53125], [1.4375, 0.0, 2.53125], [1.4375, -0.805, 2.53125],
    [0.805, -1.4375, 2.53125], [0.0, -1.4375, 2.53125], [1.5, 0.0, 2.4],
    [1.5, -0.84, 2.4], [0.84, -1.5, 2.4], [0.0, -1.5, 2.4], [1.75, 0.0, 1.875],
    [1.75, -0.98, 1.875], [0.98, -1.75, 1.875], [0.0, -1.75, 1.875],
    [2.0, 0.0, 1.35], [2.0, -1.12, 1.35], [1.12, -2.0, 1.35], [0.0, -2.0, 1.35],
    [2.0, 0.0, 0.9], [2.0, -1.12, 0.9], [1.12, -2.0, 0.9], [0.0, -2.0, 0.9],
    [-2.0, 0.0, 0.9], [2.0, 0.0, 0.45], [2.0, -1.12, 0.45], [1.12, -2.0, 0.45],
    [0.0, -2.0, 0.45], [1.5, 0.0, 0.225], [1.5, -0.84, 0.225], [0.84, -1.5, 0.225],
    [0.0, -1.5, 0.225], [1.5, 0.0, 0.15], [1.5, -0.84, 0.15], [0.84, -1.5, 0.15],
    [0.0, -1.5, 0.15], [-1.6, 0.0, 2.025], [-1.6, -0.3, 2.025], [-1.5, -0.3, 2.25],
    [-1.5, 0.0, 2.25], [-2.3, 0.0, 2.025], [-2.3, -0.3, 2.025], [-2.5, -0.3, 2.25],
    [-2.5, 0.0, 2.25], [-2.7, 0.0, 2.025], [-2.7, -0.3, 2.025], [-3.0, -0.3, 2.25],
    [-3.0, 0.0, 2.25], [-2.7, 0.0, 1.8], [-2.7, -0.3, 1.8], [-3.0, -0.3, 1.8],
    [-3.0, 0.0, 1.8], [-2.7, 0.0, 1.575], [-2.7, -0.3, 1.575], [-3.0, -0.3, 1.35],
    [-3.0, 0.0, 1.35], [-2.5, 0.0, 1.125], [-2.5, -0.3, 1.125], [-2.65, -0.3, 0.9375],
    [-2.65, 0.0, 0.9375], [-2.0, -0.3, 0.9], [-1.9, -0.3, 0.6], [-1.9, 0.0, 0.6],
    [1.7, 0.0, 1.425], [1.7, -0.66, 1.425], [1.7, -0.66, 0.6], [1.7, 0.0, 0.6],
    [2.6, 0.0, 1.425], [2.6, -0.66, 1.425], [3.1, -0.66, 0.825], [3.1, 0.0, 0.825],
    [2.3, 0.0, 2.1], [2.3, -0.25, 2.1], [2.4, -0.25, 2.025], [2.4, 0.0, 2.025],
    [2.7, 0.0, 2.4], [2.7, -0.25, 2.4], [3.3, -0.25, 2.4], [3.3, 0.0, 2.4],
    [2.8, 0.0, 2.475], [2.8, -0.25, 2.475], [3.525, -0.25, 2.49375],
    [3.525, 0.0, 2.49375], [2.9, 0.0, 2.475], [2.9, -0.15, 2.475],
    [3.45, -0.15, 2.5125], [3.45, 0.0, 2.5125], [2.8, 0.0, 2.4], [2.8, -0.15, 2.4],
    [3.2, -0.15, 2.4], [3.2, 0.0, 2.4], [0.0, 0.0, 3.15], [0.8, 0.0, 3.15],
    [0.8, -0.45, 3.15], [0.45, -0.8, 3.15], [0.0, -0.8, 3.15], [0.0, 0.0, 2.85],
    [1.4, 0.0, 2.4], [1.4, -0.784, 2.4], [0.784, -1.4, 2.4], [0.0, -1.4, 2.4],
    [0.4, 0.0, 2.55], [0.4, -0.224, 2.55], [0.224, -0.4, 2.55], [0.0, -0.4, 2.55],
    [1.3, 0.0, 2.55], [1.3, -0.728, 2.55], [0.728, -1.3, 2.55], [0.0, -1.3, 2.55],
    [1.3, 0.0, 2.4], [1.3, -0.728, 2.4], [0.728, -1.3, 2.4], [0.0, -1.3, 2.4],
    [0.0, 0.0, 0.0], [1.425, -0.798, 0.0], [1.5, 0.0, 0.075], [1.425, 0.0, 0.0],
    [0.798, -1.425, 0.0], [0.0, -1.5, 0.075], [0.0, -1.425, 0.0],
    [1.5, -0.84, 0.075], [0.84, -1.5, 0.075],
];

/// The four cubic Bernstein polynomials at `t`, and their derivatives.
fn bernstein(t: f32) -> ([f32; 4], [f32; 4]) {
    let s = 1.0 - t;
    (
        [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t],
        [
            -3.0 * s * s,
            3.0 * s * s - 6.0 * s * t,
            6.0 * s * t - 3.0 * t * t,
            3.0 * t * t,
        ],
    )
}

/// A point on a patch, and the two partial derivatives there.
fn eval(patch: &[[f32; 3]; 16], u: f32, v: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (bu, du) = bernstein(u);
    let (bv, dv) = bernstein(v);
    let mut p = [0.0f32; 3];
    let mut pu = [0.0f32; 3];
    let mut pv = [0.0f32; 3];
    for j in 0..4 {
        for k in 0..4 {
            let c = patch[j * 4 + k];
            for l in 0..3 {
                p[l] += bu[j] * bv[k] * c[l];
                pu[l] += du[j] * bv[k] * c[l];
                pv[l] += bu[j] * dv[k] * c[l];
            }
        }
    }
    (p, pu, pv)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// One patch, as a grid of quads. `GL_AUTO_NORMAL` takes the normal from the
/// cross product of the two partial derivatives, so that is what happens here.
/// At the poles of the lid and the bottom a whole row of control points is the
/// same one, both derivatives collapse and the normal is nothing; a sample
/// just inside the edge stands in.
fn patch(g: &mut Glx, points: &[[f32; 3]; 16], grid: usize, wire: bool) {
    let mut p = vec![[0.0f32; 3]; (grid + 1) * (grid + 1)];
    let mut n = vec![[0.0f32; 3]; (grid + 1) * (grid + 1)];
    for i in 0..=grid {
        for j in 0..=grid {
            let (u, v) = (i as f32 / grid as f32, j as f32 / grid as f32);
            let (q, qu, qv) = eval(points, u, v);
            let mut nor = cross(qu, qv);
            if nor[0] * nor[0] + nor[1] * nor[1] + nor[2] * nor[2] < 1e-12 {
                let nudge = 0.5 / grid as f32;
                let (_, qu, qv) = eval(
                    points,
                    u.clamp(nudge, 1.0 - nudge),
                    v.clamp(nudge, 1.0 - nudge),
                );
                nor = cross(qu, qv);
            }
            p[i * (grid + 1) + j] = q;
            n[i * (grid + 1) + j] = nor;
        }
    }

    let at = |i: usize, j: usize| i * (grid + 1) + j;
    if wire {
        g.begin(Shape::Lines);
        for i in 0..=grid {
            for j in 0..=grid {
                if i < grid {
                    g.vertex3f(p[at(i, j)][0], p[at(i, j)][1], p[at(i, j)][2]);
                    g.vertex3f(p[at(i + 1, j)][0], p[at(i + 1, j)][1], p[at(i + 1, j)][2]);
                }
                if j < grid {
                    g.vertex3f(p[at(i, j)][0], p[at(i, j)][1], p[at(i, j)][2]);
                    g.vertex3f(p[at(i, j + 1)][0], p[at(i, j + 1)][1], p[at(i, j + 1)][2]);
                }
            }
        }
        g.end();
        return;
    }

    g.begin(Shape::Quads);
    for i in 0..grid {
        for j in 0..grid {
            for (a, b) in [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)] {
                let (q, nor) = (p[at(a, b)], n[at(a, b)]);
                g.normal3f(nor[0], nor[1], nor[2]);
                g.vertex3f(q[0], q[1], q[2]);
            }
        }
    }
    g.end();
}

/// `unit_teapot`: draw a teapot about a unit across, and say how many polygons
/// went into it.
pub fn unit_teapot(g: &mut Glx, grid: usize, wire: bool) -> usize {
    let mut polys = 0;

    g.front_face_cw(true);
    g.push_matrix();
    g.rotate(270.0, 1.0, 0.0, 0.0);
    g.scale(0.5, 0.5, 0.5);
    g.translate(0.0, 0.0, -1.5);

    for i in 0..10 {
        let mut p = [[0.0f32; 3]; 16];
        let mut q = [[0.0f32; 3]; 16];
        let mut r = [[0.0f32; 3]; 16];
        let mut s = [[0.0f32; 3]; 16];
        for j in 0..4 {
            for k in 0..4 {
                for l in 0..3 {
                    p[j * 4 + k][l] = CPDATA[PATCHDATA[i][j * 4 + k]][l];
                    q[j * 4 + k][l] = CPDATA[PATCHDATA[i][j * 4 + (3 - k)]][l];
                    if l == 1 {
                        q[j * 4 + k][l] *= -1.0;
                    }
                    if i < 6 {
                        r[j * 4 + k][l] = CPDATA[PATCHDATA[i][j * 4 + (3 - k)]][l];
                        if l == 0 {
                            r[j * 4 + k][l] *= -1.0;
                        }
                        s[j * 4 + k][l] = CPDATA[PATCHDATA[i][j * 4 + k]][l];
                        if l == 0 {
                            s[j * 4 + k][l] *= -1.0;
                        }
                        if l == 1 {
                            s[j * 4 + k][l] *= -1.0;
                        }
                    }
                }
            }
        }
        patch(g, &p, grid, wire);
        polys += grid * grid * 2;
        patch(g, &q, grid, wire);
        polys += grid * grid * 2;
        if i < 6 {
            patch(g, &r, grid, wire);
            polys += grid * grid * 2;
            patch(g, &s, grid, wire);
            polys += grid * grid * 2;
        }
    }

    g.pop_matrix();
    polys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Gl;

    /// Every vertex of the drawn pot, in the frame it was drawn into.
    fn pot(grid: usize, wire: bool) -> (Vec<[f32; 3]>, usize) {
        let mut g = Gl::for_test(640, 480);
        g.glx.start_frame(640, 480);
        let polys = unit_teapot(&mut g.glx, grid, wire);
        let f = g.glx.frame();
        let mut out = Vec::new();
        for b in &f.batches {
            for v in &f.vertices[b.first..b.first + b.count] {
                out.push(b.mvp.transform(v.pos));
            }
        }
        (out, polys)
    }

    #[test]
    fn the_pot_is_about_a_unit_across_and_sits_on_the_origin() {
        let (v, polys) = pot(6, false);
        assert_eq!(polys, 32 * 6 * 6 * 2);
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in &v {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        // Widest across the spout-to-handle axis, shortest top to bottom, and
        // centred on the origin apart from that spout.
        assert!(hi[0] - lo[0] > 2.5 && hi[0] - lo[0] < 3.5, "{lo:?} {hi:?}");
        assert!(hi[1] - lo[1] > 1.5 && hi[1] - lo[1] < 2.0, "{lo:?} {hi:?}");
        // Straddling the origin rather than standing on it: upstream shifts
        // the pot down by three quarters of its height, not all of it.
        assert!(lo[1] < -0.5 && hi[1] > 0.5, "{lo:?} {hi:?}");
        // Symmetric about the plane the handle and spout lie in.
        assert!((hi[2] + lo[2]).abs() < 1e-3, "{lo:?} {hi:?}");
    }

    #[test]
    fn every_normal_points_somewhere() {
        // The lid and the bottom each have a patch whose top row of control
        // points is one point repeated, where the derivatives collapse.
        let mut g = Gl::for_test(64, 64);
        g.glx.start_frame(64, 64);
        unit_teapot(&mut g.glx, 4, false);
        let f = g.glx.frame();
        let mut checked = 0;
        for b in &f.batches {
            for v in &f.vertices[b.first..b.first + b.count] {
                let n = v.normal;
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                assert!(len > 1e-6, "a flat normal at {:?}", v.pos);
                checked += 1;
            }
        }
        assert!(checked > 1000, "only {checked} vertices");
    }

    #[test]
    fn the_wireframe_is_the_same_shape_in_lines() {
        let (solid, _) = pot(4, false);
        let (wire, _) = pot(4, true);
        assert!(!wire.is_empty());
        let extent = |v: &[[f32; 3]]| {
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for p in v {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            (lo, hi)
        };
        assert_eq!(extent(&solid), extent(&wire));
    }
}
