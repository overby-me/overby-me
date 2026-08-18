//! Ports of `hacks/glx/sphere.c` and `hacks/glx/normals.c`.
//!
//! ```text
//! sphere, Copyright (c) 2002 Paul Bourke <pbourke@swin.edu.au>,
//!         Copyright (c) 2010-2026 Jamie Zawinski <jwz@jwz.org>
//! Utility function to create a unit sphere in GL.
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
//! Two pieces of geometry that the savers keep asking for: a sphere, which
//! twenty-seven of them draw, and the normal of a triangle, which thirty-one
//! of them compute.
//!
//! The sphere is one long triangle strip rather than a strip per band, which
//! is upstream's shape and worth keeping: the last vertex of one band and the
//! first of the next make a pair of degenerate triangles that cover no pixels
//! and join the bands into a single run. That is why [`unit_sphere`] emits one
//! block rather than `stacks` of them.
//!
//! Texture coordinates are not generated. Upstream's are, along with a long
//! comment about an Android driver that turns texturing on if you so much as
//! mention a texture coordinate array; there is nothing to texture with here
//! yet, and when there is, this is where they go.

use super::gl::{Glx, Shape};

/// `calc_normal`: the unit normal at `p` given two other points on the
/// surface, pointing along `p1` cross `p2`. Not normalised, which is
/// upstream's behaviour: `GL_NORMALIZE` is what the savers rely on, and the
/// shader normalises anyway.
#[must_use]
pub fn calc_normal(p: [f32; 3], p1: [f32; 3], p2: [f32; 3]) -> [f32; 3] {
    let pa = [p1[0] - p[0], p1[1] - p[1], p1[2] - p[2]];
    let pb = [p2[0] - p[0], p2[1] - p[1], p2[2] - p[2]];
    [
        pa[1] * pb[2] - pa[2] * pb[1],
        pa[2] * pb[0] - pa[0] * pb[2],
        pa[0] * pb[1] - pa[1] * pb[0],
    ]
}

/// `do_normal`: set the current normal to the one facing out of this triangle.
pub fn do_normal(g: &mut Glx, p1: [f32; 3], p2: [f32; 3], p3: [f32; 3]) {
    let n = calc_normal(p1, p2, p3);
    g.normal3f(n[0], n[1], n[2]);
}

/// `unit_sphere`: a sphere of radius 1 about the origin. Returns the polygon
/// count, as upstream's does, for the savers that report one.
pub fn unit_sphere(g: &mut Glx, stacks: i32, slices: i32, wire: bool) -> i32 {
    unit_sphere_1(g, stacks, slices, wire, false)
}

/// `unit_dome`: half of one, from the pole at `-y` up to the equator. A caller
/// that wants it the other way up turns it over, which is what upstream's do.
pub fn unit_dome(g: &mut Glx, stacks: i32, slices: i32, wire: bool) -> i32 {
    unit_sphere_1(g, stacks, slices, wire, true)
}

fn unit_sphere_1(g: &mut Glx, stacks: i32, slices: i32, wire: bool, half: bool) -> i32 {
    let slices = slices.abs();
    // Too coarse to be a sphere at all: upstream draws a single point rather
    // than dividing by something that is about to be zero.
    if slices < 4 || stacks < 2 {
        g.begin(Shape::Points);
        g.vertex3f(0.0, 0.0, 0.0);
        g.end();
        return 0;
    }

    let stacks2 = stacks * 2;
    let end = if half { stacks / 2 } else { stacks };
    let pi = std::f32::consts::PI;
    let mut polys = 0;

    // A wireframe sphere is drawn as the quadrilateral outline of each cell,
    // which needs the previous band's two points; the solid one is a strip.
    let mut la = [0.0f32, -1.0, 0.0];
    let mut lb = [0.0f32, -1.0, 0.0];

    g.begin(if wire {
        Shape::LineStrip
    } else {
        Shape::TriangleStrip
    });
    for j in 0..end {
        let theta1 = j as f32 * (pi + pi) / stacks2 as f32 - pi / 2.0;
        let theta2 = (j + 1) as f32 * (pi + pi) / stacks2 as f32 - pi / 2.0;

        for i in (0..=slices).rev() {
            let theta3 = i as f32 * (pi + pi) / slices as f32;

            if wire {
                g.vertex3f(lb[0], lb[1], lb[2]);
                g.vertex3f(la[0], la[1], la[2]);
            }

            let n = [
                theta2.cos() * theta3.cos(),
                theta2.sin(),
                theta2.cos() * theta3.sin(),
            ];
            g.normal3f(n[0], n[1], n[2]);
            g.vertex3f(n[0], n[1], n[2]);
            if wire {
                la = n;
            }

            let n = [
                theta1.cos() * theta3.cos(),
                theta1.sin(),
                theta1.cos() * theta3.sin(),
            ];
            g.normal3f(n[0], n[1], n[2]);
            g.vertex3f(n[0], n[1], n[2]);
            if wire {
                lb = n;
            }

            polys += 1;
        }
    }
    g.end();
    polys
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A normal points out of the face, the way the winding says.
    #[test]
    fn a_normal_faces_the_way_the_winding_does() {
        // Anticlockwise seen from +z, so the normal is +z.
        let n = calc_normal([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(n, [0.0, 0.0, 1.0]);
        // The other way round, and it is -z.
        let n = calc_normal([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        assert_eq!(n, [0.0, 0.0, -1.0]);
    }

    /// Every vertex of a unit sphere is at distance one, and its normal is the
    /// vertex, which is what makes it a *unit* sphere about the origin.
    #[test]
    fn every_point_of_a_sphere_is_one_from_the_middle() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        unit_sphere(&mut g, 16, 16, false);
        let f = g.frame();
        assert_eq!(f.batches.len(), 1, "one strip, not one per band");
        assert!(f.vertices.len() > 500);
        for v in &f.vertices {
            let r = (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt();
            assert!((r - 1.0).abs() < 1e-5, "{:?} is {r} from the middle", v.pos);
            for k in 0..3 {
                assert!((v.normal[k] - v.pos[k]).abs() < 1e-6);
            }
        }
    }

    /// A dome is half a sphere: the half from the `-y` pole to the equator,
    /// and nothing above it. The name is upstream's; which way up it looks is
    /// the caller's business.
    #[test]
    fn a_dome_is_half_a_sphere() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        unit_dome(&mut g, 16, 16, false);
        let f = g.frame();
        assert!(f.vertices.iter().all(|v| v.pos[1] <= 1e-6), "went too far");
        assert!(f.vertices.iter().any(|v| v.pos[1] < -0.9), "no pole");
    }

    /// More slices is more geometry, and a sphere too coarse to be one is a
    /// point rather than a division by zero.
    #[test]
    fn the_detail_knobs_do_something() {
        let count = |stacks, slices| {
            let mut g = Glx::new();
            g.start_frame(100, 100);
            unit_sphere(&mut g, stacks, slices, false);
            g.frame().vertices.len()
        };
        assert!(count(32, 32) > count(8, 8));
        assert_eq!(count(1, 1), 1);
        assert_eq!(count(16, 2), 1);
    }
}
