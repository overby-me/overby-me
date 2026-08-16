//! Port of `hacks/glx/mirrorblob.c`.
//!
//! ```text
//! mirrorblob  Copyright (c) 2003 Jon Dowdall <jon.dowdall@bigpond.com>
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
//! The blob was inspired by a lavalamp is in no way a simulation.  The code
//! is just an attempt to generate some eye-candy.
//! ```
//!
//! A wobbly blob distorts the image behind it.
//!
//! The blob starts as a sphere built by subdividing a tetrahedron and pushing
//! every vertex out to unit length, which gives a mesh with no poles and no
//! seam. A handful of invisible bumps then drift about just outside it, each
//! pushing the surface out or pulling it in by an amount that falls away with
//! distance, and each bump's position, strength and width is itself a mass on
//! a spring drifting towards a target. Nothing about it is a simulation of
//! anything; it just moves like something soft.
//!
//! The reflection is a sphere map worked out from each vertex's normal, so the
//! picture behind the blob appears smeared over it as though it were chrome.
//! With `offsetTexture` it instead traces where the eye would go after
//! bouncing off the surface, out to a notional cube a hundred units across,
//! which bends the picture much harder.
//!
//! Two deliberate differences from upstream, both following what its own
//! OpenGL ES build does. Wireframe is `glPolygonMode`, which that build has
//! not got either, so `set_parameters` turns it off outright and so does this.
//! And upstream asks for `GL_SEPARATE_SPECULAR_COLOR`, which adds the
//! highlight *after* the texture instead of letting the texture multiply it;
//! this shader modulates everything, so a highlight here is tinted by whatever
//! part of the picture it lands on rather than staying white.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    screenhack_event_helper,
};

/// How finely the bump strength is tabulated.
const BUMP_ARRAY_SIZE: usize = 1024;
/// Two, so one picture can fade into the next.
const NUM_TEXTURES: usize = 2;

type V3 = [f64; 3];

fn dot(u: V3, v: V3) -> f64 {
    u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
}

fn cross(u: V3, v: V3) -> V3 {
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

fn sub(u: V3, v: V3) -> V3 {
    [u[0] - v[0], u[1] - v[1], u[2] - v[2]]
}

fn add(u: &mut V3, v: V3) {
    for i in 0..3 {
        u[i] += v[i];
    }
}

fn scale(v: V3, s: f64) -> V3 {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn normalise(v: V3) -> V3 {
    let m = dot(v, v).sqrt();
    if m > 1e-300 {
        scale(v, 1.0 / m)
    } else {
        [0.0; 3]
    }
}

/// The four corners of a tetrahedron, as the three corners of each of its
/// faces. Subdividing these and pushing every point out to unit length is what
/// makes the sphere.
const SQRT_3: f64 = 0.577_350_269_2;
const TETRAHEDRON: [[V3; 3]; 4] = [
    [
        [SQRT_3, SQRT_3, SQRT_3],
        [-SQRT_3, -SQRT_3, SQRT_3],
        [-SQRT_3, SQRT_3, -SQRT_3],
    ],
    [
        [SQRT_3, -SQRT_3, -SQRT_3],
        [-SQRT_3, SQRT_3, -SQRT_3],
        [-SQRT_3, -SQRT_3, SQRT_3],
    ],
    [
        [SQRT_3, SQRT_3, SQRT_3],
        [-SQRT_3, SQRT_3, -SQRT_3],
        [SQRT_3, -SQRT_3, -SQRT_3],
    ],
    [
        [SQRT_3, SQRT_3, SQRT_3],
        [SQRT_3, -SQRT_3, -SQRT_3],
        [-SQRT_3, -SQRT_3, SQRT_3],
    ],
];

/// `partial`: a point on the great-circle arc from `a` to `b`, the given
/// fraction of the way along it. Turning `a` about the axis perpendicular to
/// both, rather than interpolating and renormalising, keeps the points
/// evenly spaced.
fn partial(a: V3, b: V3, distance: f64) -> V3 {
    let axis = normalise(cross(a, b));
    let angle = dot(a, b).clamp(-1.0, 1.0).acos() * distance;
    let (s, c) = ((angle / 2.0).sin(), (angle / 2.0).cos());
    let (x, y, z, w) = (axis[0] * s, axis[1] * s, axis[2] * s, c);

    // The rotation matrix of that quaternion, and `a` through it.
    let t = [
        w * w + x * x - y * y - z * z,
        2.0 * x * y + 2.0 * w * z,
        2.0 * x * z - 2.0 * w * y,
        2.0 * x * y - 2.0 * w * z,
        w * w - x * x + y * y - z * z,
        2.0 * y * z + 2.0 * w * x,
        2.0 * x * z + 2.0 * w * y,
        2.0 * y * z - 2.0 * w * x,
        w * w - x * x - y * y + z * z,
    ];
    [
        a[0] * t[0] + a[1] * t[3] + a[2] * t[6],
        a[0] * t[1] + a[1] * t[4] + a[2] * t[7],
        a[0] * t[2] + a[1] * t[5] + a[2] * t[8],
    ]
}

/// One of the bumps that pushes the surface about. Everything about it is a
/// mass on a spring: `a` chases `c`, at velocity `v`, pulled by `m`.
#[derive(Clone, Copy, Default)]
struct Bump {
    cx: f64,
    cy: f64,
    cpower: f64,
    csize: f64,
    ax: f64,
    ay: f64,
    power: f64,
    size: f64,
    mx: f64,
    my: f64,
    mpower: f64,
    msize: f64,
    vx: f64,
    vy: f64,
    vpower: f64,
    vsize: f64,
    pos: V3,
}

#[derive(Clone, Copy, Default)]
struct Face {
    node1: usize,
    node2: usize,
    node3: usize,
    normal: V3,
}

/// Where the crossfade between two pictures has got to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrameState {
    Initialising,
    Holding,
    Loading,
    Transitioning,
}

struct MirrorBlob {
    trackball: Trackball,

    blob_center: V3,
    blob_anchor: V3,
    blob_velocity: V3,
    blob_force: V3,

    /// Where each vertex started, before the bumps moved it.
    initial: Vec<V3>,
    /// The normal each vertex has accumulated from the faces round it.
    node_normal: Vec<V3>,
    faces: Vec<Face>,

    dots: Vec<V3>,
    normals: Vec<V3>,
    colours: Vec<[f32; 4]>,
    tex_coords: Vec<[f32; 2]>,

    /// How hard a bump pushes at a given distance, and the same for a wall.
    bump_shape: Vec<f64>,
    wall_shape: Vec<f64>,
    bump_data: Vec<Bump>,

    current_texture: usize,
    textures: [Option<u32>; NUM_TEXTURES],
    state: FrameState,
    state_start_time: f64,

    blend: f32,
    do_walls: bool,
    do_texture: bool,
    do_paint_background: bool,
    do_colour: bool,
    offset_texture: bool,
    bumps: usize,
    fade_time: f64,
    hold_time: f64,
    zoom: f64,
    load_textures: bool,
}

impl MirrorBlob {
    /// `initialise_blob`: subdivide the tetrahedron into a sphere and work out
    /// which three vertices make up each face.
    ///
    /// The bounds of the loops shift about with the side because the vertices
    /// along a tetrahedron's edges belong to two faces and are only generated
    /// once, and the face indices then have to reach across into the earlier
    /// side's block to find them again. It is all upstream's, transcribed.
    fn initialise_blob(&mut self, resolution: usize) {
        let n = resolution;
        let num_nodes = 2 * n * n - 4 * n + 4;
        let num_faces = 4 * (n - 1) * (n - 1);

        self.initial = Vec::with_capacity(num_nodes);
        self.faces = vec![Face::default(); num_faces];

        let mut node = 0;
        let mut face = 0;
        let mut base2 = 0;
        for (side, corners) in TETRAHEDRON.iter().enumerate() {
            let base = node;
            if side == 2 {
                base2 = node;
            }

            let ulo = usize::from(side > 1);
            let uhi = n - usize::from(side > 0);
            for u in ulo..uhi {
                let node1 = partial(
                    normalise(corners[0]),
                    normalise(corners[1]),
                    u as f64 / (n - 1) as f64,
                );
                let node2 = partial(
                    normalise(corners[0]),
                    normalise(corners[2]),
                    u as f64 / (n - 1) as f64,
                );

                let vlo = usize::from(side > 1);
                let vhi = u.saturating_sub(usize::from(side > 2));
                for v in vlo..=vhi {
                    if vlo > vhi {
                        break;
                    }
                    let result = if u > 0 {
                        partial(node1, node2, v as f64 / u as f64)
                    } else {
                        node1
                    };
                    self.initial.push(normalise(result));
                    node += 1;
                }
            }

            // Which nodes make up each face.
            //
            // Upstream does this in signed ints, and the arithmetic relies on
            // it: the first assignment of a triangle can come out negative
            // and is then overwritten by one of the special cases below. So
            // the indices are worked out as `i64` here and only narrowed at
            // the end.
            let ne = n as i64;
            let base = base as i64;
            let base2i = base2 as i64;
            for u in 0..(ne - 1) {
                for v in 0..=u {
                    {
                        let (mut n1, mut n2, mut n3);
                        if side < 2 {
                            n1 = base + (u * (u + 1)) / 2 + v;
                            n2 = base + ((u + 1) * (u + 2)) / 2 + v + 1;
                            n3 = base + ((u + 1) * (u + 2)) / 2 + v;

                            if side == 1 && u == ne - 2 {
                                n3 = ((u + 1) * (u + 2)) / 2 + ne - v - 1;
                                n2 = ((u + 1) * (u + 2)) / 2 + ne - v - 2;
                            }
                        } else if side < 3 {
                            n1 = base + ((u - 1) * u) / 2 + v - 1;
                            n2 = base + (u * (u + 1)) / 2 + v;
                            n3 = base + (u * (u + 1)) / 2 + v - 1;

                            if u == ne - 2 {
                                let m = ne - v - 1;
                                n2 = (ne * (ne + 1)) / 2 + ((m - 1) * m) / 2;
                                n3 = (ne * (ne + 1)) / 2 + (m * (m + 1)) / 2;
                            }
                            if v == 0 {
                                n1 = ((u + 1) * (u + 2)) / 2 - 1;
                                n3 = ((u + 2) * (u + 3)) / 2 - 1;
                            }
                        } else {
                            n1 = base + ((u - 2) * (u - 1)) / 2 + v - 1;
                            n2 = base + ((u - 1) * u) / 2 + v;
                            n3 = base + ((u - 1) * u) / 2 + v - 1;

                            if v == 0 {
                                n1 = base2i + (u * (u + 1)) / 2 - 1;
                                n3 = base2i + ((u + 1) * (u + 2)) / 2 - 1;
                            }
                            if u == ne - 2 {
                                n3 = (ne * (ne + 1)) / 2 + ((v + 1) * (v + 2)) / 2 - 1;
                                n2 = (ne * (ne + 1)) / 2 + ((v + 2) * (v + 3)) / 2 - 1;
                            }
                            if v == u {
                                n1 = (u * (u + 1)) / 2;
                                n2 = ((u + 1) * (u + 2)) / 2;
                            }
                        }
                        let f = &mut self.faces[face];
                        f.node1 = n1.max(0) as usize;
                        f.node2 = n2.max(0) as usize;
                        f.node3 = n3.max(0) as usize;
                    }
                    face += 1;

                    if v < u {
                        let (mut n1, mut n2, mut n3);
                        if side < 2 {
                            n1 = base + (u * (u + 1)) / 2 + v;
                            n2 = base + (u * (u + 1)) / 2 + v + 1;
                            n3 = base + ((u + 1) * (u + 2)) / 2 + v + 1;

                            if side == 1 && u == ne - 2 {
                                n3 = ((u + 1) * (u + 2)) / 2 + ne - v - 2;
                            }
                        } else if side < 3 {
                            n1 = base + (u * (u - 1)) / 2 + v - 1;
                            n2 = base + (u * (u - 1)) / 2 + v;
                            n3 = base + (u * (u + 1)) / 2 + v;

                            if u == ne - 2 {
                                let m = ne - v - 1;
                                n3 = (ne * (ne + 1)) / 2 + (m * (m - 1)) / 2;
                            }
                            if v == 0 {
                                n1 = ((u + 1) * (u + 2)) / 2 - 1;
                            }
                        } else {
                            n1 = base + ((u - 2) * (u - 1)) / 2 + v - 1;
                            n2 = base + ((u - 2) * (u - 1)) / 2 + v;
                            n3 = base + ((u - 1) * u) / 2 + v;

                            if v == 0 {
                                n1 = base2i + (u * (u + 1)) / 2 - 1;
                            }
                            if u == ne - 2 {
                                n3 = (ne * (ne + 1)) / 2 + ((v + 2) * (v + 3)) / 2 - 1;
                            }
                            if v == u - 1 {
                                n2 = (u * (u + 1)) / 2;
                            }
                        }
                        let f = &mut self.faces[face];
                        f.node1 = n1.max(0) as usize;
                        f.node2 = n2.max(0) as usize;
                        f.node3 = n3.max(0) as usize;
                        face += 1;
                    }
                }
            }
        }

        let count = self.initial.len();
        self.node_normal = vec![[0.0; 3]; count];
        self.dots = vec![[0.0; 3]; count];
        self.normals = vec![[0.0; 3]; count];
        self.colours = vec![[1.0; 4]; count];
        self.tex_coords = vec![[0.0; 2]; count];

        // Where each bump starts, and what it is drifting towards.
        let b = self.bumps.max(1) as f64;
        self.bump_data = (0..self.bumps)
            .map(|_| {
                let mut d = Bump {
                    ax: 2.0 * (frand(1.0) - 0.5),
                    ay: 2.0 * (frand(1.0) - 0.5),
                    power: (5.0 / b.powf(0.75)) * (frand(1.0) - 0.5),
                    size: 0.1 + 0.5 * frand(1.0),
                    cx: 2.0 * (frand(1.0) - 0.5),
                    cy: 2.0 * (frand(1.0) - 0.5),
                    cpower: (5.0 / b.powf(0.75)) * (frand(1.0) - 0.5),
                    csize: 0.35,
                    mx: 0.003 * frand(1.0),
                    my: 0.003 * frand(1.0),
                    mpower: 0.003 * frand(1.0),
                    msize: 0.003 * frand(1.0),
                    ..Bump::default()
                };
                let pi = std::f64::consts::PI;
                d.pos = [
                    1.5 * (pi * d.ay).sin() * (pi * d.ax).cos(),
                    1.5 * (pi * d.ay).cos(),
                    1.5 * (pi * d.ay).sin() * (pi * d.ax).sin(),
                ];
                d
            })
            .collect();

        // How hard a bump pushes at a given squared distance.
        self.bump_shape = (0..BUMP_ARRAY_SIZE)
            .map(|i| {
                let xd = i as f64 / BUMP_ARRAY_SIZE as f64;
                0.1 / (48.0 * xd * xd + 0.1)
            })
            .collect();
        self.wall_shape = (0..BUMP_ARRAY_SIZE)
            .map(|i| {
                let xd = i as f64 / BUMP_ARRAY_SIZE as f64;
                0.4 / (40.0 * xd * xd * xd * xd + 0.1)
            })
            .collect();
    }

    /// `calc_blob`: move the bumps, push every vertex out by however much they
    /// come to at it, then work out the normals and where each vertex samples
    /// the picture.
    fn calc_blob(&mut self, limit: f64, fade: f64) {
        let pi = std::f64::consts::PI;
        for d in &mut self.bump_data {
            d.vx += d.mx * (d.cx - d.ax);
            d.vy += d.my * (d.cy - d.ay);
            d.vpower += d.mpower * (d.cpower - d.power);
            d.vsize += d.msize * (d.csize - d.size);

            d.ax += 0.1 * d.vx;
            d.ay += 0.1 * d.vy;
            d.power += 0.1 * d.vpower;
            d.size += 0.1 * d.vsize;

            d.pos = [
                (pi * d.ay).sin() * (pi * d.ax).cos(),
                (pi * d.ay).cos(),
                (pi * d.ay).sin() * (pi * d.ax).sin(),
            ];
        }

        self.blob_force = [0.0; 3];
        for index in 0..self.initial.len() {
            let node0 = self.initial[index];
            self.node_normal[index] = node0;

            let mut offset = [0.0; 3];
            for d in &self.bump_data {
                let bv = sub(d.pos, node0);
                let dist = (BUMP_ARRAY_SIZE as f64 * dot(bv, bv) * d.size) as usize;
                if dist < BUMP_ARRAY_SIZE {
                    let push = scale(node0, d.power * self.bump_shape[dist]);
                    add(&mut offset, push);
                    add(&mut self.blob_force, push);
                }
            }

            let mut node = node0;
            add(&mut node, offset);
            node = scale(node, self.zoom);
            add(&mut node, self.blob_center);

            if self.do_walls {
                self.squash_against_walls(&mut node, limit);
            }
            self.dots[index] = node;
        }

        // A vertex's normal is the sum of the normals of the faces round it.
        for f in &mut self.faces {
            f.normal = cross(
                sub(self.dots[f.node2], self.dots[f.node1]),
                sub(self.dots[f.node3], self.dots[f.node1]),
            );
            add(&mut self.node_normal[f.node1], f.normal);
            add(&mut self.node_normal[f.node2], f.normal);
            add(&mut self.node_normal[f.node3], f.normal);
        }

        if self.do_colour || self.do_texture {
            for index in 0..self.initial.len() {
                let n = normalise(self.node_normal[index]);
                self.normals[index] = n;

                if self.do_colour {
                    self.colours[index] = [
                        n[0].abs() as f32,
                        n[1].abs() as f32,
                        n[2].abs() as f32,
                        fade as f32,
                    ];
                }
                if self.do_texture {
                    self.tex_coords[index] = if self.offset_texture {
                        self.reflected_coord(index, n)
                    } else {
                        // A sphere map: which way the surface faces decides
                        // which part of the picture it shows. Upstream's is
                        // upside down and it puts a flip in the texture matrix
                        // to correct it; the textures here are top-down
                        // already, so the flip is written into the formula.
                        [
                            (0.5 * (1.0 + (n[0].asin() / (0.5 * pi)))) as f32,
                            (0.5 * (1.0 - (n[1].asin() / (0.5 * pi)))) as f32,
                        ]
                    };
                }
            }
        }

        // The blob as a whole drifts back towards its anchor, pushed about by
        // whatever the bumps did.
        let pull = scale(sub(self.blob_anchor, self.blob_center), 1.0 / 80.0);
        add(&mut self.blob_velocity, pull);
        let push = scale(self.blob_force, 0.01 / self.initial.len() as f64);
        add(&mut self.blob_velocity, push);
        let v = scale(self.blob_velocity, 0.5);
        add(&mut self.blob_center, v);
        self.blob_velocity = scale(self.blob_velocity, 0.999);
    }

    /// The `walls` option: the blob is in a box and spreads out where it
    /// touches a side.
    fn squash_against_walls(&mut self, node: &mut V3, limit: f64) {
        let bas = BUMP_ARRAY_SIZE as f64;
        // Each axis in turn, and only if the one before it was clear, which
        // is upstream's nesting.
        let mut done = false;
        for axis in [2usize, 1, 0] {
            if done {
                break;
            }
            let (a, b) = match axis {
                2 => (0, 1),
                1 => (0, 2),
                _ => (1, 2),
            };
            node[axis] = node[axis].clamp(-limit, limit);

            let near = (bas * (node[axis] + limit) * (node[axis] + limit) * 0.5) as usize;
            if near < BUMP_ARRAY_SIZE {
                let w = self.wall_shape[near];
                node[a] += (node[a] - self.blob_center[a]) * w;
                node[b] += (node[b] - self.blob_center[b]) * w;
                self.blob_force[axis] += node[axis] + limit;
                done = true;
                continue;
            }
            let far = (bas * (node[axis] - limit) * (node[axis] - limit) * 0.5) as usize;
            if far < BUMP_ARRAY_SIZE {
                let w = self.wall_shape[far];
                node[a] += (node[a] - self.blob_center[a]) * w;
                node[b] += (node[b] - self.blob_center[b]) * w;
                self.blob_force[axis] -= node[axis] - limit;
            }
        }
        node[1] = node[1].clamp(-limit, limit);
    }

    /// The `offsetTexture` option: follow the eye's reflection off the
    /// surface until it meets a notional cube a hundred units across, and
    /// sample the picture there.
    fn reflected_coord(&self, index: usize, n: V3) -> [f32; 2] {
        let cube = 100.0;
        let d = self.dots[index];
        let eye_r = normalise(sub(d, [0.0, 0.0, 50.0]));
        let r = sub(eye_r, scale(n, 2.0 * dot(eye_r, n)));

        let (mut x, mut y) = (0.0, 0.0);
        let mut n_min = 10000.0;

        if r[2].abs() > 1e-9 {
            let mut sign = 1.0;
            let mut t = (cube - d[2]) / r[2];
            if t < 0.0 {
                t = (-cube - d[2]) / r[2];
                sign = 3.0;
            }
            if t > 0.0 {
                x = sign * (d[0] + t * r[0]);
                y = sign * (d[1] + t * r[1]);
                n_min = t;
            }
        }
        if r[0].abs() > 1e-9 {
            let mut sign = 1.0;
            let mut t = (cube - d[0]) / r[0];
            if t < 0.0 {
                t = (-cube - d[0]) / r[0];
                sign = -1.0;
            }
            if t > 0.0 && t < n_min {
                x = sign * (2.0 * cube - (d[2] + t * r[2]));
                y = sign * x * (d[1] + t * r[1]) / cube;
                n_min = t;
            }
        }
        if r[1].abs() > 1e-9 {
            let mut sign = 1.0;
            let mut t = (cube - d[1]) / r[1];
            if t < 0.0 {
                t = (-cube - d[1]) / r[1];
                sign = -1.0;
            }
            if t > 0.0 && t < n_min {
                y = sign * (2.0 * cube - (d[2] + t * r[2]));
                x = sign * y * (d[0] + t * r[0]) / cube;
            }
        }

        [
            (0.5 + x / (cube * 6.0)) as f32,
            (0.5 - y / (cube * 6.0)) as f32,
        ]
    }

    /// `draw_blob`: every face, in one call.
    ///
    /// Upstream reads the trackball inside here, which applies its inertia
    /// once per call and so two or three times a frame while crossfading. The
    /// matrix is read once a frame instead and handed in.
    fn draw_blob(&self, g: &mut Gl, m: crate::runtime::gl::Mat4) {
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
        g.glx.translate(0.0, 0.0, -4.0);
        g.glx.mult_matrix(m);

        g.glx.begin(Shape::Triangles);
        for f in &self.faces {
            for i in [f.node1, f.node2, f.node3] {
                if self.do_colour {
                    let c = self.colours[i];
                    g.glx.color4f(c[0], c[1], c[2], c[3]);
                }
                if self.load_textures {
                    let t = self.tex_coords[i];
                    g.glx.tex_coord2f(t[0], t[1]);
                }
                let n = self.normals[i];
                g.glx.normal3f(n[0] as f32, n[1] as f32, n[2] as f32);
                let d = self.dots[i];
                g.glx.vertex3f(d[0] as f32, d[1] as f32, d[2] as f32);
            }
        }
        g.glx.end();
        g.glx.pop_matrix();
    }

    /// `draw_background`: the picture itself, over the whole frame, behind
    /// everything.
    fn draw_background(&self, g: &mut Gl, alpha: f32) {
        g.glx.texturing(true);
        g.glx.lighting(false);
        g.glx.color_material(true);
        g.glx.color4f(1.0, 1.0, 1.0, alpha);

        g.glx.matrix_mode_projection();
        g.glx.push_matrix();
        g.glx.load_identity();
        let (w, h) = (g.width() as f32, g.height() as f32);
        g.glx.ortho(0.0, w, h, 0.0, -1000.0, 1000.0);
        g.glx.matrix_mode_modelview();
        g.glx.push_matrix();
        g.glx.load_identity();

        g.glx.begin(Shape::Quads);
        for (x, y, u, v) in [
            (0.0, 0.0, 0.0, 0.0),
            (0.0, h, 0.0, 1.0),
            (w, h, 1.0, 1.0),
            (w, 0.0, 1.0, 0.0),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, 0.0);
        }
        g.glx.end();

        g.glx.pop_matrix();
        g.glx.matrix_mode_projection();
        g.glx.pop_matrix();
        g.glx.matrix_mode_modelview();
    }

    /// Ask for a picture into the given slot. True once it is there.
    fn grab_texture(&mut self, g: &mut Gl, which: usize) -> bool {
        let w = (g.width() / 2 - 1).max(10);
        let h = (g.height() / 2 - 1).max(10);
        let Some(img) = g.load_image(w, h) else {
            return false;
        };
        let id = self.textures[which].unwrap_or_else(|| g.glx.gen_texture());
        g.glx.bind_texture(id);
        g.glx.tex_image_2d(img.width, img.height, img.pixels);
        g.glx.tex_clamp(true);
        self.textures[which] = Some(id);
        true
    }
}

impl Hack3d for MirrorBlob {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let now = g.time;

        if self.load_textures && self.textures[self.current_texture].is_none() {
            let c = self.current_texture;
            self.grab_texture(g, c);
        }

        let fade = match self.state {
            FrameState::Initialising => 1.0,
            FrameState::Transitioning => {
                1.0 - (now - self.state_start_time) / self.fade_time.max(1e-6)
            }
            FrameState::Loading | FrameState::Holding => 1.0,
        }
        .clamp(0.0, 1.0);

        g.glx.clear_color(0.0, 0.0, 0.0, 1.0);
        g.glx.depth_test(false);
        g.glx.clear();

        if self.do_paint_background
            && let Some(id) = self.textures[self.current_texture]
        {
            g.glx.bind_texture(id);
            g.glx.blend(Blend::Off);
            self.draw_background(g, 1.0);

            if self.state == FrameState::Transitioning
                && let Some(next) = self.textures[1 - self.current_texture]
            {
                g.glx.bind_texture(next);
                g.glx.blend(Blend::Alpha);
                self.draw_background(g, 1.0 - fade as f32);
            }
            // The background is behind everything, so let the blob win.
            g.glx.clear_depth();
        }

        self.calc_blob(2.5, fade * f64::from(self.blend));
        let m = self.trackball.matrix();

        // The blob's own state: culled, lit, and its colour coming from the
        // vertex rather than the material.
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.front_face_cw(false);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_enable(1, true);
        g.glx.color_material(true);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(32.0);

        let alpha = fade as f32 * self.blend;
        if self.blend < 1.0 {
            g.glx.blend(Blend::Alpha);
            g.glx.color4f(0.9, 0.9, 1.0, alpha);
        } else {
            g.glx.blend(Blend::Off);
            g.glx.color4f(0.9, 0.9, 1.0, 1.0);
        }

        if self.do_texture
            && let Some(id) = self.textures[self.current_texture]
        {
            g.glx.texturing(true);
            g.glx.bind_texture(id);
        } else {
            g.glx.texturing(false);
        }

        // A translucent blob is drawn twice: once into the depth buffer only,
        // so that its own far side cannot show through its near side, and
        // then again for real.
        if self.blend < 1.0 {
            g.glx.color_mask(false);
            self.draw_blob(g, m);
            g.glx.color_mask(true);
        }
        g.glx.depth_func(crate::runtime::gl::DepthFunc::LessEqual);
        self.draw_blob(g, m);

        // And a third time while crossfading, with the next picture on it.
        if self.load_textures && self.hold_time > 0.0 {
            match self.state {
                FrameState::Initialising => {
                    if self.textures[self.current_texture].is_some() {
                        self.state = FrameState::Holding;
                        self.state_start_time = now;
                    }
                }
                FrameState::Holding => {
                    if now - self.state_start_time > self.hold_time {
                        let other = 1 - self.current_texture;
                        self.grab_texture(g, other);
                        self.state = FrameState::Loading;
                    }
                }
                FrameState::Loading => {
                    if self.textures[1 - self.current_texture].is_some() {
                        self.state = FrameState::Transitioning;
                        self.state_start_time = now;
                    }
                }
                FrameState::Transitioning => {
                    if self.do_texture
                        && let Some(next) = self.textures[1 - self.current_texture]
                    {
                        g.glx.bind_texture(next);
                        g.glx.blend(Blend::Alpha);
                        g.glx
                            .color4f(0.9, 0.9, 1.0, (1.0 - fade) as f32 * self.blend);
                        self.draw_blob(g, m);
                    }
                    if now - self.state_start_time > self.fade_time {
                        self.state = FrameState::Holding;
                        self.state_start_time = now;
                        self.current_texture = 1 - self.current_texture;
                    }
                }
            }
        }

        g.glx.depth_func(crate::runtime::gl::DepthFunc::Less);
        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(60.0, 1.0, 1.0, 1024.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event {
            match key {
                '+' | '=' => {
                    self.zoom *= 1.1;
                    return true;
                }
                '-' | '_' => {
                    self.zoom *= 0.9;
                    return true;
                }
                _ => {}
            }
        }
        if screenhack_event_helper(event) {
            self.state_start_time = 0.0;
            self.state = FrameState::Holding;
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    // Upstream's wireframe uses `glPolygonMode`, which its own OpenGL ES build
    // does not have either: `set_parameters` turns wireframe off outright
    // under `HAVE_JWZGLES`. So does this.
    let do_texture = g.res.bool("texture");
    let do_paint_background = g.res.bool("paintBackground");

    let mut st = MirrorBlob {
        trackball: Trackball::new(),
        blob_center: [0.0; 3],
        blob_anchor: [0.0; 3],
        blob_velocity: [0.0; 3],
        blob_force: [0.0; 3],
        initial: Vec::new(),
        node_normal: Vec::new(),
        faces: Vec::new(),
        dots: Vec::new(),
        normals: Vec::new(),
        colours: Vec::new(),
        tex_coords: Vec::new(),
        bump_shape: Vec::new(),
        wall_shape: Vec::new(),
        bump_data: Vec::new(),
        current_texture: 0,
        textures: [None; NUM_TEXTURES],
        state: FrameState::Initialising,
        state_start_time: 0.0,
        blend: g.res.float("blend").clamp(0.1, 1.0) as f32,
        do_walls: g.res.bool("walls"),
        do_texture,
        do_paint_background,
        do_colour: g.res.bool("colour"),
        // Nothing to offset if there is no texture.
        offset_texture: do_texture && g.res.bool("offsetTexture"),
        bumps: g.res.int("bumps").clamp(0, 50) as usize,
        fade_time: g.res.float("fadeTime").clamp(0.0, 30.0),
        hold_time: g.res.float("holdTime").clamp(5.0, 300.0),
        zoom: g.res.float("zoom").clamp(0.1, 3.0),
        load_textures: do_texture || do_paint_background,
    };

    let resolution = g.res.int("resolution").clamp(4, MAX_RESOLUTION) as usize;
    st.initialise_blob(resolution);

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    // Two lamps, one from in front and to the right and one from behind the
    // viewer, with no scene ambient to speak of.
    g.glx.light_model_ambient([0.2, 0.2, 0.2, 1.0]);
    g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
    g.glx.light_diffuse(0, [0.6, 0.6, 0.6, 1.0]);
    g.glx.light_specular(0, [0.8, 0.8, 0.9, 1.0]);
    g.glx.light_position(0, 500.0, 100.0, 200.0, 1.0);
    g.glx.light_ambient(1, [0.0, 0.0, 0.0, 1.0]);
    g.glx.light_diffuse(1, [0.6, 0.6, 0.6, 1.0]);
    g.glx.light_specular(1, [0.7, 0.7, 0.7, 1.0]);
    g.glx.light_position(1, -50.0, -100.0, 2500.0, 1.0);

    Box::new(st)
}

/// Upstream's own ceiling, kept: see the note on the `resolution` option.
const MAX_RESOLUTION: i32 = 150;

const DEFAULTS: &[&str] = &[
    "*delay:           10000",
    "*showFPS:         False",
    "*wireframe:       False",
    "*blend:           1.0",
    "*walls:           False",
    "*colour:          False",
    "*texture:         True",
    "*offsetTexture:   False",
    "*paintBackground: True",
    "*resolution:      60",
    "*bumps:           10",
    "*holdTime:        30.0",
    "*fadeTime:        5.0",
    "*zoom:            1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("zoom", "Zoom", 0.1, 3.0, 0.1, 1, "1.0"),
    Opt::slider(
        "holdTime",
        "Time until loading a new image",
        5.0,
        300.0,
        5.0,
        0,
        "30",
    ),
    Opt::slider("fadeTime", "Transition duration", 0.0, 30.0, 1.0, 0, "5"),
    // Upstream's full range. Every vertex is pushed about by every bump and
    // every face contributes to three vertices' normals, all on the CPU and
    // all every frame, so this was measured before being left alone: at
    // 1280x720 the default 60 is 42k vertices and 0.8ms a frame, and the top
    // of the slider is 266k and 5.8ms. Two draw calls either way.
    Opt::slider("resolution", "Resolution", 4.0, 150.0, 1.0, 0, "60"),
    Opt::slider("bumps", "Bumps", 0.0, 50.0, 1.0, 0, "10"),
    Opt::slider("blend", "Transparency", 0.1, 1.0, 0.05, 2, "1.0"),
    Opt::boolean("walls", "Enable walls", "false"),
    Opt::boolean("colour", "Enable colouring", "false"),
    Opt::boolean("texture", "Enable reflected image", "true"),
    Opt::boolean("paintBackground", "Show image on background", "true"),
    Opt::boolean("offsetTexture", "Offset texture coordinates", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "mirrorblob",
    label: "Mirror Blob",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jon Dowdall",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=o4GTO18KHe8"),
        blurb: "A wobbly blob distorts images behind it.",
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

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// A blob with no GL behind it.
    fn a_blob(resolution: usize, bumps: usize) -> MirrorBlob {
        let mut st = MirrorBlob {
            trackball: Trackball::new(),
            blob_center: [0.0; 3],
            blob_anchor: [0.0; 3],
            blob_velocity: [0.0; 3],
            blob_force: [0.0; 3],
            initial: Vec::new(),
            node_normal: Vec::new(),
            faces: Vec::new(),
            dots: Vec::new(),
            normals: Vec::new(),
            colours: Vec::new(),
            tex_coords: Vec::new(),
            bump_shape: Vec::new(),
            wall_shape: Vec::new(),
            bump_data: Vec::new(),
            current_texture: 0,
            textures: [None; NUM_TEXTURES],
            state: FrameState::Initialising,
            state_start_time: 0.0,
            blend: 1.0,
            do_walls: false,
            do_texture: true,
            do_paint_background: true,
            do_colour: false,
            offset_texture: false,
            bumps,
            fade_time: 5.0,
            hold_time: 30.0,
            zoom: 1.0,
            load_textures: true,
        };
        st.initialise_blob(resolution);
        st
    }

    /// Subdividing the tetrahedron gives the counts upstream's arithmetic
    /// says it should, and every vertex lands on the unit sphere: no poles,
    /// no seam, nothing bunched up.
    #[test]
    fn the_blob_starts_as_a_sphere() {
        for n in [4usize, 10, 30, 60] {
            let st = a_blob(n, 0);
            assert_eq!(
                st.initial.len(),
                2 * n * n - 4 * n + 4,
                "resolution {n} made the wrong number of vertices"
            );
            assert_eq!(st.faces.len(), 4 * (n - 1) * (n - 1), "resolution {n}");

            for (i, p) in st.initial.iter().enumerate() {
                let r = dot(*p, *p).sqrt();
                assert!(
                    (r - 1.0).abs() < 1e-9,
                    "vertex {i} of resolution {n} is {r} from the middle"
                );
            }
        }
    }

    /// The vertices are spread out rather than bunched: the nearest neighbour
    /// of any vertex is roughly as far away as the nearest neighbour of any
    /// other, which is what subdividing along arcs buys over subdividing a
    /// flat triangle and normalising.
    #[test]
    fn the_vertices_are_evenly_spread() {
        let st = a_blob(10, 0);
        let mut nearest: Vec<f64> = Vec::new();
        for (i, a) in st.initial.iter().enumerate() {
            let d = st
                .initial
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, b)| dot(sub(*a, *b), sub(*a, *b)).sqrt())
                .fold(f64::MAX, f64::min);
            nearest.push(d);
        }
        let lo = nearest.iter().copied().fold(f64::MAX, f64::min);
        let hi = nearest.iter().copied().fold(0.0, f64::max);
        assert!(lo > 0.0, "two vertices are in the same place");
        assert!(
            hi / lo < 2.0,
            "spacing runs from {lo} to {hi}, which is bunched"
        );
    }

    /// Every face names three distinct vertices that exist. A face that
    /// repeated a vertex would be a degenerate triangle with no normal.
    #[test]
    fn every_face_is_a_real_triangle() {
        // Every resolution the slider offers, since the index arithmetic
        // has special cases that only fire at particular sizes.
        for n in 4..=MAX_RESOLUTION as usize {
            let st = a_blob(n, 0);
            let count = st.initial.len();
            for (i, f) in st.faces.iter().enumerate() {
                assert!(
                    f.node1 < count && f.node2 < count && f.node3 < count,
                    "face {i} of resolution {n} names a vertex that is not there"
                );
                assert!(
                    f.node1 != f.node2 && f.node2 != f.node3 && f.node1 != f.node3,
                    "face {i} of resolution {n} repeats a vertex"
                );
            }
        }
    }

    /// A bump pushes hardest where it is and falls away with distance, so a
    /// blob with bumps is no longer a sphere but is still all in one piece.
    #[test]
    fn the_bumps_push_the_sphere_out_of_shape() {
        let mut st = a_blob(20, 10);
        // With no bumps at all nothing moves.
        let mut flat = a_blob(20, 0);
        flat.calc_blob(2.5, 1.0);
        for (i, d) in flat.dots.iter().enumerate() {
            let r = dot(*d, *d).sqrt();
            assert!((r - 1.0).abs() < 1e-6, "vertex {i} moved without a bump");
        }

        for _ in 0..30 {
            st.calc_blob(2.5, 1.0);
        }
        let radii: Vec<f64> = st
            .dots
            .iter()
            .map(|d| dot(sub(*d, st.blob_center), sub(*d, st.blob_center)).sqrt())
            .collect();
        let lo = radii.iter().copied().fold(f64::MAX, f64::min);
        let hi = radii.iter().copied().fold(0.0, f64::max);
        assert!(hi - lo > 0.01, "the bumps did nothing: {lo} to {hi}");
        assert!(lo > 0.1, "the blob turned inside out at {lo}");
        assert!(hi < 5.0, "the blob blew up to {hi}");
    }

    /// The sphere map runs over the whole picture and no further: a normal
    /// pointing up samples the top of it and one pointing down the bottom.
    #[test]
    fn the_reflection_covers_the_picture_the_right_way_up() {
        let mut st = a_blob(20, 10);
        st.calc_blob(2.5, 1.0);
        for (i, t) in st.tex_coords.iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(&t[0]) && (0.0..=1.0).contains(&t[1]),
                "vertex {i} samples {t:?}, off the picture"
            );
        }

        // The topmost vertex takes the top of the picture, which is v near
        // zero here because these textures are top-down.
        let top = (0..st.normals.len())
            .max_by(|a, b| st.normals[*a][1].total_cmp(&st.normals[*b][1]))
            .expect("a blob has vertices");
        let bottom = (0..st.normals.len())
            .min_by(|a, b| st.normals[*a][1].total_cmp(&st.normals[*b][1]))
            .expect("a blob has vertices");
        assert!(
            st.tex_coords[top][1] < st.tex_coords[bottom][1],
            "the reflection is upside down: {} against {}",
            st.tex_coords[top][1],
            st.tex_coords[bottom][1]
        );
    }

    /// It draws: the picture behind, and the blob with the picture on it.
    #[test]
    fn the_blob_reflects_the_picture() {
        let r = run("resolution=20", 4);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        assert!(
            f.batches.iter().filter(|b| b.texture.is_some()).count() >= 2,
            "the background and the blob are not both textured"
        );
    }

    /// The walls option keeps the blob inside a box.
    #[test]
    fn walls_keep_the_blob_in_its_box() {
        let mut st = a_blob(20, 20);
        st.do_walls = true;
        for _ in 0..60 {
            st.calc_blob(2.5, 1.0);
        }
        for (i, d) in st.dots.iter().enumerate() {
            assert!(
                d[1].abs() <= 2.5 + 1e-6,
                "vertex {i} left the box at y = {}",
                d[1]
            );
        }
    }
}
