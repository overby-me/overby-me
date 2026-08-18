//! Port of `hacks/glx/hilbert.c`.
//!
//! ```text
//! hilbert, Copyright (c) 2011-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! 2D and 3D Hilbert space-filling curves.
//!
//! Inspired by "Visualizing Hilbert Curves" by Nelson Max, 1998:
//! https://e-reports-ext.llnl.gov/pdf/234149.pdf
//! ```
//!
//! The recursive Hilbert space-filling curve, in two dimensions and in three.
//!
//! A point on the curve is found from its index along it rather than by
//! recursing: read the index two bits at a time (three in 3D), and each group
//! picks one of the four sub-squares (eight sub-cubes) and a reflection to
//! apply to everything finer than it. Multiplying those reflections together
//! from the coarsest group down gives the point. The colour runs along the
//! curve, so two points that are near each other in space are usually near each
//! other in colour, which is the property the curve is famous for.
//!
//! It draws one depth growing from nothing, then hands over to the next depth
//! by drawing the two at once, the coarse one retreating from the front while
//! the fine one advances into it. The two are not actually joined where they
//! meet and sometimes overlap, which upstream notes and says goes by too fast
//! to see.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap, make_uniform_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::opts::SelectItem;
use crate::runtime::shapes::unit_sphere;
use crate::runtime::tube::TubeMesh;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

/// How long to hold still between one depth and the next.
const PAUSE_TICKS: i32 = 180;

/// The tube and sphere resolutions, coarsest last. Which one is used follows
/// from how wide the tube comes out on screen.
const DLIST_FACES: [i32; 5] = [20, 10, 8, 4, 3];

/// Which bits of the index contribute to each coordinate. Everything finer is
/// defined by recursion from there.
const XBIT2D: [i32; 4] = [0, 0, 1, 1];
const YBIT2D: [i32; 4] = [0, 1, 1, 0];

const XBIT3D: [i32; 8] = [0, 0, 0, 0, 1, 1, 1, 1];
const YBIT3D: [i32; 8] = [0, 1, 1, 0, 0, 1, 1, 0];
const ZBIT3D: [i32; 8] = [0, 0, 1, 1, 1, 1, 0, 0];

/// The reflection each of the four sub-squares applies to everything below it.
/// This is the ordinary Hilbert descent.
///
/// ```text
///        _    _
///       | |..| |
///       :_    _:
///        _|  |_
/// ```
const R2D: [[[i32; 2]; 2]; 4] = [
    [[0, 1], [1, 0]],
    [[1, 0], [0, 1]],
    [[1, 0], [0, 1]],
    [[0, -1], [-1, 0]],
];

/// The same, for the outermost level only when the path is a closed loop.
///
/// ```text
///        __    __
///       |  |..|  |
///       :   ..   :
///       |__|  |__|
/// ```
const S2D: [[[i32; 2]; 2]; 4] = [
    [[-1, 0], [0, -1]],
    [[1, 0], [0, 1]],
    [[1, 0], [0, 1]],
    [[-1, 0], [0, -1]],
];

/// The eight sub-cubes of the ordinary 3D descent.
const R3D: [[[i32; 3]; 3]; 8] = [
    [[0, 1, 0], [1, 0, 0], [0, 0, 1]],
    [[0, 0, 1], [0, 1, 0], [1, 0, 0]],
    [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
    [[0, 0, -1], [-1, 0, 0], [0, 1, 0]],
    [[0, 0, 1], [1, 0, 0], [0, 1, 0]],
    [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
    [[0, 0, -1], [0, 1, 0], [-1, 0, 0]],
    [[0, -1, 0], [-1, 0, 0], [0, 0, 1]],
];

/// The same, for the outermost level only when the path is a closed loop. Only
/// the first and last differ from [`R3D`], which is what joins the two ends.
const S3D: [[[i32; 3]; 3]; 8] = [
    [[-1, 0, 0], [0, 0, -1], [0, 1, 0]],
    [[0, 0, 1], [0, 1, 0], [1, 0, 0]],
    [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
    [[0, 0, -1], [-1, 0, 0], [0, 1, 0]],
    [[0, 0, 1], [1, 0, 0], [0, 1, 0]],
    [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
    [[0, 0, -1], [0, 1, 0], [-1, 0, 0]],
    [[-1, 0, 0], [0, 0, -1], [0, 1, 0]],
];

fn mul2d(a: [[i32; 2]; 2], b: [[i32; 2]; 2]) -> [[i32; 2]; 2] {
    let mut d = [[0; 2]; 2];
    for (i, row) in d.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j];
        }
    }
    d
}

fn mul3d(a: [[i32; 3]; 3], b: [[i32; 3]; 3]) -> [[i32; 3]; 3] {
    let mut d = [[0; 3]; 3];
    for (i, row) in d.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    d
}

/// `t_to_xy`: where the point at index `t` sits on a 2D curve of depth `n`.
fn t_to_xy(n: i32, t: i64, closed: bool) -> (i32, i32) {
    let mut rt = [[1, 0], [0, 1]];
    let (mut x, mut y) = (0, 0);

    for k in (0..n).rev() {
        let j = (3 & (t >> (2 * k))) as usize;
        let va = [2 * XBIT2D[j] - 1, 2 * YBIT2D[j] - 1];
        let vb = [
            rt[0][0] * va[0] + rt[0][1] * va[1],
            rt[1][0] * va[0] + rt[1][1] * va[1],
        ];
        x += ((vb[0] + 1) / 2) << k;
        y += ((vb[1] + 1) / 2) << k;
        if k > 0 {
            let rq = rt;
            rt = mul2d(rq, if k == n - 1 && closed { S2D[j] } else { R2D[j] });
        }
    }
    (x, y)
}

/// `t_to_xyz`: the same in three dimensions.
fn t_to_xyz(n: i32, t: i64, closed: bool) -> (i32, i32, i32) {
    let mut rt = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
    let (mut x, mut y, mut z) = (0, 0, 0);

    for k in (0..n).rev() {
        let j = (7 & (t >> (3 * k))) as usize;
        let va = [2 * XBIT3D[j] - 1, 2 * YBIT3D[j] - 1, 2 * ZBIT3D[j] - 1];
        let vb = [
            rt[0][0] * va[0] + rt[0][1] * va[1] + rt[0][2] * va[2],
            rt[1][0] * va[0] + rt[1][1] * va[1] + rt[1][2] * va[2],
            rt[2][0] * va[0] + rt[2][1] * va[1] + rt[2][2] * va[2],
        ];
        x += ((vb[0] + 1) / 2) << k;
        y += ((vb[1] + 1) / 2) << k;
        z += ((vb[2] + 1) / 2) << k;
        if k > 0 {
            let rq = rt;
            rt = mul3d(rq, if k == n - 1 && closed { S3D[j] } else { R3D[j] });
        }
    }
    (x, y, z)
}

/// The most points one curve may be drawn from.
///
/// Upstream's own limit is a depth of twenty, which in three dimensions is
/// 8^20 points and would want six gigabytes of cache before it drew anything.
/// The real limit here is what a frame can carry: a curve is one tube and one
/// sphere per point, and two depths are on screen at once while one hands over
/// to the next.
///
/// Measured at 1280x720 with the default thickness, over a full cycle up to
/// the cap: 3D comes to 1025 batches and 368k vertices at depth 3, 6046 and
/// 847k at depth 4, and 4.6M vertices at depth 5. So this is set at 4096
/// points, which caps 3D at depth 4 and 2D at depth 6 (1821 batches, 294k
/// vertices). Upstream's default `maxDepth` of 5 is therefore reached in two
/// dimensions and not in three.
const MAX_POINTS: i64 = 1 << 12;

/// The deepest curve of this kind that stays inside [`MAX_POINTS`].
fn depth_limit(twodee: bool) -> i32 {
    let per = if twodee { 2 } else { 3 };
    let mut d = 2;
    while (1i64 << (per * (d + 1))) <= MAX_POINTS {
        d += 1;
    }
    d
}

struct Hilbert {
    rot: Rotator,
    rot2: Rotator,
    trackball: Trackball,
    twodee: bool,
    closed: bool,
    ncolors: usize,
    colors: Vec<XColor>,

    depth: i32,
    depth_tick: i32,

    path_start: f32,
    path_end: f32,
    path_tick: i32,
    pause: i32,
    diam: f32,

    /// The points of each depth, worked out once and kept. Indexed by depth.
    caches: Vec<Option<Vec<[u16; 3]>>>,

    /// One tube mesh per entry of [`DLIST_FACES`], and the sphere list that
    /// goes with it.
    tubes: Vec<(i32, TubeMesh)>,
    spheres: Vec<(i32, u32)>,

    speed: f32,
    max_depth: i32,
    thickness: f32,
    do_spin: bool,
    wireframe: bool,
    /// Set while a segment has dropped to wireframe because it is too thin to
    /// be worth drawing as a tube, so the next one knows to put the state back.
    dropped_to_wire: bool,
}

impl Hilbert {
    fn mesh(&self, faces: i32) -> &TubeMesh {
        &self
            .tubes
            .iter()
            .find(|(f, _)| *f == faces)
            .expect("every face count in DLIST_FACES has a mesh")
            .1
    }

    fn sphere(&self, faces: i32) -> u32 {
        self.spheres
            .iter()
            .find(|(f, _)| *f == faces)
            .expect("every face count in DLIST_FACES has a sphere")
            .1
    }

    /// `draw_joint`: a sphere where two tubes meet, to round the corner off.
    fn draw_joint(&self, g: &mut Gl, p: [f32; 3], diam: f32, faces: i32) {
        // Too small to see. Upstream's cutoff is four faces; this one is
        // eight, which is the measurement below rather than a guess: a joint
        // at eight faces is about six pixels across at 1280x720, and there is
        // one per point of the curve.
        if faces <= 8 {
            return;
        }
        let diam = diam * 0.99; /* try to clean up the edges a bit */
        // A prefabricated unit sphere put in place with the matrix stack, as
        // upstream does. Unlike the tubes these are *not* transformed inline:
        // a sphere is a triangle strip, and flattening it into loose triangles
        // to make it merge would treble its vertex count, which in three
        // dimensions costs more than the draw call it saves.
        g.glx.push_matrix();
        g.glx.translate(p[0], p[1], p[2]);
        g.glx.scale(diam, diam, diam);
        g.glx.call_list(self.sphere(faces));
        g.glx.pop_matrix();
    }

    /// `draw_segment`: one step of the curve, clipped to the part of the path
    /// that is showing, and cut into as many pieces as the colour ramp needs.
    ///
    /// Returns whether anything was drawn.
    #[allow(clippy::too_many_arguments)]
    fn draw_segment(
        &mut self,
        g: &mut Gl,
        mut p0: [f32; 3],
        mut p1: [f32; 3],
        t: i64,
        end_t: i64,
        path_start: f32,
        path_end: f32,
        mut head_cap: bool,
        last_color: &mut i32,
    ) -> bool {
        let t0 = (t - 1) as f32 / end_t as f32;
        let t1 = t as f32 / end_t as f32;

        // Wholly before or wholly after the part that is showing.
        if path_start >= t1 || path_end < t0 {
            return false;
        }

        let owire = self.wireframe;
        let mut wire = owire;
        let mut dd = self.diam;
        // More polys in 2D mode.
        if self.twodee {
            dd *= 2.0;
        }

        let mut faces = if dd > 0.040 {
            DLIST_FACES[0]
        } else if dd > 0.020 {
            DLIST_FACES[1]
        } else if dd > 0.010 {
            DLIST_FACES[2]
        } else if dd > 0.005 {
            DLIST_FACES[3]
        } else if dd > 0.002 {
            DLIST_FACES[4]
        } else {
            1
        };

        // In 2D we can drop to wireframe at this size and it still looks all
        // right; in 3D it would not, so take the coarsest tube instead.
        if faces == 1 {
            if self.twodee {
                wire = true;
            } else {
                faces = 3;
            }
        }

        if wire && !owire {
            g.glx.depth_test(false);
            g.glx.cull_face(false);
            g.glx.lighting(false);
            self.dropped_to_wire = true;
        }

        let lerp = |a: [f32; 3], b: [f32; 3], r: f32| {
            [
                a[0] + (b[0] - a[0]) * r,
                a[1] + (b[1] - a[1]) * r,
                a[2] + (b[2] - a[2]) * r,
            ]
        };

        let seg_range = t1 - t0;
        if t0 < path_start {
            p0 = lerp(p0, p1, (path_start - t0) / seg_range);
        }
        if t1 > path_end {
            p1 = lerp(p0, p1, (path_end - t0) / seg_range);
        }
        if p0 == p1 {
            return false;
        }

        let segs = ((self.ncolors as f32 * (t1 - t0)) as i32).max(1);
        let mut p1b = p1;
        for i in 0..segs {
            let fi = i as f32 / segs as f32;
            let fj = (i + 1) as f32 / segs as f32;
            let p0b = lerp(p0, p1, fi);
            p1b = lerp(p0, p1, fj);

            // Upstream marks this one as not quite right.
            let t0b = t0 + i as f32 * (t1 - t0) / segs as f32;
            let c = ((self.ncolors as f32 * t0b) as i32).min(self.ncolors as i32 - 1);

            // Above depth six this was five per cent of the time, so only set
            // the colour when it changes.
            if c != *last_color {
                let xc = self.colors[c.max(0) as usize];
                let color = [
                    f32::from(xc.red) / 65536.0,
                    f32::from(xc.green) / 65536.0,
                    f32::from(xc.blue) / 65536.0,
                    1.0,
                ];
                if wire {
                    g.glx.color4f(color[0], color[1], color[2], 1.0);
                } else {
                    g.glx.material_ambient_diffuse(color);
                }
                *last_color = c;
            }

            if wire {
                g.glx.begin(Shape::Lines);
                g.glx.vertex3f(p0b[0], p0b[1], p0b[2]);
                g.glx.vertex3f(p1b[0], p1b[1], p1b[2]);
                g.glx.end();
            } else {
                // Upstream puts a prefabricated unit tube in place with the
                // matrix stack. Here the mesh is transformed on the way out
                // instead, so that a run of tubes of one colour lands in one
                // draw call rather than one apiece: a 3D curve is one tube per
                // point and there are tens of thousands of them.
                self.mesh(faces).draw(&mut g.glx, p0b, p1b, self.diam, 0.0);

                // If this is the bleeding edge, cap it too.
                if head_cap {
                    self.draw_joint(g, p0b, self.diam, faces);
                    head_cap = false;
                }
            }
        }

        if !wire {
            self.draw_joint(g, p1b, self.diam, faces);
        }
        true
    }

    /// `hilbert`: the whole curve at one depth, less whatever falls outside
    /// the part of the path that is showing.
    fn draw_curve(&mut self, g: &mut Gl, size: i32, path_start: f32, path_end: f32) {
        let wire = self.wireframe;
        let w = (1i64 << size) as f32;
        let end_t = if self.twodee {
            1i64 << (2 * size)
        } else {
            1i64 << (3 * size)
        };

        if !wire {
            g.glx.depth_test(true);
            g.glx.cull_face(true);
            g.glx.lighting(true);
        }
        self.dropped_to_wire = false;

        let size_u = size as usize;
        if self.caches.len() <= size_u {
            self.caches.resize(size_u + 1, None);
        }
        if self.caches[size_u].is_none() {
            let mut v = Vec::with_capacity(end_t as usize);
            for t in 0..end_t {
                let (x, y, z) = if self.twodee {
                    let (x, y) = t_to_xy(size, t, self.closed);
                    (x, y, (w / 2.0) as i32)
                } else {
                    t_to_xyz(size, t, self.closed)
                };
                v.push([x as u16, y as u16, z as u16]);
            }
            self.caches[size_u] = Some(v);
        }

        let mut prev = [0.0f32; 3];
        let mut first = [0.0f32; 3];
        let mut first_p = false;
        let mut any = false;
        let mut last_color = -1;

        for t in 0..end_t {
            let cb = self.caches[size_u].as_ref().expect("just filled")[t as usize];
            let c = [
                f32::from(cb[0]) / w - 0.5,
                f32::from(cb[1]) / w - 0.5,
                f32::from(cb[2]) / w - 0.5,
            ];

            if t > 0
                && self.draw_segment(
                    g,
                    prev,
                    c,
                    t,
                    end_t,
                    path_start,
                    path_end,
                    !any,
                    &mut last_color,
                )
            {
                any = true;
            }
            prev = c;
            if !first_p {
                first = c;
                first_p = true;
            }
        }

        if self.closed && path_end >= 1.0 {
            self.draw_segment(
                g,
                prev,
                first,
                end_t,
                end_t,
                path_start,
                path_end,
                !any,
                &mut last_color,
            );
        }

        // A segment that dropped to wireframe turned these off for itself.
        if self.dropped_to_wire && !wire {
            g.glx.depth_test(true);
            g.glx.cull_face(true);
            g.glx.lighting(true);
        }
    }
}

impl Hack3d for Hilbert {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.wireframe;
        let down = self.trackball.button_down();

        if !wire {
            g.glx.depth_test(true);
            g.glx.cull_face(true);
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
        }
        g.glx.clear();

        g.glx.push_matrix();
        g.glx.scale(1.1, 1.1, 1.1);

        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 8.0,
            (z as f32 - 0.5) * 15.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        if self.twodee && self.do_spin {
            // Face front, but upside down is all right.
            let max = 70.0;
            let (x2, y2, _) = self.rot2.position(!down);
            g.glx.rotate(max / 2.0 - x2 as f32 * max, 1.0, 0.0, 0.0);
            g.glx.rotate(max / 2.0 - y2 as f32 * max, 0.0, 1.0, 0.0);
            g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        } else {
            g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
            g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        }

        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);
        g.glx.material_ambient_diffuse([1.0, 1.0, 1.0, 1.0]);

        g.glx.scale(8.0, 8.0, 8.0);
        g.glx.translate(0.1, 0.1, 0.0);

        if !self.do_spin && !self.twodee {
            // Tilt the cube a little, and make the start and end visible.
            g.glx.translate(-0.1, 0.0, 0.0);
            g.glx.rotate(140.0, 0.0, 1.0, 0.0);
            g.glx.rotate(30.0, 1.0, 0.0, 0.0);
        }

        if wire {
            g.glx.line_width(if self.depth > 4 {
                1.0
            } else if self.depth > 3 {
                2.0
            } else {
                3.0
            });
        }

        if self.path_tick > 0 {
            // Advancing the end point, drawing one partial path. This only
            // happens the first time round.
            if !down {
                self.path_end += 0.01 * self.speed;
            }
            if self.path_end >= 1.0 {
                self.path_start = 0.0;
                self.path_end = 1.0;
                self.path_tick = -1;
                self.pause = PAUSE_TICKS;
            }

            self.diam = self.thickness / (1i64 << self.depth) as f32;
            g.glx.polygon_offset(Some((0.0, 0.0)));
            let (d, s, e) = (self.depth, self.path_start, self.path_end);
            self.draw_curve(g, d, s, e);
        } else {
            // Advancing the start point, drawing two paths at different
            // resolutions.
            if self.pause > 0 {
                self.pause -= 1;
            } else {
                if !down {
                    self.path_start += 0.01 * self.speed;
                }
                if self.path_start > 1.0 {
                    self.path_start = 0.0;
                    self.depth += self.depth_tick;
                    self.pause = PAUSE_TICKS;
                    if self.depth > self.max_depth - 1 {
                        self.depth = self.max_depth;
                        self.depth_tick = -1;
                    } else if self.depth <= 1 {
                        self.depth = 1;
                        self.depth_tick = 1;
                    }
                }
            }

            let d1 = self.thickness / (1i64 << self.depth) as f32;
            let d2 = self.thickness / (1i64 << (self.depth + self.depth_tick).max(0)) as f32;
            self.diam = d1 * (1.0 - self.path_start) + d2 * self.path_start;

            // The coarse path retreats from the front while the fine one
            // advances into it. They are not joined where they meet, and
            // sometimes overlap, but it goes by too fast to see.
            g.glx.polygon_offset(Some((0.0, 0.0)));
            let (d, s, e) = (self.depth, self.path_start, self.path_end);
            self.draw_curve(g, d, s, e);

            g.glx.polygon_offset(Some((1.0, 1.0)));
            let (d, s) = (self.depth + self.depth_tick, self.path_start);
            if d >= 1 {
                self.draw_curve(g, d, 0.0, s);
            }
        }

        g.glx.polygon_offset(None);
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event {
            match key {
                '+' | '=' => {
                    self.depth = (self.depth + 1).min(self.max_depth);
                    return true;
                }
                '-' | '_' => {
                    self.depth = (self.depth - 1).max(1);
                    return true;
                }
                _ => {}
            }
        }
        if screenhack_event_helper(event) {
            self.depth += self.depth_tick;
            if self.depth > self.max_depth - 1 {
                self.depth = self.max_depth;
                self.depth_tick = -1;
            } else if self.depth <= 1 {
                self.depth = 1;
                self.depth_tick = 1;
            }
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let do_spin = g.res.bool("spin");
    let do_wander = g.res.bool("wander");

    let twodee = match g.res.string("mode") {
        "2d" | "2D" => true,
        "3d" | "3D" => false,
        _ => random().is_multiple_of(3),
    };
    let closed = match g.res.string("ends") {
        "closed" => true,
        "open" => false,
        _ => !random().is_multiple_of(3),
    };

    // More colours means more tube segments, so keeping this small is worth
    // doing. An open path wants its first and last colours to differ; a closed
    // one has to come back round to where it started.
    let asked = if twodee { 1024 } else { 128 };
    let (colors, ncolors) = if closed {
        let c = make_smooth_colormap(asked);
        let n = c.len();
        (c, n)
    } else {
        // Upstream fills twice as many and then halves the count, so only the
        // first half of the ramp is used and the two ends do not meet.
        let c = make_uniform_colormap(asked * 2);
        let n = c.len() / 2;
        (c, n)
    };

    let spin_speed = 0.04;
    let tilt_speed = spin_speed / 10.0;
    let wander_speed = 0.008;
    let spin_accel = 0.01;

    let thickness = g.res.float("thickness").clamp(0.01, 1.0) as f32;
    // Upstream's own ceiling is twenty, which in three dimensions would want a
    // cache of eight thousand million points. The cap here is what one frame
    // can carry; see MAX_POINTS.
    let max_depth = g.res.int("maxDepth").clamp(2, depth_limit(twodee));

    let mut tubes = Vec::new();
    let mut spheres = Vec::new();
    for faces in DLIST_FACES {
        // Upstream's tubes have no end caps: the sphere at each joint is what
        // closes them, and where there is no sphere the tube is too thin for
        // an open end to show.
        tubes.push((faces, TubeMesh::tube(faces, true, false, wire)));
        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        unit_sphere(&mut g.glx, faces, faces, wire);
        g.glx.end_list();
        spheres.push((faces, list));
    }

    let mut st = Hilbert {
        rot: Rotator::new(
            if do_spin { spin_speed } else { 0.0 },
            if do_spin { spin_speed } else { 0.0 },
            if do_spin { spin_speed } else { 0.0 },
            spin_accel,
            if do_wander { wander_speed } else { 0.0 },
            do_spin,
        ),
        rot2: Rotator::new(0.0, 0.0, 0.0, 0.0, tilt_speed, true),
        trackball: Trackball::new(),
        twodee,
        closed,
        ncolors,
        colors,
        depth: 2.min(max_depth - 1).max(1),
        depth_tick: 1,
        path_start: 0.0,
        path_end: 0.0,
        path_tick: 1,
        pause: 0,
        diam: 0.0,
        caches: Vec::new(),
        tubes,
        spheres,
        speed: (g.res.float("speed").max(0.0001)) as f32,
        max_depth,
        thickness,
        do_spin,
        wireframe: wire,
        dropped_to_wire: false,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
    g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
    g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
    g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:      30000",
    "*count:      30",
    "*showFPS:    False",
    "*wireframe:  False",
    "*spin:       True",
    "*wander:     False",
    "*speed:      1.0",
    "*mode:       random",
    "*ends:       random",
    "*maxDepth:   5",
    "*thickness:  0.25",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "2D or 3D",
    },
    SelectItem {
        value: "2d",
        label: "2D",
    },
    SelectItem {
        value: "3d",
        label: "3D",
    },
];

const ENDS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Open or closed paths",
    },
    SelectItem {
        value: "closed",
        label: "Closed",
    },
    SelectItem {
        value: "open",
        label: "Open",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.02, 10.0, 0.02, 2, "1.0"),
    Opt::slider("maxDepth", "Recursion levels", 2.0, 5.0, 1.0, 0, "5"),
    Opt::slider("thickness", "Line thickness", 0.01, 1.0, 0.01, 2, "0.25"),
    Opt::select("mode", "Dimensions", MODES, "random"),
    Opt::select("ends", "Ends", ENDS, "random"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hilbert",
    label: "Hilbert",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2011",
        video: Some("https://www.youtube.com/watch?v=NhKmipo_Ek4"),
        blurb: "The recursive Hilbert space-filling curve, 2D and 3D.",
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

    /// The defining property: the curve visits every cell of the grid exactly
    /// once, and never jumps.
    #[test]
    fn the_curve_fills_the_grid_without_crossing_itself() {
        for n in 1..=5 {
            let w = 1usize << n;
            let mut seen = vec![false; w * w];
            let mut prev: Option<(i32, i32)> = None;
            for t in 0..(w * w) as i64 {
                let (x, y) = t_to_xy(n, t, false);
                assert!(
                    (0..w as i32).contains(&x) && (0..w as i32).contains(&y),
                    "depth {n} step {t} left the grid at {x},{y}"
                );
                let i = y as usize * w + x as usize;
                assert!(!seen[i], "depth {n} visited {x},{y} twice");
                seen[i] = true;
                if let Some((px, py)) = prev {
                    let d = (x - px).abs() + (y - py).abs();
                    assert_eq!(d, 1, "depth {n} jumped from {px},{py} to {x},{y}");
                }
                prev = Some((x, y));
            }
            assert!(seen.iter().all(|&b| b), "depth {n} missed a cell");
        }
    }

    /// The same in three dimensions.
    #[test]
    fn the_3d_curve_fills_the_cube_without_crossing_itself() {
        for n in 1..=4 {
            let w = 1usize << n;
            let mut seen = vec![false; w * w * w];
            let mut prev: Option<(i32, i32, i32)> = None;
            for t in 0..(w * w * w) as i64 {
                let (x, y, z) = t_to_xyz(n, t, false);
                let inside = |v: i32| (0..w as i32).contains(&v);
                assert!(
                    inside(x) && inside(y) && inside(z),
                    "depth {n} step {t} left the cube at {x},{y},{z}"
                );
                let i = (z as usize * w + y as usize) * w + x as usize;
                assert!(!seen[i], "depth {n} visited {x},{y},{z} twice");
                seen[i] = true;
                if let Some((px, py, pz)) = prev {
                    let d = (x - px).abs() + (y - py).abs() + (z - pz).abs();
                    assert_eq!(d, 1, "depth {n} jumped to {x},{y},{z}");
                }
                prev = Some((x, y, z));
            }
            assert!(seen.iter().all(|&b| b), "depth {n} missed a cell");
        }
    }

    /// The closed variant differs from the ordinary one only in the outermost
    /// level, and its two ends land next to each other so the loop closes.
    #[test]
    fn a_closed_path_comes_back_to_where_it_started() {
        for n in 2..=5 {
            let w = 1i64 << n;
            let last = w * w - 1;
            let (x0, y0) = t_to_xy(n, 0, true);
            let (x1, y1) = t_to_xy(n, last, true);
            let d = (x1 - x0).abs() + (y1 - y0).abs();
            assert_eq!(d, 1, "depth {n} ends at {x1},{y1}, not next to {x0},{y0}");
        }
        for n in 2..=4 {
            let w = 1i64 << n;
            let last = w * w * w - 1;
            let (x0, y0, z0) = t_to_xyz(n, 0, true);
            let (x1, y1, z1) = t_to_xyz(n, last, true);
            let d = (x1 - x0).abs() + (y1 - y0).abs() + (z1 - z0).abs();
            assert_eq!(d, 1, "3D depth {n} does not close: {x1},{y1},{z1}");
        }
    }

    /// An open path does *not* close, which is the whole difference.
    #[test]
    fn an_open_path_ends_somewhere_else() {
        let n = 4;
        let last = (1i64 << n) * (1i64 << n) - 1;
        let (x0, y0) = t_to_xy(n, 0, false);
        let (x1, y1) = t_to_xy(n, last, false);
        assert!(
            (x1 - x0).abs() + (y1 - y0).abs() > 1,
            "an open path came back to its start"
        );
    }

    /// The depth cap is what one frame can carry, and it bites harder in three
    /// dimensions than in two because there are eight children per level
    /// instead of four.
    #[test]
    fn the_depth_is_capped_by_what_a_frame_can_carry() {
        assert_eq!(depth_limit(false), 4, "3D");
        assert_eq!(depth_limit(true), 6, "2D");
        assert!((1i64 << (3 * depth_limit(false))) <= MAX_POINTS);
        assert!((1i64 << (2 * depth_limit(true))) <= MAX_POINTS);
    }

    /// It draws, in both dimensions.
    #[test]
    fn the_curve_is_drawn() {
        for mode in ["2d", "3d"] {
            let r = run(&format!("mode={mode}&maxDepth=3"), 30);
            let f = r.frame();
            assert!(!f.vertices.is_empty(), "{mode} drew nothing");
        }
    }
}
