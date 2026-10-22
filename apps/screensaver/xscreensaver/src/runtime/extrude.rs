//! Sweeping a flat outline along a path in space.
//!
//! `extrusion` draws everything through GLE, the tubing and extrusion library,
//! which XScreenSaver links against rather than bundling. That is what kept
//! the saver on the blocked list, and it was the wrong thing to measure: what
//! the saver needs is the *geometry*, not the library, and the slice of GLE it
//! reaches for is narrow.
//!
//! Two facts make it narrow. The only join style any of the seven shapes asks
//! for is `TUBE_JN_ANGLE`, so none of GLE's cut, round or raw join machinery
//! is wanted. And the named shapes are thin wrappers: `gleHelicoid` is
//! `gleSpiral` with a circular contour, and `gleSpiral` generates a helical
//! path with a per-station transform and hands it to the ordinary extrusion.
//! So there is one real routine here, [`extrude`], and the rest arrange its
//! arguments.
//!
//! # How a corner is mitred
//!
//! The interesting part, and the reason this is not just a stack of
//! transformed copies of the outline. At each station the outline sits in the
//! plane bisecting the angle between the segment arriving and the segment
//! leaving, so consecutive segments meet along a shared ring with no gap on
//! the outside of the bend and no overlap on the inside. GLE finds that ring
//! by running a line through each outline point parallel to the segment and
//! intersecting it with the bisecting plane, which is what the arithmetic in
//! [`extrude`] is doing.
//!
//! The outline's orientation is carried along the path by reflecting an "up"
//! vector in each bisecting plane as it goes, rather than by parallel
//! transport, which is upstream's choice and gives the same twist on a helix.

use super::gl::{Glx, Shape};

/// A 2 by 3 affine transform of the outline, as GLE's `gleAffine`: the outline
/// point is a column vector and this is applied to it at one station.
pub type Affine = [[f64; 3]; 2];

/// The identity, for a station that is not deformed.
pub const IDENTITY: Affine = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(v: [f64; 3]) -> [f64; 3] {
    let l = dot(v, v).sqrt();
    if l < 1e-12 {
        [0.0, 0.0, 0.0]
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

/// `BISECTING_PLANE`: the normal of the plane bisecting the angle at `v2`.
///
/// It is the sum of the two unit direction vectors, which points along the
/// bisector of the *outside* of the angle and so is normal to the plane that
/// splits it. A degenerate leg falls back to the other one's direction.
fn bisecting_plane(v1: [f64; 3], v2: [f64; 3], v3: [f64; 3]) -> [f64; 3] {
    let d21 = sub(v2, v1);
    let d32 = sub(v3, v2);
    let (l21, l32) = (dot(d21, d21).sqrt(), dot(d32, d32).sqrt());
    if l21 < 1e-9 {
        return norm(d32);
    }
    if l32 < 1e-9 {
        return norm(d21);
    }
    let n = [
        d21[0] / l21 + d32[0] / l32,
        d21[1] / l21 + d32[1] / l32,
        d21[2] / l21 + d32[2] / l32,
    ];
    if dot(n, n) < 1e-18 {
        // The path doubles straight back on itself: the bisecting plane is
        // the one across the segment.
        return norm(d21);
    }
    norm(n)
}

/// `VEC_REFLECT`: reflect `v` in the plane with normal `n`.
fn reflect(v: [f64; 3], n: [f64; 3]) -> [f64; 3] {
    let d = 2.0 * dot(v, n);
    [v[0] - d * n[0], v[1] - d * n[1], v[2] - d * n[2]]
}

/// What to sweep, and along what.
pub struct Extrusion<'a> {
    /// The outline, in its own plane.
    pub contour: &'a [[f64; 2]],
    /// One normal per outline segment, if the shape is to be lit.
    pub normals: Option<&'a [[f64; 2]]>,
    /// Which way is up for the outline. `None` means the y axis.
    pub up: Option<[f64; 3]>,
    /// The path. Its first and last points are not drawn: they say which way
    /// the ends are pointing, which is GLE's convention and what the shape
    /// generators here rely on.
    pub path: &'a [[f64; 3]],
    /// A colour per station, if the shape is not one solid colour.
    pub colors: Option<&'a [[f32; 3]]>,
    /// A transform of the outline per station, for the shapes that taper or
    /// twist as they go.
    pub xforms: Option<&'a [Affine]>,
}

/// Apply a 2 by 3 affine to an outline point.
fn xform_point(m: &Affine, p: [f64; 2]) -> [f64; 2] {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2],
    ]
}

/// Apply the 2 by 2 part to a normal. GLE uses the inverse transpose so that
/// a squashed shape still lights correctly.
fn xform_normal(m: &Affine, n: [f64; 2]) -> [f64; 2] {
    let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    if det.abs() < 1e-12 {
        return n;
    }
    [
        (m[1][1] * n[0] - m[1][0] * n[1]) / det,
        (-m[0][1] * n[0] + m[0][0] * n[1]) / det,
    ]
}

/// `extrusion_angle_join`: sweep the outline along the path, mitring the
/// corners, and draw it.
pub fn extrude(g: &mut Glx, e: &Extrusion) {
    let ncp = e.contour.len();
    let n = e.path.len();
    if ncp < 2 || n < 4 {
        return;
    }

    let mut yup = e.up.unwrap_or([0.0, 1.0, 0.0]);
    let mut bi0 = bisecting_plane(e.path[0], e.path[1], e.path[2]);
    yup = reflect(yup, bi0);

    for i in 1..n - 2 {
        let inext = i + 1;
        let seg = sub(e.path[inext], e.path[i]);
        let len_seg = dot(seg, seg).sqrt();
        if len_seg < 1e-12 {
            continue;
        }
        let bi1 = bisecting_plane(e.path[i], e.path[inext], e.path[inext + 1]);

        // The local frame: the origin at this station, local -z down the
        // segment, local y along the carried up vector.
        let d = norm(seg);
        let zaxis = [-d[0], -d[1], -d[2]];
        let up_par = dot(yup, zaxis);
        let yaxis = norm([
            yup[0] - up_par * zaxis[0],
            yup[1] - up_par * zaxis[1],
            yup[2] - up_par * zaxis[2],
        ]);
        let xaxis = cross3(yaxis, zaxis);
        let to_world = |p: [f64; 3]| {
            [
                (e.path[i][0] + xaxis[0] * p[0] + yaxis[0] * p[1] + zaxis[0] * p[2]) as f32,
                (e.path[i][1] + xaxis[1] * p[0] + yaxis[1] * p[1] + zaxis[1] * p[2]) as f32,
                (e.path[i][2] + xaxis[2] * p[0] + yaxis[2] * p[1] + zaxis[2] * p[2]) as f32,
            ]
        };
        // The bisecting planes, rotated into the local frame.
        let into_local = |v: [f64; 3]| [dot(v, xaxis), dot(v, yaxis), dot(v, zaxis)];
        let b0 = into_local(bi0);
        let b1 = into_local(bi1);

        let xf0 = e.xforms.and_then(|x| x.get(i.saturating_sub(1)));
        let xf1 = e.xforms.and_then(|x| x.get(i));

        let colour = |k: usize| e.colors.and_then(|c| c.get(k)).copied();

        g.begin(Shape::TriangleStrip);
        for j in 0..=ncp {
            let jj = j % ncp;
            let cp = e.contour[jj];
            let p0 = xf0.map_or(cp, |m| xform_point(m, cp));
            let p1 = xf1.map_or(cp, |m| xform_point(m, cp));

            // Where the line through the outline point, parallel to the
            // segment, meets each bisecting plane. The front plane passes
            // through the origin and the back one through (0, 0, -len).
            let z0 = if b0[2].abs() < 1e-9 {
                0.0
            } else {
                -(b0[0] * p0[0] + b0[1] * p0[1]) / b0[2]
            };
            let z1 = if b1[2].abs() < 1e-9 {
                -len_seg
            } else {
                -len_seg - (b1[0] * p1[0] + b1[1] * p1[1]) / b1[2]
            };

            if let Some(ns) = e.normals {
                let nn = ns[jj.min(ns.len() - 1)];
                let nf = xf0.map_or(nn, |m| xform_normal(m, nn));
                let world = [
                    (xaxis[0] * nf[0] + yaxis[0] * nf[1]) as f32,
                    (xaxis[1] * nf[0] + yaxis[1] * nf[1]) as f32,
                    (xaxis[2] * nf[0] + yaxis[2] * nf[1]) as f32,
                ];
                g.normal3f(world[0], world[1], world[2]);
            }
            if let Some(c) = colour(i - 1) {
                g.color3f(c[0], c[1], c[2]);
            }
            let v = to_world([p0[0], p0[1], z0]);
            g.vertex3f(v[0], v[1], v[2]);

            if let Some(c) = colour(i) {
                g.color3f(c[0], c[1], c[2]);
            }
            let v = to_world([p1[0], p1[1], z1]);
            g.vertex3f(v[0], v[1], v[2]);
        }
        g.end();

        bi0 = bi1;
        yup = reflect(yup, bi0);
    }
}

/// `gleSpiral`: a helical path with an optional transform that changes as it
/// goes, swept with the given outline.
///
/// The radius and height change per *revolution*, which is why both are
/// scaled by the fraction of a turn each step covers.
#[allow(clippy::too_many_arguments)]
pub fn spiral(
    g: &mut Glx,
    contour: &[[f64; 2]],
    normals: Option<&[[f64; 2]]>,
    up: Option<[f64; 3]>,
    start_radius: f64,
    drd_theta: f64,
    start_z: f64,
    dzd_theta: f64,
    start_xform: Option<Affine>,
    dxform_d_theta: Option<Affine>,
    start_theta: f64,
    sweep_theta: f64,
) {
    const SLICES: usize = 20;
    let npoints = ((SLICES as f64 / 360.0) * sweep_theta.abs()) as usize + 4;
    let delta_angle = sweep_theta.to_radians() / (npoints - 3) as f64;
    let mut theta = start_theta.to_radians() - delta_angle;

    // The differentials are per revolution, so they are scaled by how much of
    // one a step is. The first point is hidden, so both back-step.
    let delta = delta_angle / std::f64::consts::TAU;
    let dz = dzd_theta * delta;
    let dr = drd_theta * delta;
    let mut z = start_z - dz;
    let mut r = start_radius - dr;

    let mut path = Vec::with_capacity(npoints);
    for _ in 0..npoints {
        path.push([r * theta.cos(), r * theta.sin(), z]);
        z += dz;
        r += dr;
        theta += delta_angle;
    }

    let xforms: Option<Vec<Affine>> = start_xform.map(|start| {
        let mut out = Vec::with_capacity(npoints);
        let mut m = start;
        for _ in 0..npoints {
            out.push(m);
            if let Some(d) = dxform_d_theta {
                // Upstream exponentiates the tangent matrix; a step is small
                // enough that one term of the series is what it comes to.
                let step = [
                    [d[0][0] * delta, d[0][1] * delta, d[0][2] * delta],
                    [d[1][0] * delta, d[1][1] * delta, d[1][2] * delta],
                ];
                let a = [
                    [1.0 + step[0][0], step[0][1]],
                    [step[1][0], 1.0 + step[1][1]],
                ];
                m = [
                    [
                        a[0][0] * m[0][0] + a[0][1] * m[1][0],
                        a[0][0] * m[0][1] + a[0][1] * m[1][1],
                        m[0][2] + step[0][2],
                    ],
                    [
                        a[1][0] * m[0][0] + a[1][1] * m[1][0],
                        a[1][0] * m[0][1] + a[1][1] * m[1][1],
                        m[1][2] + step[1][2],
                    ],
                ];
            }
        }
        out
    });

    extrude(
        g,
        &Extrusion {
            contour,
            normals,
            up,
            path: &path,
            colors: None,
            xforms: xforms.as_deref(),
        },
    );
}

/// A circle of `n` points, and its outward normals: the outline every helical
/// shape is swept from.
pub fn circle(n: usize, radius: f64) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    let mut pts = Vec::with_capacity(n);
    let mut nrm = Vec::with_capacity(n);
    for i in 0..n {
        let a = std::f64::consts::TAU * i as f64 / n as f64;
        nrm.push([a.cos(), a.sin()]);
        pts.push([radius * a.cos(), radius * a.sin()]);
    }
    (pts, nrm)
}

/// `gleHelicoid`: a tube of circular cross-section wound along a spiral.
#[allow(clippy::too_many_arguments)]
pub fn helicoid(
    g: &mut Glx,
    r_toroid: f64,
    start_radius: f64,
    drd_theta: f64,
    start_z: f64,
    dzd_theta: f64,
    start_xform: Option<Affine>,
    dxform_d_theta: Option<Affine>,
    start_theta: f64,
    sweep_theta: f64,
) {
    let (pts, nrm) = circle(20, r_toroid);
    spiral(
        g,
        &pts,
        Some(&nrm),
        Some([1.0, 0.0, 0.0]), /* up along x, as super_helix sets */
        start_radius,
        drd_theta,
        start_z,
        dzd_theta,
        start_xform,
        dxform_d_theta,
        start_theta,
        sweep_theta,
    );
}

/// `gleScrew`: a shape swept straight along z while turning, which is a
/// spiral of zero radius.
pub fn screw(
    g: &mut Glx,
    contour: &[[f64; 2]],
    normals: Option<&[[f64; 2]]>,
    up: Option<[f64; 3]>,
    startz: f64,
    endz: f64,
    twist: f64,
) {
    // A straight path with a twist applied per station, which is what a screw
    // is: upstream builds it as a lathe with no radius.
    const N: usize = 40;
    let mut path = Vec::with_capacity(N + 4);
    let mut xforms = Vec::with_capacity(N + 4);
    let step = (endz - startz) / N as f64;
    for i in 0..N + 4 {
        let k = i as f64 - 1.0;
        path.push([0.0, 0.0, startz + step * k]);
        let a = (twist * k / N as f64).to_radians();
        xforms.push([[a.cos(), -a.sin(), 0.0], [a.sin(), a.cos(), 0.0]]);
    }
    extrude(
        g,
        &Extrusion {
            contour,
            normals,
            up,
            path: &path,
            colors: None,
            xforms: Some(&xforms),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bisecting plane of a right angle points along the outside of the
    /// bend, which is what makes the mitre come out symmetric.
    #[test]
    fn a_right_angle_bisects_at_forty_five_degrees() {
        let n = bisecting_plane([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]);
        let s = 0.5f64.sqrt();
        assert!((n[0] - s).abs() < 1e-12, "{n:?}");
        assert!((n[1] - s).abs() < 1e-12, "{n:?}");
        assert!(n[2].abs() < 1e-12);

        // A straight path bisects across the direction of travel.
        let n = bisecting_plane([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
        assert!((n[0] - 1.0).abs() < 1e-12, "{n:?}");
    }

    /// Reflecting twice in the same plane is the identity, which is what
    /// keeps the outline's orientation from drifting along a straight path.
    #[test]
    fn reflecting_twice_returns_the_vector() {
        let n = norm([1.0, 2.0, -3.0]);
        let v = [0.3, -0.7, 0.2];
        let r = reflect(reflect(v, n), n);
        for i in 0..3 {
            assert!((r[i] - v[i]).abs() < 1e-12, "{r:?} vs {v:?}");
        }
    }

    /// A straight path with a circular outline is a cylinder: every vertex
    /// lands at the outline's radius from the axis, and the sweep spans the
    /// path's drawn length.
    #[test]
    fn a_straight_sweep_is_a_cylinder() {
        let mut g = Glx::new();
        g.start_frame(64, 64);
        let (pts, nrm) = circle(16, 2.0);
        // Six stations, of which the first and last are only there to say
        // which way the ends point.
        let path: Vec<[f64; 3]> = (0..6).map(|i| [0.0, 0.0, f64::from(i)]).collect();
        extrude(
            &mut g,
            &Extrusion {
                contour: &pts,
                normals: Some(&nrm),
                up: Some([1.0, 0.0, 0.0]),
                path: &path,
                colors: None,
                xforms: None,
            },
        );
        let f = g.frame();
        assert!(!f.vertices.is_empty(), "nothing was swept");
        let (mut zmin, mut zmax) = (f64::MAX, f64::MIN);
        for v in &f.vertices {
            let r = (f64::from(v.pos[0]).powi(2) + f64::from(v.pos[1]).powi(2)).sqrt();
            assert!((r - 2.0).abs() < 1e-4, "a vertex sits at radius {r}");
            zmin = zmin.min(f64::from(v.pos[2]));
            zmax = zmax.max(f64::from(v.pos[2]));
        }
        // Stations 1 to 4 are drawn: the first and last are control points.
        assert!((zmin - 1.0).abs() < 1e-4, "starts at {zmin}");
        assert!((zmax - 4.0).abs() < 1e-4, "ends at {zmax}");
    }

    /// A mitred corner shares its ring: the last vertices of one segment are
    /// the first of the next, so there is no gap on the outside of the bend
    /// and no overlap on the inside.
    #[test]
    fn a_corner_shares_one_ring() {
        let mut g = Glx::new();
        g.start_frame(64, 64);
        let (pts, nrm) = circle(8, 1.0);
        // A right-angle bend in the middle.
        let path = [
            [0.0, 0.0, -2.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 2.0],
            [2.0, 0.0, 2.0],
            [4.0, 0.0, 2.0],
        ];
        extrude(
            &mut g,
            &Extrusion {
                contour: &pts,
                normals: Some(&nrm),
                up: Some([1.0, 0.0, 0.0]),
                path: &path,
                colors: None,
                xforms: None,
            },
        );
        let f = g.frame();
        // Two segments were drawn, so two strips.
        assert_eq!(f.batches.len(), 2, "expected one strip a segment");
        let n = pts.len() + 1;
        // The back ring of the first strip and the front ring of the second
        // are the same points: they are the shared mitre ring.
        let a = &f.vertices[..2 * n];
        let b = &f.vertices[2 * n..];
        for k in 0..n {
            let back = a[2 * k + 1].pos;
            let front = b[2 * k].pos;
            for c in 0..3 {
                assert!(
                    (back[c] - front[c]).abs() < 1e-4,
                    "ring point {k} differs: {back:?} vs {front:?}"
                );
            }
        }
    }

    /// A helicoid winds: it goes round, and it climbs.
    #[test]
    fn a_helicoid_goes_round_and_up() {
        let mut g = Glx::new();
        g.start_frame(64, 64);
        helicoid(&mut g, 0.3, 2.0, 0.0, -1.0, 1.0, None, None, 0.0, 720.0);
        let f = g.frame();
        assert!(!f.vertices.is_empty());
        let (mut zmin, mut zmax) = (f64::MAX, f64::MIN);
        let (mut rmin, mut rmax) = (f64::MAX, f64::MIN);
        for v in &f.vertices {
            let r = (f64::from(v.pos[0]).powi(2) + f64::from(v.pos[1]).powi(2)).sqrt();
            zmin = zmin.min(f64::from(v.pos[2]));
            zmax = zmax.max(f64::from(v.pos[2]));
            rmin = rmin.min(r);
            rmax = rmax.max(r);
        }
        // Two turns at one unit of height each.
        assert!(zmax - zmin > 1.5, "it climbed {}", zmax - zmin);
        // The tube is 0.3 thick around a radius of 2.
        assert!((rmax - 2.3).abs() < 0.2, "outer radius {rmax}");
        assert!((rmin - 1.7).abs() < 0.2, "inner radius {rmin}");
    }

    /// Nothing degenerate panics: a path too short to draw, an outline of one
    /// point, a path that doubles back on itself.
    #[test]
    fn degenerate_sweeps_are_survivable() {
        let mut g = Glx::new();
        g.start_frame(64, 64);
        let (pts, nrm) = circle(6, 1.0);
        for path in [
            vec![],
            vec![[0.0, 0.0, 0.0]],
            vec![[0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3]],
            vec![
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0],
            ],
        ] {
            extrude(
                &mut g,
                &Extrusion {
                    contour: &pts,
                    normals: Some(&nrm),
                    up: None,
                    path: &path,
                    colors: None,
                    xforms: None,
                },
            );
        }
        // And an outline with too few points draws nothing rather than
        // indexing off the end of itself.
        extrude(
            &mut g,
            &Extrusion {
                contour: &[[0.0, 0.0]],
                normals: None,
                up: None,
                path: &[[0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0, 2.0], [0.0, 0.0, 3.0]],
                colors: None,
                xforms: None,
            },
        );
    }
}
