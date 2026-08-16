//! Port of `hacks/glx/tube.c`.
//!
//! ```text
//! tube, Copyright (c) 2001-2012 Jamie Zawinski <jwz@jwz.org>
//! Utility functions to create tubes and cones in GL.
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
//! A cylinder or a cone between two points, which seventeen of the savers use
//! to draw everything from knots to gear teeth to insect legs.
//!
//! The whole of the geometry is [`unit_tube`] and [`unit_cone`]: a cylinder of
//! radius one from `y = 0` to `y = 1`, about the y axis. [`tube`] just puts one
//! of those where it was asked for, and how it does that is worth reading,
//! because it is doing in two rotations what a general "point this at that"
//! usually takes a basis to do: turn about z until the tube's y axis lies in
//! the plane of the target, then about x until it points along it.
//!
//! `smooth` is the difference between a pipe and a prism, and it is a matter
//! of normals rather than of geometry: with it, each corner of the ring gets
//! its own outward normal and the shading runs round smoothly; without, both
//! corners of a face share the normal of the middle of that face, and each
//! face is visibly flat. That is also why the flat version is triangles rather
//! than a strip, since a strip cannot give two faces different normals at a
//! shared vertex.
//!
//! Texture coordinates are not computed. Neither are upstream's, which says so
//! in the same words.

use super::gl::{Glx, Shape};

/// `unit_tube`: a cylinder of radius 1 from `y = 0` to `y = 1`.
pub fn unit_tube(g: &mut Glx, faces: i32, smooth: bool, caps: bool, wire: bool) -> i32 {
    let faces = faces.max(3);
    let step = std::f32::consts::PI * 2.0 / faces as f32;
    let s2 = step / 2.0;
    let mut polys = 0;

    let (mut th, mut x, mut y) = (0.0f32, 1.0f32, 0.0f32);
    let (mut x0, mut y0) = if smooth {
        (0.0, 0.0)
    } else {
        (s2.cos(), s2.sin())
    };

    // Smooth walls close the ring by repeating the first pair, which one more
    // turn of the loop does; flat ones emit each face on its own.
    let n = if smooth { faces + 1 } else { faces };
    g.front_face_cw(false);
    g.begin(if wire {
        Shape::Lines
    } else if smooth {
        Shape::TriangleStrip
    } else {
        Shape::Triangles
    });
    for _ in 0..n {
        let normal = if smooth { [x, 0.0, y] } else { [x0, 0.0, y0] };
        g.normal3f(normal[0], normal[1], normal[2]);
        g.vertex3f(x, 0.0, y); /* bottom point A */
        g.vertex3f(x, 1.0, y); /* top point A */

        th += step;
        let (nx, ny) = (th.cos(), th.sin());

        if !smooth {
            x0 = (th + s2).cos();
            y0 = (th + s2).sin();
            // The face is two triangles, and every one of their six vertices
            // carries the normal of the middle of this face, which is what
            // makes it read as a flat facet rather than a curve.
            g.vertex3f(nx, 1.0, ny); /* top point B */
            g.vertex3f(x, 0.0, y); /* bottom point A */
            g.vertex3f(nx, 1.0, ny); /* top point B */
            g.vertex3f(nx, 0.0, ny); /* bottom point B */
            polys += 1;
        }
        x = nx;
        y = ny;
        polys += 1;
    }
    g.end();

    if caps {
        for z in 0..=1 {
            let z = z as f32;
            g.normal3f(0.0, if z == 0.0 { -1.0 } else { 1.0 }, 0.0);
            g.begin(if wire {
                Shape::LineLoop
            } else {
                Shape::TriangleFan
            });
            if !wire {
                g.vertex3f(0.0, z, 0.0);
            }
            // The far cap is wound the other way round, so both caps face out.
            // Upstream's smooth walls close the ring by incrementing `faces`,
            // and the cap loop then reads the incremented count and lays down
            // one point past the seam. This closes exactly instead; the extra
            // sliver covered no pixels either way.
            let mut th = 0.0f32;
            for _ in 0..=faces {
                g.vertex3f(th.cos(), z, th.sin());
                th += if z == 0.0 { step } else { -step };
                polys += 1;
            }
            g.end();
        }
    }
    polys
}

/// `unit_cone`: radius 1 at `y = 0`, meeting at a point at `y = 1`.
pub fn unit_cone(g: &mut Glx, faces: i32, smooth: bool, cap: bool, wire: bool) -> i32 {
    let faces = faces.max(3);
    let step = std::f32::consts::PI * 2.0 / faces as f32;
    let s2 = step / 2.0;
    let mut polys = 0;

    let (mut th, mut x, mut y) = (0.0f32, 1.0f32, 0.0f32);
    let (mut x0, mut y0) = (s2.cos(), s2.sin());

    g.front_face_cw(false);
    g.begin(if wire { Shape::Lines } else { Shape::Triangles });
    for _ in 0..faces {
        if smooth {
            g.normal3f(x, 0.0, y);
        } else {
            g.normal3f(x0, 0.0, y0);
        }
        g.vertex3f(x, 0.0, y); /* bottom point A */

        // The tip always takes the face's normal, smooth or not: there is no
        // one outward direction at the point of a cone.
        g.normal3f(x0, 0.0, y0);
        g.vertex3f(0.0, 1.0, 0.0); /* tip point */

        th += step;
        x0 = (th + s2).cos();
        y0 = (th + s2).sin();
        x = th.cos();
        y = th.sin();

        if smooth {
            g.normal3f(x, 0.0, y);
        } else {
            g.normal3f(x0, 0.0, y0);
        }
        g.vertex3f(x, 0.0, y); /* bottom point B */
        polys += 1;
    }
    g.end();

    if cap {
        g.normal3f(0.0, -1.0, 0.0);
        g.begin(if wire {
            Shape::LineLoop
        } else {
            Shape::TriangleFan
        });
        if !wire {
            g.vertex3f(0.0, 0.0, 0.0);
        }
        let mut th = 0.0f32;
        for _ in 0..=faces {
            g.vertex3f(th.cos(), 0.0, th.sin());
            th += step;
            polys += 1;
        }
        g.end();
    }
    polys
}

/// `tube`: a cylinder from one point to another.
///
/// `cap_size` extends both ends by that much along the axis, which is how a
/// saver makes two tubes meeting at an angle look like one bent pipe rather
/// than two cut ends.
#[allow(clippy::too_many_arguments)]
pub fn tube(
    g: &mut Glx,
    from: [f32; 3],
    to: [f32; 3],
    diameter: f32,
    cap_size: f32,
    faces: i32,
    smooth: bool,
    caps: bool,
    wire: bool,
) -> i32 {
    tube_1(
        g, from, to, diameter, cap_size, faces, smooth, caps, wire, false,
    )
}

/// `cone`: the same, tapering to a point at `to`.
#[allow(clippy::too_many_arguments)]
pub fn cone(
    g: &mut Glx,
    from: [f32; 3],
    to: [f32; 3],
    diameter: f32,
    cap_size: f32,
    faces: i32,
    smooth: bool,
    cap: bool,
    wire: bool,
) -> i32 {
    tube_1(
        g, from, to, diameter, cap_size, faces, smooth, cap, wire, true,
    )
}

/// A tube's geometry, built once and then transformed into place.
///
/// [`tube`] puts each cylinder where it goes by pushing a matrix, which is what
/// the C does and is the right thing for the savers that draw a few dozen of
/// them. It is the wrong thing for one that draws thousands: a matrix change
/// starts a new batch, and `highvoltage` traces each of its towers out of some
/// six hundred tubes, which at a draw call apiece is more than the whole frame
/// can afford. So keep the unit cylinder as a flat list of triangles and
/// transform it on the way out instead. Every tube drawn between two matrix
/// changes then lands in a single draw call.
///
/// Two things are lost by not going through the matrix stack, neither of which
/// has yet mattered: the winding is whatever the caller set rather than being
/// forced counter-clockwise, and a caller reading the matrix back would not see
/// the tube's own transform.
pub struct TubeMesh {
    /// Position and normal, in the order they are drawn.
    verts: Vec<([f32; 3], [f32; 3])>,
    shape: Shape,
    polys: i32,
}

impl TubeMesh {
    /// The unit tube, recorded once and flattened into a list that can be
    /// concatenated: a strip or a fan cannot be joined to the next one without
    /// growing triangles nobody asked for, so they are cut up here.
    pub fn tube(faces: i32, smooth: bool, caps: bool, wire: bool) -> Self {
        Self::build(|g| unit_tube(g, faces, smooth, caps, wire), wire)
    }

    /// The same for a cone.
    pub fn cone(faces: i32, smooth: bool, cap: bool, wire: bool) -> Self {
        Self::build(|g| unit_cone(g, faces, smooth, cap, wire), wire)
    }

    fn build(f: impl FnOnce(&mut Glx) -> i32, wire: bool) -> Self {
        use super::gl::Primitive;
        let mut g = Glx::new();
        g.start_frame(1, 1);
        let polys = f(&mut g);
        let frame = g.frame();
        let mut verts = Vec::new();
        let mut push = |v: &super::gl::Vertex| verts.push((v.pos, v.normal));
        for b in &frame.batches {
            let vs = &frame.vertices[b.first..b.first + b.count];
            match b.primitive {
                Primitive::Triangles | Primitive::Lines => vs.iter().for_each(&mut push),
                Primitive::TriangleStrip => {
                    for (i, w) in vs.windows(3).enumerate() {
                        // Every other triangle in a strip is wound the other
                        // way round, so swap two of its corners back.
                        let o = if i % 2 == 0 { [0, 1, 2] } else { [0, 2, 1] };
                        o.iter().for_each(|&k| push(&w[k]));
                    }
                }
                Primitive::TriangleFan => {
                    for w in vs[1..].windows(2) {
                        push(&vs[0]);
                        w.iter().for_each(&mut push);
                    }
                }
                Primitive::LineStrip | Primitive::LineLoop => {
                    for w in vs.windows(2) {
                        w.iter().for_each(&mut push);
                    }
                    if b.primitive == Primitive::LineLoop && vs.len() > 2 {
                        push(&vs[vs.len() - 1]);
                        push(&vs[0]);
                    }
                }
                Primitive::Points => vs.iter().for_each(&mut push),
            }
        }
        TubeMesh {
            verts,
            shape: if wire { Shape::Lines } else { Shape::Triangles },
            polys,
        }
    }

    /// Draw the tube from one point to another, with the same arguments
    /// [`tube`] takes and the same result.
    pub fn draw(
        &self,
        g: &mut Glx,
        from: [f32; 3],
        to: [f32; 3],
        diameter: f32,
        cap_size: f32,
    ) -> i32 {
        if diameter <= 0.0 {
            return 0;
        }
        let (x, y, z) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
        if x == 0.0 && y == 0.0 && z == 0.0 {
            return 0;
        }
        let length = (x * x + y * y + z * z).sqrt();
        let h = (x * x + y * y).sqrt();

        // The two turns [`tube`] makes with the matrix stack, written out: one
        // about z to bring the tube's y axis into the plane of the target, then
        // one about x to lay it along it.
        let (sa, ca) = if h == 0.0 {
            (0.0, 1.0)
        } else {
            (-x / h, y / h)
        };
        let (sb, cb) = (z / length, h / length);
        let rot = |p: [f32; 3]| {
            [
                ca * p[0] - sa * cb * p[1] + sa * sb * p[2],
                sa * p[0] + ca * cb * p[1] - ca * sb * p[2],
                sb * p[1] + cb * p[2],
            ]
        };

        // And the same extension of both ends by the cap size.
        let c = if cap_size != 0.0 {
            cap_size / length
        } else {
            0.0
        };
        let (sx, sy) = (diameter, length * (1.0 + c + c));

        g.begin(self.shape);
        for (p, n) in &self.verts {
            let q = rot([p[0] * sx, (p[1] * (1.0 + c + c) - c) * length, p[2] * sx]);
            let m = rot([n[0] / sx, n[1] / sy, n[2] / sx]);
            let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
            let m = if len > 0.0 {
                [m[0] / len, m[1] / len, m[2] / len]
            } else {
                m
            };
            g.normal3f(m[0], m[1], m[2]);
            g.vertex3f(q[0] + from[0], q[1] + from[1], q[2] + from[2]);
        }
        g.end();
        self.polys
    }
}

#[allow(clippy::too_many_arguments)]
fn tube_1(
    g: &mut Glx,
    from: [f32; 3],
    to: [f32; 3],
    diameter: f32,
    cap_size: f32,
    faces: i32,
    smooth: bool,
    caps: bool,
    wire: bool,
    cone_p: bool,
) -> i32 {
    if diameter <= 0.0 {
        return 0;
    }
    let (x, y, z) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    if x == 0.0 && y == 0.0 && z == 0.0 {
        return 0;
    }
    let length = (x * x + y * y + z * z).sqrt();
    let deg = 180.0 / std::f32::consts::PI;

    g.push_matrix();
    g.translate(from[0], from[1], from[2]);
    // Two turns rather than a basis: about z to bring the y axis into the
    // plane of the target, then about x to lay it along it.
    g.rotate(-x.atan2(y) * deg, 0.0, 0.0, 1.0);
    g.rotate(z.atan2((x * x + y * y).sqrt()) * deg, 1.0, 0.0, 0.0);
    g.scale(diameter, length, diameter);

    /* extend the endpoints of the tube by the cap size in both directions */
    if cap_size != 0.0 {
        let c = cap_size / length;
        g.translate(0.0, -c, 0.0);
        g.scale(1.0, 1.0 + c + c, 1.0);
    }

    let polys = if cone_p {
        unit_cone(g, faces, smooth, caps, wire)
    } else {
        unit_tube(g, faces, smooth, caps, wire)
    };
    g.pop_matrix();
    polys
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The baked tube is the same tube: the same points, in the same places,
    /// only with the transform in the vertices rather than in the matrix.
    #[test]
    fn a_baked_tube_lands_where_the_matrix_would_put_it() {
        let round = |p: [f32; 3]| {
            let mut v: Vec<i32> = p.iter().map(|c| (c * 1000.0).round() as i32).collect();
            v.push(0);
            v
        };
        for (smooth, caps, cap_size) in [(false, false, 0.0), (true, true, 0.25)] {
            let from = [1.0, -2.0, 0.5];
            let to = [-3.0, 4.0, 2.0];

            let mut a = Glx::new();
            a.start_frame(100, 100);
            let pa = tube(&mut a, from, to, 0.5, cap_size, 6, smooth, caps, false);
            let fa = a.frame();
            let mut va: Vec<Vec<i32>> = fa
                .batches
                .iter()
                .flat_map(|b| {
                    fa.vertices[b.first..b.first + b.count]
                        .iter()
                        .map(|v| round(b.modelview.transform(v.pos)))
                })
                .collect();

            let mut b = Glx::new();
            b.start_frame(100, 100);
            let mesh = TubeMesh::tube(6, smooth, caps, false);
            let pb = mesh.draw(&mut b, from, to, 0.5, cap_size);
            let fb = b.frame();
            let mut vb: Vec<Vec<i32>> = fb
                .batches
                .iter()
                .flat_map(|b2| {
                    fb.vertices[b2.first..b2.first + b2.count]
                        .iter()
                        .map(|v| round(b2.modelview.transform(v.pos)))
                })
                .collect();

            assert_eq!(pa, pb, "different polygon counts");
            va.sort();
            va.dedup();
            vb.sort();
            vb.dedup();
            assert_eq!(va, vb, "the baked tube is somewhere else");
            // And it is one block rather than three, which is the point.
            assert_eq!(fb.batches.len(), 1, "the baked tube is not one draw");
        }
    }

    /// A unit tube is a unit tube: everything one from the axis, between the
    /// two ends.
    #[test]
    fn a_unit_tube_is_a_unit_tube() {
        for smooth in [false, true] {
            let mut g = Glx::new();
            g.start_frame(100, 100);
            unit_tube(&mut g, 12, smooth, false, false);
            let f = g.frame();
            assert!(!f.vertices.is_empty());
            for v in &f.vertices {
                let r = (v.pos[0] * v.pos[0] + v.pos[2] * v.pos[2]).sqrt();
                assert!((r - 1.0).abs() < 1e-5, "{:?} is {r} from the axis", v.pos);
                assert!(v.pos[1] == 0.0 || v.pos[1] == 1.0, "{:?}", v.pos);
            }
        }
    }

    /// Smooth and flat differ in their normals and not in their shape: a
    /// smooth wall's normal is where the vertex is, a flat one's is not.
    #[test]
    fn smooth_normals_point_at_their_own_vertex() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        unit_tube(&mut g, 12, true, false, false);
        for v in &g.frame().vertices {
            assert!((v.normal[0] - v.pos[0]).abs() < 1e-5);
            assert!((v.normal[2] - v.pos[2]).abs() < 1e-5);
            assert_eq!(v.normal[1], 0.0, "a wall normal never points along it");
        }

        let mut g = Glx::new();
        g.start_frame(100, 100);
        unit_tube(&mut g, 12, false, false, false);
        let f = g.frame();
        let off = f
            .vertices
            .iter()
            .any(|v| (v.normal[0] - v.pos[0]).abs() > 0.01);
        assert!(off, "a flat wall shares one normal across each face");
    }

    /// A cone comes to a point at one end and is open at the other.
    #[test]
    fn a_cone_comes_to_a_point() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        unit_cone(&mut g, 12, true, false, false);
        let f = g.frame();
        let tips = f
            .vertices
            .iter()
            .filter(|v| v.pos == [0.0, 1.0, 0.0])
            .count();
        assert_eq!(tips, 12, "one tip vertex per face");
    }

    /// Capping a tube closes both ends, so there is geometry in the middle of
    /// each of them where there was none.
    #[test]
    fn caps_close_the_ends() {
        let count = |caps| {
            let mut g = Glx::new();
            g.start_frame(100, 100);
            unit_tube(&mut g, 12, true, caps, false);
            let f = g.frame();
            f.vertices
                .iter()
                .filter(|v| v.pos[0] == 0.0 && v.pos[2] == 0.0)
                .count()
        };
        assert_eq!(count(false), 0);
        assert_eq!(count(true), 2, "the middle of each end");
    }

    /// And the point of the whole module: a tube goes where it is told, with
    /// the radius it was told.
    #[test]
    fn a_tube_runs_between_the_two_points() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        tube(
            &mut g,
            [1.0, 2.0, 3.0],
            [1.0, 2.0, 9.0],
            0.5,
            0.0,
            8,
            true,
            true,
            false,
        );
        let f = g.frame();
        let mv = f.batches[0].modelview;
        for v in &f.vertices {
            let p = mv.transform(v.pos);
            // Along z from 3 to 9, half a unit off the axis at (1, 2).
            assert!((3.0..=9.0).contains(&p[2]), "{p:?}");
            let r = ((p[0] - 1.0).powi(2) + (p[1] - 2.0).powi(2)).sqrt();
            assert!(r <= 0.5 + 1e-5, "{p:?} is {r} from the axis");
        }
        // It really reaches both ends.
        let zs: Vec<f32> = f.vertices.iter().map(|v| mv.transform(v.pos)[2]).collect();
        assert!(zs.iter().any(|z| (z - 3.0).abs() < 1e-4));
        assert!(zs.iter().any(|z| (z - 9.0).abs() < 1e-4));
    }

    /// A tube with no length is nothing, not a division by zero.
    #[test]
    fn a_tube_of_no_length_draws_nothing() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        let n = tube(
            &mut g,
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            1.0,
            0.0,
            8,
            true,
            true,
            false,
        );
        assert_eq!(n, 0);
        assert!(g.frame().batches.is_empty());
    }
}
