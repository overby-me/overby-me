//! Port of `hacks/glx/jigglypuff.c`.
//!
//! ```text
//! jigglypuff - a most, most, unfortunate screensaver.
//!
//! Copyright (c) 2003 Keith Macleod (kmacleod@primus.ca)
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Draws all varieties of obscene, spastic, puffy balls
//! orbiting lazily about the screen. More of an accident
//! than anything else.
//!
//! Apologies to anyone who thought they were getting a Pokemon
//! out of this.
//! ```
//!
//! A tetrahedron, subdivided a few times over and then optionally rounded into
//! a sphere, with a spring along every edge and a pull towards the unit sphere
//! at every vertex. The vertices have inertia, so the shape overshoots and
//! comes back, and the result ranges from a blob that barely moves to a
//! frenetic polygon storm.
//!
//! The mesh is a half-edge boundary representation, which is what makes the
//! subdivision cheap to write: a face is a ring of half-edges, each knowing
//! its neighbour, its edge and its vertex, and the whole shape is built by
//! splitting vertices and faces rather than by rebuilding index lists. Here
//! the four kinds of record live in arenas and point at each other by index.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Shape, TexEnv};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};
use std::f64::consts::PI;

/// The distance at which the edge springs sit still, at four subdivisions.
const STABLE_DISTANCE: f32 = 0.088_388_35;

const COLOR_STYLE_NORMAL: usize = 0;
const COLOR_STYLE_CYCLE: usize = 1;
const COLOR_STYLE_CLOWNBARF: usize = 2;
const COLOR_STYLE_FLOWERBOX: usize = 3;
const COLOR_STYLE_CHROME: usize = 4;

const CLOWNBARF_COLORS: [[f32; 4]; 5] = [
    [0.7, 0.7, 0.0, 1.0],
    [0.8, 0.1, 0.1, 1.0],
    [0.1, 0.1, 0.8, 1.0],
    [0.9, 0.9, 0.9, 1.0],
    [0.0, 0.0, 0.0, 1.0],
];

const FLOWERBOX_COLORS: [[f32; 4]; 4] = [
    [0.7, 0.7, 0.0, 1.0],
    [0.9, 0.0, 0.0, 1.0],
    [0.0, 0.9, 0.0, 1.0],
    [0.0, 0.0, 0.9, 1.0],
];

/// Upstream divides by twice `RAND_MAX`, so its "random" is only ever half a
/// unit. "Why isn't RAND_MAX correct in the first place?"
fn half_rand() -> f32 {
    (frand(0.5)) as f32
}

type Vec3 = [f32; 3];

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - b[1] * a[2],
        a[2] * b[0] - b[2] * a[0],
        a[0] * b[1] - b[0] * a[1],
    ]
}

fn magnitude2(v: Vec3) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

fn magnitude(v: Vec3) -> f32 {
    magnitude2(v).sqrt()
}

fn scale(v: Vec3, s: f32) -> Vec3 {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn add_to(a: &mut Vec3, b: Vec3) {
    a[0] += b[0];
    a[1] += b[1];
    a[2] += b[2];
}

fn normalize_to(v: Vec3, m: f32) -> Vec3 {
    let mag = 1.0 / magnitude(v) / m;
    scale(v, mag)
}

fn midpoint(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[0] + 0.5 * (b[0] - a[0]),
        a[1] + 0.5 * (b[1] - a[1]),
        a[2] + 0.5 * (b[2] - a[2]),
    ]
}

/// One side of an edge, going round one face. Its `next` and `prev` walk the
/// face's ring; its `e` finds the half-edge going the other way.
#[derive(Clone, Copy)]
struct Hedge {
    f: usize,
    e: Option<usize>,
    vtx: usize,
    next: usize,
    prev: usize,
}

#[derive(Clone, Copy)]
struct Edge {
    left: Option<usize>,
    right: Option<usize>,
    /// The solid's edge list, newest first, as upstream builds it.
    next: Option<usize>,
}

#[derive(Clone, Copy)]
struct Face {
    start: usize,
    color: [f32; 4],
    next: Option<usize>,
}

#[derive(Clone, Copy)]
struct Vertex {
    h: usize,
    /// Where it is, its normal, the force on it and its velocity.
    v: Vec3,
    n: Vec3,
    force: Vec3,
    vel: Vec3,
    next: Option<usize>,
}

struct Jigglypuff {
    stable_distance: f32,
    hold_strength: f32,
    spherify_strength: f32,
    damping_velocity: f32,
    damping_factor: f32,

    wire: bool,
    spooky: i32,
    color_style: usize,
    shininess: f32,
    jiggly_color: [f32; 4],
    color_dir: Vec3,

    trackball: Trackball,
    angle: f32,
    axis: f32,
    speed: f32,

    faces: Vec<Face>,
    edges: Vec<Edge>,
    hedges: Vec<Hedge>,
    vertices: Vec<Vertex>,
    /// The heads of the solid's three lists.
    face_head: Option<usize>,
    edge_head: Option<usize>,
    vertex_head: Option<usize>,

    texid: u32,
    aspect: f32,
    scale: f32,
}

impl Jigglypuff {
    /// The half-edge on the other side of this one's edge.
    fn partner(&self, h: usize) -> usize {
        let Some(e) = self.hedges[h].e else {
            return h;
        };
        if self.edges[e].left == Some(h) {
            self.edges[e].right.unwrap_or(h)
        } else {
            self.edges[e].left.unwrap_or(h)
        }
    }

    fn vertex_new(&mut self, v: Vec3) -> usize {
        let i = self.vertices.len();
        self.vertices.push(Vertex {
            h: 0,
            v,
            n: [0.0; 3],
            force: [0.0; 3],
            vel: [0.0; 3],
            next: self.vertex_head,
        });
        self.vertex_head = Some(i);
        i
    }

    /// Insert a new half-edge just after `hafter` in its face's ring.
    fn hedge_new(&mut self, hafter: usize, vtx: usize) -> usize {
        let i = self.hedges.len();
        let after = self.hedges[hafter];
        self.hedges.push(Hedge {
            f: after.f,
            e: None,
            vtx,
            next: after.next,
            prev: hafter,
        });
        self.hedges[hafter].next = i;
        let n = self.hedges[i].next;
        self.hedges[n].prev = i;
        i
    }

    fn edge_new(&mut self) -> usize {
        let i = self.edges.len();
        self.edges.push(Edge {
            left: None,
            right: None,
            next: self.edge_head,
        });
        self.edge_head = Some(i);
        i
    }

    fn face_new(&mut self, h: usize) -> usize {
        let i = self.faces.len();
        self.faces.push(Face {
            start: h,
            color: [1.0; 4],
            next: self.face_head,
        });
        self.face_head = Some(i);
        i
    }

    /// Split a vertex, putting a new one at `v` and a new edge to reach it.
    fn vertex_split(&mut self, h: usize, v: Vec3) -> usize {
        let h2 = self.partner(h);
        let vtxn = self.vertex_new(v);
        let hn1 = self.hedge_new(h, vtxn);
        self.vertices[vtxn].h = hn1;
        let hn2 = self.hedge_new(h2, vtxn);
        self.hedges[hn2].e = self.hedges[h].e;

        if let Some(e2) = self.hedges[h2].e {
            if self.edges[e2].left == Some(h2) {
                self.edges[e2].left = Some(hn2);
            } else {
                self.edges[e2].right = Some(hn2);
            }
        }

        let en = self.edge_new();
        self.edges[en].left = Some(hn1);
        self.edges[en].right = Some(h2);
        self.hedges[hn1].e = Some(en);
        self.hedges[h2].e = Some(en);
        vtxn
    }

    /// Cut a face in two along a new edge between the two given half-edges.
    fn face_split(&mut self, f: usize, h1: usize, h2: usize) -> usize {
        // Close the two loops.
        let (p1, p2) = (self.hedges[h1].prev, self.hedges[h2].prev);
        self.hedges[p1].next = h2;
        self.hedges[p2].next = h1;
        self.hedges[h1].prev = p2;
        self.hedges[h2].prev = p1;

        let (v1, v2) = (self.hedges[h1].vtx, self.hedges[h2].vtx);
        let a = self.hedges[h2].prev;
        let hn1 = self.hedge_new(a, v1);
        let b = self.hedges[h1].prev;
        let hn2 = self.hedge_new(b, v2);
        let en = self.edge_new();
        self.edges[en].left = Some(hn1);
        self.edges[en].right = Some(hn2);
        self.hedges[hn1].e = Some(en);
        self.hedges[hn2].e = Some(en);

        // The new face starts at whichever of the two is not in the old one.
        let mut tmp = self.faces[f].start;
        while tmp != h1 && tmp != h2 {
            tmp = self.hedges[tmp].next;
        }
        let tmp = if tmp == h1 { h2 } else { h1 };
        let fnew = self.face_new(tmp);
        let mut walk = tmp;
        loop {
            self.hedges[walk].f = fnew;
            walk = self.hedges[walk].next;
            if walk == self.faces[fnew].start {
                break;
            }
        }
        self.faces[fnew].color = self.faces[f].color;
        fnew
    }

    /// The degenerate starting solid: one vertex, one edge, two faces.
    fn solid_new(&mut self, where_: Vec3) {
        let h1 = self.hedges.len();
        self.hedges.push(Hedge {
            f: 0,
            e: None,
            vtx: 0,
            next: h1,
            prev: h1,
        });
        let h2 = self.hedges.len();
        self.hedges.push(Hedge {
            f: 0,
            e: None,
            vtx: 0,
            next: h2,
            prev: h2,
        });

        let vtx = self.vertex_new(where_);
        self.vertices[vtx].h = h1;
        self.hedges[h1].vtx = vtx;
        self.hedges[h2].vtx = vtx;

        let e = self.edge_new();
        self.edges[e].left = Some(h1);
        self.edges[e].right = Some(h2);
        self.hedges[h1].e = Some(e);
        self.hedges[h2].e = Some(e);

        let f1 = self.face_new(h1);
        let f2 = self.face_new(h2);
        self.hedges[h1].f = f1;
        self.hedges[h2].f = f2;
    }

    /// `face_tessel2`: fan a face out into triangles.
    fn face_tessel2(&mut self, f: usize) {
        let mut f = f;
        let start = self.faces[f].start;
        let mut h1 = self.hedges[start].prev;
        let mut h2 = self.hedges[start].next;
        if self.hedges[h1].next == h1 {
            return;
        }
        while h2 != h1 && self.hedges[h2].next != h1 {
            f = self.face_split(f, h1, h2);
            h1 = self.faces[f].start;
            h2 = self.hedges[self.hedges[h1].next].next;
        }
    }

    /// `solid_tesselate`: a vertex in the middle of every edge, then the
    /// faces walked and the dots joined.
    ///
    /// Upstream relies on new faces and edges going on the *head* of their
    /// lists, so that walking from the old head visits only what was there
    /// before; the lists here are built the same way round for that reason.
    fn solid_tesselate(&mut self) {
        let mut e = self.edge_head;
        while let Some(ei) = e {
            let (l, r) = (self.edges[ei].left, self.edges[ei].right);
            if let (Some(l), Some(r)) = (l, r) {
                let v = midpoint(
                    self.vertices[self.hedges[l].vtx].v,
                    self.vertices[self.hedges[r].vtx].v,
                );
                self.vertex_split(l, v);
            }
            e = self.edges[ei].next;
        }
        let mut f = self.face_head;
        while let Some(fi) = f {
            self.face_tessel2(fi);
            f = self.faces[fi].next;
        }
    }

    fn solid_spherify(&mut self, size: f32) {
        let mut v = self.vertex_head;
        while let Some(vi) = v {
            self.vertices[vi].v = normalize_to(self.vertices[vi].v, size);
            v = self.vertices[vi].next;
        }
    }

    /// Build the tetrahedron out of the degenerate solid, one split at a time.
    fn tetrahedron(&mut self) {
        self.solid_new([1.0, 1.0, 1.0]);
        let h = self.faces[self.face_head.unwrap_or(0)].start;
        let vtx = self.vertex_split(h, [-1.0, -1.0, 1.0]);
        let vtx = self.vertex_split(self.vertices[vtx].h, [-1.0, 1.0, -1.0]);
        let h = self.vertices[vtx].h;
        let head = self.face_head.unwrap_or(0);
        let f = self.face_split(head, h, self.hedges[h].prev);
        self.vertex_split(self.faces[f].start, [1.0, -1.0, -1.0]);

        // The third face down the list, which the splits have reordered.
        let mut f = self.face_head.unwrap_or(0);
        for _ in 0..2 {
            f = self.faces[f].next.unwrap_or(f);
        }
        let h = self.faces[f].start;
        let h2 = self.hedges[self.hedges[h].next].next;
        self.face_split(f, h, h2);

        if self.color_style == COLOR_STYLE_FLOWERBOX {
            let mut f = self.face_head;
            for c in FLOWERBOX_COLORS {
                let Some(fi) = f else { break };
                self.faces[fi].color = c;
                f = self.faces[fi].next;
            }
        }
    }

    fn clownbarf_colorize(&mut self) {
        let mut f = self.face_head;
        while let Some(fi) = f {
            self.faces[fi].color = CLOWNBARF_COLORS[random() as usize % CLOWNBARF_COLORS.len()];
            f = self.faces[fi].next;
        }
    }

    /// The normal at a vertex: the faces round it, added up.
    fn vertex_calcnormal(&mut self, vtx: usize) {
        let start = self.vertices[vtx].h;
        let mut h = start;
        let mut n = [0.0f32; 3];
        loop {
            let u = sub(
                self.vertices[self.hedges[self.hedges[h].prev].vtx].v,
                self.vertices[vtx].v,
            );
            let v = sub(
                self.vertices[self.hedges[self.hedges[h].next].vtx].v,
                self.vertices[vtx].v,
            );
            add_to(&mut n, cross(u, v));
            h = self.hedges[self.partner(h)].next;
            if h == start {
                break;
            }
        }
        // Spookiness leaves the normals unnormalised, so a long one dominates
        // its neighbours where they are interpolated across a face.
        self.vertices[vtx].n = if self.spooky == 0 {
            normalize_to(n, 1.0)
        } else {
            scale(n, self.spooky as f32)
        };
    }

    fn render(&mut self, g: &mut Gl) {
        let mut v = self.vertex_head;
        while let Some(vi) = v {
            self.vertex_calcnormal(vi);
            v = self.vertices[vi].next;
        }

        let coloured =
            self.color_style == COLOR_STYLE_FLOWERBOX || self.color_style == COLOR_STYLE_CLOWNBARF;
        let mut f = self.face_head;
        while let Some(fi) = f {
            if coloured {
                let c = self.faces[fi].color;
                g.glx.color4f(c[0], c[1], c[2], c[3]);
            }
            g.glx.begin(if self.wire {
                Shape::LineLoop
            } else {
                Shape::Triangles
            });
            let mut h1 = self.faces[fi].start;
            let hend = self.hedges[h1].prev;
            let mut h2 = self.hedges[h1].next;
            while h1 != hend && h2 != hend {
                for h in [h1, h2, hend] {
                    let vtx = &self.vertices[self.hedges[h].vtx];
                    g.glx.normal3f(vtx.n[0], vtx.n[1], vtx.n[2]);
                    g.glx.vertex3f(vtx.v[0], vtx.v[1], vtx.v[2]);
                }
                h1 = h2;
                h2 = self.hedges[h1].next;
            }
            g.glx.end();
            f = self.faces[fi].next;
        }
    }

    /// `update_shape`: every edge is a spring, every vertex is pulled towards
    /// the unit sphere, and both together move it.
    fn update_shape(&mut self) {
        let mut e = self.edge_head;
        while let Some(ei) = e {
            if let (Some(l), Some(r)) = (self.edges[ei].left, self.edges[ei].right) {
                let (lv, rv) = (self.hedges[l].vtx, self.hedges[r].vtx);
                let d = sub(self.vertices[lv].v, self.vertices[rv].v);
                let mag = self.stable_distance - magnitude(d);
                let f = scale(d, mag);
                add_to(&mut self.vertices[lv].force, f);
                add_to(&mut self.vertices[rv].force, scale(f, -1.0));
            }
            e = self.edges[ei].next;
        }

        let mut v = self.vertex_head;
        while let Some(vi) = v {
            let vtx = &mut self.vertices[vi];
            vtx.force = scale(vtx.force, self.hold_strength);
            let mut to_sphere = vtx.v;
            let mag = 1.0 - magnitude(to_sphere);
            to_sphere = scale(to_sphere, mag * self.spherify_strength);
            add_to(&mut vtx.force, to_sphere);
            let force = vtx.force;
            add_to(&mut vtx.vel, force);
            vtx.force = [0.0; 3];
            if magnitude2(vtx.vel) > self.damping_velocity {
                vtx.vel = scale(vtx.vel, self.damping_factor);
            }
            let vel = vtx.vel;
            add_to(&mut vtx.v, vel);
            v = vtx.next;
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let complexity = g.res.int("complexity");
    let subdivs = match complexity {
        1 => 4,
        3 => 6,
        _ => 5,
    };

    let color = g.res.string("color").to_string();
    let mut color_style = match color.as_str() {
        "clownbarf" => COLOR_STYLE_CLOWNBARF,
        "flowerbox" => COLOR_STYLE_FLOWERBOX,
        "chrome" => COLOR_STYLE_CHROME,
        "cycle" => COLOR_STYLE_CYCLE,
        _ => COLOR_STYLE_NORMAL,
    };
    let mut jiggly_color = [1.0f32; 4];
    let mut color_dir = [0.0f32; 3];
    if color_style == COLOR_STYLE_CYCLE {
        jiggly_color = [
            half_rand() * 0.7 + 0.3,
            half_rand() * 0.7 + 0.3,
            half_rand() * 0.7 + 0.3,
            1.0,
        ];
        color_dir = [
            half_rand() / 100.0,
            half_rand() / 100.0,
            half_rand() / 100.0,
        ];
    } else if color_style == COLOR_STYLE_NORMAL {
        // A literal `#rrggbb`, which is what upstream falls through to.
        if let Some(pixel) = crate::runtime::color::parse_color(&color) {
            let (r, gg, b) = crate::runtime::color::unrgb(pixel);
            jiggly_color = [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0];
        }
    }

    let mut do_tetrahedron = g.res.bool("tetra");
    let mut wire = g.res.bool("wireframe");
    let mut spooky = g.res.int("spooky");
    let mut shininess = g.res.int("shininess") as f32;
    let mut speed = g.res.int("speed");
    let mut spherism = g.res.int("spherism");
    let mut hold = g.res.int("hold");
    let mut distance = g.res.int("distance");
    let mut damping = g.res.int("damping");

    if g.res.bool("random") {
        // `randomize_parameters`.
        do_tetrahedron = random() % 2 == 1;
        wire = random().is_multiple_of(4);
        color_style = random() as usize % 5;
        if color_style == COLOR_STYLE_NORMAL || color_style == COLOR_STYLE_CYCLE {
            jiggly_color = [
                half_rand() * 0.5 + 0.5,
                half_rand() * 0.5 + 0.5,
                half_rand() * 0.5 + 0.5,
                1.0,
            ];
            if color_style == COLOR_STYLE_CYCLE {
                color_dir = [
                    half_rand() / 100.0,
                    half_rand() / 100.0,
                    half_rand() / 100.0,
                ];
            }
        }
        spooky = if color_style != COLOR_STYLE_CHROME && random() % 2 == 1 {
            (random() % 6) as i32 + 4
        } else {
            0
        };
        shininess = (random() % 200) as f32;
        speed = (random() % 700) as i32 + 50;
        // It is dull if this is too high when it starts as a sphere.
        spherism = if do_tetrahedron {
            (random() % 500) as i32 + 20
        } else {
            (random() % 100) as i32 + 10
        };
        hold = (random() % 800) as i32 + 100;
        distance = (random() % 500) as i32 + 100;
        damping = (random() % 800) as i32 + 50;
    }

    // `calculate_parameters`: try to compensate for the instability at low
    // complexity.
    let dist_factor = match subdivs {
        6 => 2.0,
        5 => 1.0,
        _ => 0.5,
    };

    let mut this = Jigglypuff {
        stable_distance: (distance as f32 / 500.0) * (STABLE_DISTANCE / dist_factor),
        hold_strength: hold as f32 / 10000.0,
        spherify_strength: spherism as f32 / 10000.0,
        damping_velocity: damping as f32 / 100000.0,
        damping_factor: 0.0,
        wire,
        spooky: spooky << (subdivs - 3),
        color_style,
        shininess,
        jiggly_color,
        color_dir,
        trackball: Trackball::new(),
        angle: frand(180.0) as f32,
        axis: frand(PI) as f32,
        speed: speed as f32 / 1000.0,
        faces: Vec::new(),
        edges: Vec::new(),
        hedges: Vec::new(),
        vertices: Vec::new(),
        face_head: None,
        edge_head: None,
        vertex_head: None,
        texid: 0,
        aspect: 1.0,
        scale: 1.0,
    };
    this.damping_factor = 0.001 / this.hold_strength.max(this.spherify_strength);

    this.tetrahedron();
    for _ in 0..subdivs {
        this.solid_tesselate();
    }
    if !do_tetrahedron {
        this.solid_spherify(1.0);
    }
    if this.color_style == COLOR_STYLE_CLOWNBARF {
        this.clownbarf_colorize();
    }

    if this.color_style == COLOR_STYLE_CHROME {
        // The sky the chrome reflects, which is not there.
        if let Some((w, h, rgba)) = crate::runtime::png::decode_rgba(crate::images::JIGGLYMAP) {
            this.texid = g.glx.gen_texture();
            g.glx.bind_texture(this.texid);
            g.glx.tex_nearest(false);
            g.glx.tex_clamp(false);
            g.glx.tex_image_2d(w, h, rgba);
        }
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Jigglypuff {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        let mut h = height as f32 / width as f32;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = 1.0 / h;
        self.scale = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx
            .frustum(-0.5 * self.aspect, 0.5 * self.aspect, -0.5, 0.5, 1.0, 20.0);
        g.glx.matrix_mode_modelview();

        g.glx.clear();
        g.glx.depth_test(true);
        if self.wire {
            g.glx.cull_face(false);
        } else {
            g.glx.cull_face(true);
            g.glx.front_face_cw(true);
        }

        if self.color_style != COLOR_STYLE_CHROME {
            g.glx.texturing(false);
            g.glx.tex_gen_sphere(false);
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, -12.0, 8.0, 12.0, 0.0);
            g.glx.light_diffuse(0, [0.7, 0.7, 0.65, 1.0]);
            g.glx.light_enable(1, true);
            g.glx.light_position(1, 7.0, -5.0, 0.0, 0.0);
            g.glx.light_diffuse(1, [0.3, 0.2, 0.1, 1.0]);
            g.glx.color_material(true);
            let c = self.jiggly_color;
            g.glx.color4f(c[0], c[1], c[2], c[3]);
            g.glx.material_specular([0.9, 0.9, 0.9, 0.5]);
            g.glx.material_shininess(self.shininess);
        } else {
            // Chrome: the texture is wrapped on by sphere mapping, which
            // upstream's own OpenGL ES build has to leave out. `GL_DECAL`
            // against the default white vertex colour is the same picture as
            // modulating it, which is what there is here.
            g.glx.lighting(false);
            g.glx.bind_texture(self.texid);
            g.glx.texturing(true);
            g.glx.tex_gen_sphere(true);
            g.glx.tex_env(TexEnv::Modulate);
            g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        }

        g.glx.load_identity();
        g.glx.translate(0.0, 0.0, -10.0);
        g.glx.scale(self.scale, self.scale, self.scale);
        g.glx.rotate(
            self.angle,
            self.axis.sin(),
            self.axis.cos(),
            -self.axis.sin(),
        );
        g.glx.translate(0.0, 0.0, 5.0);
        if !self.trackball.button_down() {
            self.angle += self.speed;
            if self.angle >= 360.0 {
                self.angle -= 360.0;
            }
            self.axis += 0.01;
            if self.axis >= 2.0 * PI as f32 {
                self.axis -= 2.0 * PI as f32;
            }
        }
        g.glx.mult_matrix(self.trackball.matrix());

        if self.color_style == COLOR_STYLE_CYCLE {
            for i in 0..3 {
                self.jiggly_color[i] += self.color_dir[i];
                if self.jiggly_color[i] > 1.0 || self.jiggly_color[i] < 0.3 {
                    self.color_dir[i] = -self.color_dir[i];
                    self.jiggly_color[i] += self.color_dir[i];
                }
            }
            let c = self.jiggly_color;
            g.glx.color4f(c[0], c[1], c[2], c[3]);
        }

        self.render(g);
        self.update_shape();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       20000",
    "*showFPS:     False",
    "*wireframe:   False",
    "*color:       cycle",
    "*shininess:   100",
    "*complexity:  2",
    "*speed:       500",
    "*distance:    100",
    "*hold:        800",
    "*spherism:    75",
    "*damping:     500",
    "*random:      True",
    "*tetra:       False",
    "*spooky:      0",
];

const COLORS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "cycle",
        label: "Cycle",
    },
    crate::runtime::opts::SelectItem {
        value: "flowerbox",
        label: "Flower box",
    },
    crate::runtime::opts::SelectItem {
        value: "clownbarf",
        label: "Clown barf",
    },
    crate::runtime::opts::SelectItem {
        value: "chrome",
        label: "Chrome",
    },
];

const STARTS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "false",
        label: "Sphere",
    },
    crate::runtime::opts::SelectItem {
        value: "true",
        label: "Tetrahedron",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::boolean("random", "Randomize almost everything", "true"),
    Opt::select("color", "Coloration", COLORS, "cycle"),
    Opt::select("tetra", "Start as", STARTS, "false"),
    Opt::slider("speed", "Rotation speed", 50.0, 1000.0, 10.0, 0, "500"),
    Opt::slider("damping", "Inertial damping", 10.0, 1000.0, 10.0, 0, "500").inverted(),
    Opt::slider("hold", "Vertex-vertex force", 0.0, 1000.0, 10.0, 0, "800"),
    Opt::slider("complexity", "Complexity", 1.0, 3.0, 1.0, 0, "2"),
    Opt::slider("spherism", "Sphere strength", 0.0, 1000.0, 10.0, 0, "75"),
    Opt::slider(
        "distance",
        "Vertex-vertex behavior",
        0.0,
        1000.0,
        10.0,
        0,
        "100",
    ),
    Opt::slider("spooky", "Spookiness", 0.0, 12.0, 1.0, 0, "0"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "jigglypuff",
    label: "Jiggly Puff",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Keith Macleod",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=60vfs2WcDtE"),
        blurb: "Quasi-spherical objects are distorted: a tetrahedron whose \
                vertices are pulled towards a sphere and towards each other, \
                with inertia.",
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

    /// Euler's formula: a closed surface with no holes has two more vertices
    /// and faces than it has edges. If the half-edge bookkeeping were wrong,
    /// this is what would break.
    #[test]
    fn the_mesh_closes_up() {
        for subdivs in 0..4 {
            let mut j = bare();
            j.tetrahedron();
            for _ in 0..subdivs {
                j.solid_tesselate();
            }
            let count = |mut h: Option<usize>, next: &dyn Fn(usize) -> Option<usize>| {
                let mut n = 0;
                while let Some(i) = h {
                    n += 1;
                    h = next(i);
                }
                n
            };
            let v = count(j.vertex_head, &|i| j.vertices[i].next);
            let e = count(j.edge_head, &|i| j.edges[i].next);
            let f = count(j.face_head, &|i| j.faces[i].next);
            assert_eq!(
                v + f,
                e + 2,
                "at {subdivs} subdivisions: {v} + {f} != {e} + 2"
            );
        }
    }

    /// Every face comes out a triangle, which is what the subdivision and the
    /// drawing both assume.
    #[test]
    fn every_face_is_a_triangle() {
        let mut j = bare();
        j.tetrahedron();
        j.solid_tesselate();
        j.solid_tesselate();
        let mut f = j.face_head;
        let mut n = 0;
        while let Some(fi) = f {
            let start = j.faces[fi].start;
            let mut h = start;
            let mut sides = 0;
            loop {
                sides += 1;
                h = j.hedges[h].next;
                assert!(sides <= 8, "face {fi} does not close");
                if h == start {
                    break;
                }
            }
            assert_eq!(sides, 3, "face {fi} has {sides} sides");
            n += 1;
            f = j.faces[fi].next;
        }
        assert_eq!(n, 4 * 4 * 4, "{n} faces after two subdivisions");
    }

    /// Every half-edge's partner points back at it, and they are on opposite
    /// faces.
    #[test]
    fn every_half_edge_has_a_partner() {
        let mut j = bare();
        j.tetrahedron();
        j.solid_tesselate();
        for h in 0..j.hedges.len() {
            let p = j.partner(h);
            assert_ne!(p, h, "half-edge {h} is its own partner");
            assert_eq!(j.partner(p), h, "half-edge {h} and {p} disagree");
            assert_ne!(j.hedges[h].f, j.hedges[p].f, "{h} and {p} share a face");
        }
    }

    /// Rounding the tetrahedron off puts every vertex on the unit sphere.
    #[test]
    fn spherifying_puts_it_on_the_sphere() {
        let mut j = bare();
        j.tetrahedron();
        j.solid_tesselate();
        j.solid_tesselate();
        j.solid_spherify(1.0);
        let mut v = j.vertex_head;
        while let Some(vi) = v {
            let r = magnitude(j.vertices[vi].v);
            assert!((r - 1.0).abs() < 1e-5, "a vertex is at radius {r}");
            v = j.vertices[vi].next;
        }
    }

    /// The blob keeps jiggling rather than settling or flying apart.
    #[test]
    fn it_jiggles_without_exploding() {
        let mut r = start(StartArgs::new(
            320,
            240,
            "random=false&tetra=true",
            20260812,
        ));
        for _ in 0..400 {
            r.step();
        }
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing was drawn");
        for v in &f.vertices {
            let d = magnitude(v.pos);
            assert!(d < 10.0, "a vertex flew off to {d}");
        }
    }

    /// An empty shape, for the mesh tests: no display behind it.
    fn bare() -> Jigglypuff {
        Jigglypuff {
            stable_distance: 0.0,
            hold_strength: 0.1,
            spherify_strength: 0.1,
            damping_velocity: 1.0,
            damping_factor: 0.5,
            wire: false,
            spooky: 0,
            color_style: COLOR_STYLE_NORMAL,
            shininess: 100.0,
            jiggly_color: [1.0; 4],
            color_dir: [0.0; 3],
            trackball: Trackball::new(),
            angle: 0.0,
            axis: 0.0,
            speed: 0.5,
            faces: Vec::new(),
            edges: Vec::new(),
            hedges: Vec::new(),
            vertices: Vec::new(),
            face_head: None,
            edge_head: None,
            vertex_head: None,
            texid: 0,
            aspect: 1.0,
            scale: 1.0,
        }
    }
}
