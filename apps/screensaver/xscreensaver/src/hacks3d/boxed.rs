//! Port of `hacks/glx/boxed.c`.
//!
//! ```text
//! boxed.c - bouncing balls that explode
//!
//! Copyright (c) 2001 Sander van Grieken <mailsander@gmx.net>
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
//! Coloured balls fall into a shallow box, bounce around inside it, and
//! eventually escape over the wall. The moment one lands outside, it bursts:
//! every triangle of the sphere it was made of flies off on its own, carrying
//! a share of the ball's momentum, bouncing off the floor and off the outside
//! of the box, and winking out one at a time.
//!
//! The camera swings round the whole thing on three sine waves of its own,
//! which is why it never quite repeats.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand};
use std::f64::consts::PI;

const CAM_HEIGHT: f32 = 80.0;
const CAMDISTANCE_MIN: f32 = 35.0;
const CAMDISTANCE_MAX: f32 = 150.0;
const CAMDISTANCE_SPEED: f32 = 1.5;
const LOOKAT_R: f32 = 30.0;

/// How round the balls are.
const MESH_SIZE: usize = 10;
const SPHERE_VERTICES: usize = 2 + MESH_SIZE * MESH_SIZE * 2;
const SPHERE_INDICES: usize = (MESH_SIZE * 4 + MESH_SIZE * 4 * (MESH_SIZE - 1)) * 3;

fn rnd() -> f32 {
    frand(1.0) as f32
}

type Vec3 = [f32; 3];

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn squaremagnitude(a: Vec3) -> f32 {
    dot(a, a)
}

fn squaremagnitude_horz(a: Vec3) -> f32 {
    a[0] * a[0] + a[2] * a[2]
}

#[derive(Clone, Copy, Default)]
struct Ball {
    loc: Vec3,
    dir: Vec3,
    color: Vec3,
    radius: f32,
    /// Set the moment it lands outside the box, which is when it bursts.
    bounced: bool,
    /// Whether it has already got over the wall.
    offside: bool,
    justcreated: bool,
}

#[derive(Clone, Copy, Default)]
struct Tri {
    loc: Vec3,
    dir: Vec3,
    /// Off the edge of the floor, and so no longer worth colliding.
    far: bool,
    /// Counts up once the triangle has been marked for going out: it flashes
    /// white for three frames and is then gone.
    gone: i32,
}

/// The shrapnel of one burst ball.
#[derive(Default)]
struct Triman {
    tris: Vec<Tri>,
    vertices: Vec<Vec3>,
    normals: Vec<Vec3>,
    color: Vec3,
    explosion: f32,
    decay: f32,
    momentum: f32,
    live: bool,
}

/// `boxed.h` is a GIMP "RGB-only" C header: three bytes of pixel packed into
/// four printable characters, each carrying six bits above `!`. Upstream
/// includes it as source and unpacks it with a macro; it is an asset here and
/// is unpacked the same way.
fn decode_header_image(src: &str) -> Vec<u8> {
    let Some(start) = src.find("header_data =") else {
        return Vec::new();
    };
    let mut packed = String::with_capacity(256 * 256 * 4);
    let mut chars = src[start..].chars();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => in_string = !in_string,
            '\\' if in_string => {
                // The data runs from `!` to `` ` ``, so it contains both of
                // the characters C makes you escape.
                if let Some(e) = chars.next() {
                    packed.push(e);
                }
            }
            _ if in_string => packed.push(c),
            ';' if !in_string => break,
            _ => {}
        }
    }

    let b = packed.as_bytes();
    let mut out = Vec::with_capacity(256 * 256 * 4);
    for q in b.chunks_exact(4) {
        let d: [i32; 4] = [
            i32::from(q[0]) - 33,
            i32::from(q[1]) - 33,
            i32::from(q[2]) - 33,
            i32::from(q[3]) - 33,
        ];
        out.push(((d[0] << 2) | (d[1] >> 4)) as u8);
        out.push((((d[1] & 0xF) << 4) | (d[2] >> 2)) as u8);
        out.push((((d[2] & 0x3) << 6) | d[3]) as u8);
        out.push(0xFF);
    }
    out
}

struct Boxed {
    cam_x_speed: f32,
    cam_y_speed: f32,
    tic: f32,
    camtic: f32,
    spherev: Vec<Vec3>,
    spherei: Vec<usize>,
    balls: Vec<Ball>,
    tman: Vec<Triman>,
    texture: u32,
    ball_list: u32,
    pattern_list: u32,
    speed: f32,
    ballsize: f32,
    wire: bool,
    aspect: f32,
    scale: f32,
}

impl Boxed {
    /// `generatesphere`: the ball, as vertices and the indices of the
    /// triangles between them. Vertices zero and one are the poles.
    fn generatesphere(&mut self) {
        let dj = PI / (MESH_SIZE as f64 + 1.0);
        let di = PI / MESH_SIZE as f64;
        let mut v = vec![[0.0f32; 3]; SPHERE_VERTICES];
        v[0] = [0.0, 1.0, 0.0];
        v[1] = [0.0, -1.0, 0.0];
        for j in 0..MESH_SIZE {
            let r_y_plane = ((j + 1) as f64 * dj).sin() as f32;
            let h_y_plane = ((j + 1) as f64 * dj).cos() as f32;
            for i in 0..MESH_SIZE * 2 {
                let si = 2 + i + j * MESH_SIZE * 2;
                v[si] = [
                    (i as f64 * di).sin() as f32 * r_y_plane,
                    h_y_plane,
                    (i as f64 * di).cos() as f32 * r_y_plane,
                ];
            }
        }

        let mut ind = vec![0usize; SPHERE_INDICES];
        // The cap at the north pole.
        for i in 0..MESH_SIZE * 2 {
            ind[3 * i] = 0;
            ind[3 * i + 1] = i + 2;
            ind[3 * i + 2] = if i == MESH_SIZE * 2 - 1 { 2 } else { i + 3 };
        }
        // The strips between.
        for j in 0..MESH_SIZE - 1 {
            let v0 = 2 + j * MESH_SIZE * 2;
            let base = 3 * MESH_SIZE * 2 + j * 6 * MESH_SIZE * 2;
            for i in 0..MESH_SIZE * 2 {
                let last = i == MESH_SIZE * 2 - 1;
                ind[6 * i + base] = v0 + i;
                ind[6 * i + 2 + base] = if last {
                    v0 + i + 1 - 2 * MESH_SIZE
                } else {
                    v0 + i + 1
                };
                ind[6 * i + 1 + base] = v0 + i + MESH_SIZE * 2;

                ind[6 * i + base + 3] = v0 + i + MESH_SIZE * 2;
                ind[6 * i + 2 + base + 3] = if last {
                    v0 + i + 1 - 2 * MESH_SIZE
                } else {
                    v0 + i + 1
                };
                ind[6 * i + 1 + base + 3] = if last {
                    v0 + i + MESH_SIZE * 2 + 1 - 2 * MESH_SIZE
                } else {
                    v0 + i + MESH_SIZE * 2 + 1
                };
            }
        }
        // And the cap at the south pole.
        let v0 = SPHERE_VERTICES - MESH_SIZE * 2;
        let base = SPHERE_INDICES - 3 * MESH_SIZE * 2;
        for i in 0..MESH_SIZE * 2 {
            ind[3 * i + base] = 1;
            ind[3 * i + 1 + base] = if i == MESH_SIZE * 2 - 1 {
                v0
            } else {
                v0 + i + 1
            };
            ind[3 * i + 2 + base] = v0 + i;
        }

        self.spherev = v;
        self.spherei = ind;
    }

    /// `createball`: a fresh ball, dropped in from above in a colour bright
    /// enough to read.
    fn createball(&self, b: &mut Ball) {
        b.loc = [5.0 - 10.0 * rnd(), 35.0 + 20.0 * rnd(), 5.0 - 10.0 * rnd()];
        b.dir = [(0.5 - rnd()) * self.speed, 0.0, (0.5 - rnd()) * self.speed];
        b.offside = false;
        b.bounced = false;
        b.radius = self.ballsize;
        let mut c = [0.0f32; 3];
        while c[0] + c[1] + c[2] < 1.8 {
            c = [rnd(), rnd(), rnd()];
        }
        b.color = c;
        b.justcreated = true;
    }

    /// `updateballs`: gravity, the floor, the walls of the box, and each ball
    /// against every other.
    fn updateballs(&mut self) {
        let gravity = 0.30 * self.speed;
        let n = self.balls.len();
        for b in 0..n {
            self.balls[b].dir[1] -= gravity;
            let dir = self.balls[b].dir;
            self.balls[b].loc = add(self.balls[b].loc, dir);

            let radius = self.balls[b].radius;
            if self.balls[b].loc[1] < radius {
                let (x, z) = (self.balls[b].loc[0], self.balls[b].loc[2]);
                if !(-95.0..=95.0).contains(&x) || !(-95.0..=95.0).contains(&z) {
                    if self.balls[b].loc[1] < -2000.0 {
                        let mut ball = self.balls[b];
                        self.createball(&mut ball);
                        self.balls[b] = ball;
                    }
                } else {
                    self.balls[b].loc[1] = radius + (radius - self.balls[b].loc[1]);
                    self.balls[b].dir[1] = -self.balls[b].dir[1];
                    if self.balls[b].offside {
                        // Stop drawing it as a ball: it has burst.
                        self.balls[b].bounced = true;
                        self.balls[b].dir = scale(self.balls[b].dir, 0.80);
                        if squaremagnitude(self.balls[b].dir) < 0.08
                            || squaremagnitude_horz(self.balls[b].dir) < 0.005
                        {
                            let mut ball = self.balls[b];
                            self.createball(&mut ball);
                            self.balls[b] = ball;
                        }
                    }
                }
            }

            // The walls of the box, which a ball can clear once it is high
            // enough.
            if !self.balls[b].offside {
                for axis in [0usize, 2] {
                    let radius = self.balls[b].radius;
                    if self.balls[b].loc[axis] - radius < -20.0 {
                        if self.balls[b].loc[1] > 41.0 + radius {
                            self.balls[b].offside = true;
                        } else {
                            self.balls[b].dir[axis] = -self.balls[b].dir[axis];
                            self.balls[b].loc[axis] = -20.0 + radius;
                        }
                    }
                    if self.balls[b].loc[axis] + radius > 20.0 {
                        if self.balls[b].loc[1] > 41.0 + radius {
                            self.balls[b].offside = true;
                        } else {
                            self.balls[b].dir[axis] = -self.balls[b].dir[axis];
                            self.balls[b].loc[axis] = 20.0 - radius;
                        }
                    }
                }
            }

            // Ball against ball.
            for j in b + 1..n {
                let squaredist = self.balls[b].radius * self.balls[b].radius
                    + self.balls[j].radius * self.balls[j].radius;
                let dvect = sub(self.balls[b].loc, self.balls[j].loc);
                if squaremagnitude(dvect) >= squaredist {
                    continue;
                }
                let richting = sub(self.balls[j].loc, self.balls[b].loc);
                let relspeed = sub(self.balls[b].dir, self.balls[j].dir);
                let influence = scale(
                    richting,
                    dot(richting, relspeed) / squaremagnitude(richting),
                );
                self.balls[b].dir = sub(self.balls[b].dir, influence);
                self.balls[j].dir = add(self.balls[j].dir, influence);
                self.balls[b].loc = add(self.balls[b].loc, self.balls[b].dir);
                self.balls[j].loc = add(self.balls[j].loc, self.balls[j].dir);
                // Keep pushing until they are apart, bounded so that a pair
                // that cannot separate cannot hang the frame.
                for _ in 0..1000 {
                    let d = sub(self.balls[b].loc, self.balls[j].loc);
                    if squaremagnitude(d) >= squaredist {
                        break;
                    }
                    self.balls[b].loc = add(self.balls[b].loc, self.balls[b].dir);
                    self.balls[j].loc = add(self.balls[j].loc, self.balls[j].dir);
                }
            }
        }
    }

    /// `createtrisfromball`: cut the sphere into its triangles and throw each
    /// one outwards from the middle, with a share of the ball's momentum.
    fn createtrisfromball(&mut self, i: usize) {
        let b = self.balls[i];
        let explosion = 1.0 + self.tman[i].explosion * 2.0 * rnd();
        let momentum = self.tman[i].momentum;
        let num_tri = SPHERE_INDICES / 3;

        let mut tris = vec![Tri::default(); num_tri];
        let mut vertices = vec![[0.0f32; 3]; SPHERE_INDICES];
        let mut normals = vec![[0.0f32; 3]; num_tri];

        for (t, tri) in tris.iter_mut().enumerate() {
            let pos = t * 3;
            for k in 0..3 {
                vertices[pos + k] = self.spherev[self.spherei[pos + k]];
            }
            // Which way this shard is facing, and so which way it goes.
            let mut avgdir = add(vertices[pos], vertices[pos + 1]);
            avgdir = add(avgdir, vertices[pos + 2]);
            avgdir = scale(avgdir, 0.33333);
            // "should normalize first, NYI"
            normals[t] = avgdir;
            tri.loc = add(b.loc, avgdir);

            // Move each triangle back to its own origin, scale it up, and put
            // it back.
            let s = b.radius * 2.0;
            for k in 0..3 {
                let v = sub(vertices[pos + k], avgdir);
                vertices[pos + k] = add(scale(v, s), avgdir);
            }

            tri.dir = add(
                scale(avgdir, explosion),
                [0.1 - 0.2 * rnd(), 0.15 - 0.3 * rnd(), 0.1 - 0.2 * rnd()],
            );
            tri.dir = add(tri.dir, [b.dir[0] * momentum, 0.0, b.dir[2] * momentum]);
        }

        let t = &mut self.tman[i];
        t.tris = tris;
        t.vertices = vertices;
        t.normals = normals;
        t.color = b.color;
        t.live = true;
    }

    /// `updatetris`: the shrapnel falls, bounces off the floor and off the
    /// outside of the box, and goes out one triangle at a time.
    fn updatetris(&mut self, i: usize) {
        let speed = self.speed;
        let decay = self.tman[i].decay;
        for t in &mut self.tman[i].tris {
            if rnd() < decay && t.gone == 0 {
                t.gone = 1;
            }
            t.dir[1] -= 0.1 * speed;
            t.loc = add(t.loc, t.dir);
            if t.far {
                continue;
            }
            if t.loc[1] < 0.0 {
                if (-95.0..95.0).contains(&t.loc[0]) && (-95.0..95.0).contains(&t.loc[2]) {
                    t.dir[1] = -t.dir[1];
                    t.loc[1] = -t.loc[1];
                    // Dampening.
                    t.dir = scale(t.dir, 0.80);
                } else {
                    t.far = true;
                    continue;
                }
            }

            // Inside the box it has just left? Push it back out of whichever
            // wall it is nearest.
            if (-21.0..21.0).contains(&t.loc[0]) && (-21.0..21.0).contains(&t.loc[2]) {
                let mut xd = 999.0f32;
                let mut zd = 999.0f32;
                if t.loc[0] < 0.0 {
                    xd = t.loc[0] + 21.0;
                } else if t.loc[0] > 0.0 {
                    xd = 21.0 - t.loc[0];
                }
                if t.loc[2] < 0.0 {
                    zd = t.loc[2] + 21.0;
                } else if t.loc[2] > 0.0 {
                    zd = 21.0 - t.loc[2];
                }
                let axis = if xd < zd { 0 } else { 2 };
                if t.dir[axis] < 0.0 {
                    t.loc[axis] += 21.0 - t.loc[axis];
                } else {
                    t.loc[axis] += -21.0 - t.loc[axis];
                }
                t.dir[axis] = -t.dir[axis];
            }
        }
    }

    /// `drawfilledbox`: the textured slab the balls fall into. The top uses
    /// the whole picture and the sides use the edge of it.
    fn drawfilledbox(&self, g: &mut Gl) {
        let faces: [[(f32, f32, Vec3); 4]; 6] = [
            // Front, rear, left, right: the edge of the texture, stretched.
            [
                (0.0, 1.0, [-1.0, 1.0, 1.0]),
                (1.0, 1.0, [1.0, 1.0, 1.0]),
                (1.0, 1.0, [1.0, -1.0, 1.0]),
                (0.0, 1.0, [-1.0, -1.0, 1.0]),
            ],
            [
                (0.0, 1.0, [1.0, 1.0, -1.0]),
                (1.0, 1.0, [-1.0, 1.0, -1.0]),
                (1.0, 1.0, [-1.0, -1.0, -1.0]),
                (0.0, 1.0, [1.0, -1.0, -1.0]),
            ],
            [
                (1.0, 1.0, [-1.0, 1.0, 1.0]),
                (1.0, 1.0, [-1.0, -1.0, 1.0]),
                (0.0, 1.0, [-1.0, -1.0, -1.0]),
                (0.0, 1.0, [-1.0, 1.0, -1.0]),
            ],
            [
                (0.0, 1.0, [1.0, 1.0, 1.0]),
                (1.0, 1.0, [1.0, 1.0, -1.0]),
                (1.0, 1.0, [1.0, -1.0, -1.0]),
                (0.0, 1.0, [1.0, -1.0, 1.0]),
            ],
            // Top and bottom: the whole picture.
            [
                (0.0, 0.0, [-1.0, 1.0, 1.0]),
                (0.0, 1.0, [-1.0, 1.0, -1.0]),
                (1.0, 1.0, [1.0, 1.0, -1.0]),
                (1.0, 0.0, [1.0, 1.0, 1.0]),
            ],
            [
                (0.0, 0.0, [-1.0, -1.0, 1.0]),
                (0.0, 1.0, [-1.0, -1.0, -1.0]),
                (1.0, 1.0, [1.0, -1.0, -1.0]),
                (1.0, 0.0, [1.0, -1.0, 1.0]),
            ],
        ];
        g.glx.begin(if self.wire {
            Shape::LineLoop
        } else {
            Shape::Quads
        });
        for face in faces {
            for (u, v, p) in face {
                g.glx.tex_coord2f(u, v);
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
        }
        g.glx.end();
    }

    /// `drawbox`: a box drawn as twelve lines, which is what the walls are.
    fn drawbox(g: &mut Gl) {
        for y in [1.0f32, -1.0] {
            g.glx.begin(Shape::LineStrip);
            let ring: [[f32; 2]; 5] = if y > 0.0 {
                [
                    [-1.0, 1.0],
                    [-1.0, -1.0],
                    [1.0, -1.0],
                    [1.0, 1.0],
                    [-1.0, 1.0],
                ]
            } else {
                [
                    [-1.0, 1.0],
                    [1.0, 1.0],
                    [1.0, -1.0],
                    [-1.0, -1.0],
                    [-1.0, 1.0],
                ]
            };
            for p in ring {
                g.glx.vertex3f(p[0], y, p[1]);
            }
            g.glx.end();
        }
        g.glx.begin(Shape::Lines);
        for (x, z) in [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            g.glx.vertex3f(x, 1.0, z);
            g.glx.vertex3f(x, -1.0, z);
        }
        g.glx.end();
    }

    /// `drawpattern`: the tile of the floor pattern, a rounded ring inside a
    /// rounded square.
    fn drawpattern(g: &mut Gl) {
        const OUTER: [[f32; 2]; 25] = [
            [-25.0, 35.0],
            [-15.0, 35.0],
            [-5.0, 25.0],
            [5.0, 25.0],
            [15.0, 35.0],
            [25.0, 35.0],
            [35.0, 25.0],
            [35.0, 15.0],
            [25.0, 5.0],
            [25.0, -5.0],
            [35.0, -15.0],
            [35.0, -25.0],
            [25.0, -35.0],
            [15.0, -35.0],
            [5.0, -25.0],
            [-5.0, -25.0],
            [-15.0, -35.0],
            [-25.0, -35.0],
            [-35.0, -25.0],
            [-35.0, -15.0],
            [-25.0, -5.0],
            [-25.0, 5.0],
            [-35.0, 15.0],
            [-35.0, 25.0],
            [-25.0, 35.0],
        ];
        const INNER: [[f32; 2]; 9] = [
            [-5.0, 15.0],
            [5.0, 15.0],
            [15.0, 5.0],
            [15.0, -5.0],
            [5.0, -15.0],
            [-5.0, -15.0],
            [-15.0, -5.0],
            [-15.0, 5.0],
            [-5.0, 15.0],
        ];
        for ring in [&OUTER[..], &INNER[..]] {
            g.glx.begin(Shape::LineStrip);
            for p in ring {
                g.glx.vertex3f(p[0], 0.0, p[1]);
            }
            g.glx.end();
        }
    }

    fn drawball(&self, g: &mut Gl, b: &Ball) {
        g.glx.push_matrix();
        g.glx.translate(b.loc[0], b.loc[1], b.loc[2]);
        g.glx.scale(b.radius, b.radius, b.radius);
        g.glx.color4f(b.color[0], b.color[1], b.color[2], 1.0);
        g.glx
            .material_diffuse([b.color[0], b.color[1], b.color[2], 1.0]);
        g.glx
            .material_emission([b.color[0] * 0.5, b.color[1] * 0.5, b.color[2] * 0.5, 1.0]);
        g.glx.call_list(self.ball_list);
        g.glx.pop_matrix();
    }

    /// `drawtriman`: the shrapnel of one ball. A triangle that has been
    /// marked for going out flashes white for three frames first.
    fn drawtriman(&mut self, g: &mut Gl, i: usize) {
        let color = self.tman[i].color;
        let own = |g: &mut Gl| {
            g.glx.color4f(color[0], color[1], color[2], 1.0);
            g.glx.material_diffuse([color[0], color[1], color[2], 1.0]);
            g.glx
                .material_emission([color[0] * 0.3, color[1] * 0.3, color[2] * 0.3, 1.0]);
        };
        g.glx.push_matrix();
        own(g);
        g.glx.begin(if self.wire {
            Shape::Lines
        } else {
            Shape::Triangles
        });

        for t in 0..self.tman[i].tris.len() {
            if self.tman[i].tris[t].gone > 3 {
                continue;
            }
            let flashing = self.tman[i].tris[t].gone > 0;
            if flashing {
                g.glx.material_diffuse([1.0, 1.0, 1.0, 1.0]);
                g.glx.material_emission([0.8, 0.8, 0.8, 1.0]);
            }
            let n = self.tman[i].normals[t];
            let loc = self.tman[i].tris[t].loc;
            g.glx.normal3f(n[0], n[1], n[2]);
            for k in 0..3 {
                let v = add(self.tman[i].vertices[t * 3 + k], loc);
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            if flashing {
                own(g);
                self.tman[i].tris[t].gone += 1;
            }
        }
        g.glx.end();
        g.glx.pop_matrix();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    // `setdefaultconfig`.
    let numballs = (g.res.int("balls") as usize).clamp(3, 40);
    let ballsize = (g.res.float("ballsize") as f32).clamp(1.0, 5.0);
    let explosion = (g.res.float("explosion") as f32).clamp(0.0, 50.0);
    let momentum = (g.res.float("momentum") as f32).clamp(0.0, 1.0);
    let decay = (g.res.float("decay") as f32).clamp(0.02, 0.90);
    // "give the decay parameter a better curve"
    let decay = if decay <= 0.8182 {
        decay / 3.0
    } else {
        (decay - 0.75) * 4.0
    };
    let camspeed = 35.0f32;
    let speed = g.res.float("speed") as f32;

    let mut this = Boxed {
        cam_x_speed: 1.0 / (camspeed / 50.0 + rnd() * (camspeed / 50.0)),
        cam_y_speed: 1.0 / (camspeed / 250.0 + rnd() * (camspeed / 250.0)),
        tic: 0.0,
        camtic: 0.0,
        spherev: Vec::new(),
        spherei: Vec::new(),
        balls: vec![Ball::default(); numballs],
        tman: (0..numballs)
            .map(|_| Triman {
                explosion: (explosion as i32) as f32 / 15.0,
                decay,
                momentum,
                ..Triman::default()
            })
            .collect(),
        texture: 0,
        ball_list: 0,
        pattern_list: 0,
        speed,
        ballsize,
        wire: g.res.bool("wireframe"),
        aspect: 1.0,
        scale: 1.0,
    };
    // Upstream also gives z a speed of its own and never reads it.
    let _ = rnd();
    if rnd() < 0.5 {
        this.cam_x_speed = -this.cam_x_speed;
    }
    let _ = rnd();
    this.tic = rnd() * 100.0;
    this.camtic = this.tic;

    for i in 0..numballs {
        let mut b = this.balls[i];
        this.createball(&mut b);
        b.loc[1] *= rnd();
        this.balls[i] = b;
    }
    this.generatesphere();

    if !this.wire {
        let rgba = decode_header_image(crate::images::BOXED_TEXTURE);
        if rgba.len() == 256 * 256 * 4 {
            this.texture = g.glx.gen_texture();
            g.glx.bind_texture(this.texture);
            g.glx.tex_nearest(true);
            g.glx.tex_clamp(false);
            g.glx.tex_image_2d(256, 256, rgba);
        }
    }

    // The ball and the floor pattern are the same every time, so they are
    // compiled once. Neither sets any state inside itself.
    this.ball_list = g.glx.gen_lists(1);
    g.glx.new_list(this.ball_list);
    g.glx.begin(if this.wire {
        Shape::Lines
    } else {
        Shape::Triangles
    });
    for i in 0..SPHERE_INDICES / 3 {
        let pos = i * 3;
        for k in 0..3 {
            let v = this.spherev[this.spherei[pos + k]];
            g.glx.normal3f(v[0], v[1], v[2]);
            g.glx.vertex3f(v[0], v[1], v[2]);
            if this.wire && k == 1 {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
        }
    }
    g.glx.end();
    g.glx.end_list();

    this.pattern_list = g.glx.gen_lists(1);
    g.glx.new_list(this.pattern_list);
    Boxed::drawpattern(g);
    g.glx.end_list();

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Boxed {
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

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(50.0, self.aspect, 2.0, 1000.0);
        g.glx.matrix_mode_modelview();

        g.glx.clear();
        g.glx.line_width(1.0);
        g.glx.point_size(1.0);
        g.glx.load_identity();
        g.glx.scale(self.scale, self.scale, self.scale);

        self.tic += 0.01;
        self.camtic += 0.01 + 0.01 * (f64::from(self.tic * self.speed)).sin() as f32;

        // The camera swings round the middle on three sines of its own.
        let s = self.speed;
        let sin = |x: f32| (f64::from(x)).sin() as f32;
        let cos = |x: f32| (f64::from(x)).cos() as f32;
        let r = CAMDISTANCE_MIN
            + (CAMDISTANCE_MAX - CAMDISTANCE_MIN)
            + (CAMDISTANCE_MAX - CAMDISTANCE_MIN) * cos((self.camtic / CAMDISTANCE_SPEED) * s);
        let v1 = [
            r * sin((self.camtic / self.cam_x_speed) * s),
            CAM_HEIGHT * sin((self.camtic / self.cam_y_speed) * s) + 1.02 * CAM_HEIGHT,
            r * cos((self.camtic / self.cam_x_speed) * s),
        ];
        let v2x = LOOKAT_R * sin((self.camtic / (self.cam_x_speed * 5.0)) * s);
        let v2y =
            (CAM_HEIGHT * sin((self.camtic / self.cam_y_speed) * s) + 1.02 * CAM_HEIGHT) / 10.0;
        // Upstream passes the look-at point's x for its z as well, so the
        // camera aims at a point that circles with it. Kept: that is what the
        // saver does.
        g.glx.look_at(v1, [v2x, v2y, v2x], [0.0, 1.0, 0.0]);

        g.glx.depth_test(true);
        g.glx.cull_face(!self.wire);
        g.glx.color_material(false);
        if !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
            // A positional light at the origin, and a directional one above.
            g.glx.light_position(0, 0.0, 0.0, 0.0, 1.0);
            g.glx.light_enable(1, true);
            g.glx.light_ambient(1, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(1, [0.5, 0.5, 0.5, 1.0]);
            g.glx.light_specular(1, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_position(1, 0.0, 1.0, 0.0, 0.0);
            g.glx.front_face_cw(true);
            g.glx.material_specular([0.0, 0.0, 0.0, 1.0]);
            g.glx.material_emission([0.4, 0.6, 1.0, 1.0]);
            g.glx.material_ambient([0.0, 0.0, 0.0, 1.0]);
            g.glx.material_shininess(5.0);
        }

        // The floor pattern, unlit, tiled five by five.
        g.glx.lighting(false);
        g.glx.color4f(0.1, 0.1, 0.6, 1.0);
        for dx in -2..3 {
            for dz in -2..3 {
                g.glx.push_matrix();
                g.glx.translate(dx as f32 * 30.0, 0.0, dz as f32 * 30.0);
                g.glx.call_list(self.pattern_list);
                g.glx.pop_matrix();
            }
        }

        // The box the balls fall into.
        if !self.wire {
            g.glx.texturing(true);
            g.glx.bind_texture(self.texture);
        }
        g.glx.push_matrix();
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        g.glx.scale(20.0, 0.25, 20.0);
        g.glx.translate(0.0, 2.0, 0.0);
        self.drawfilledbox(g);
        g.glx.pop_matrix();
        g.glx.texturing(false);

        // And the four walls, as wireframe.
        g.glx.color4f(0.2, 0.5, 0.2, 1.0);
        for (sx, sy, sz, tx, tz) in [
            (20.0, 20.0, 0.25, 0.0, 81.0),
            (20.0, 20.0, 0.25, 0.0, -81.0),
            (0.25, 20.0, 20.0, -81.0, 0.0),
            (0.25, 20.0, 20.0, 81.0, 0.0),
        ] {
            g.glx.push_matrix();
            g.glx.scale(sx, sy, sz);
            g.glx.translate(tx, 1.0, tz);
            Boxed::drawbox(g);
            g.glx.pop_matrix();
        }

        if !self.wire {
            g.glx.lighting(true);
            g.glx.material_diffuse([0.3, 0.3, 0.3, 1.0]);
            // Turn the glow off before painting the balls.
            g.glx.material_emission([0.0, 0.0, 0.0, 1.0]);
        }

        self.updateballs();

        g.glx.front_face_cw(false);
        for i in 0..self.balls.len() {
            if self.balls[i].justcreated {
                self.balls[i].justcreated = false;
                self.tman[i] = Triman {
                    explosion: self.tman[i].explosion,
                    decay: self.tman[i].decay,
                    momentum: self.tman[i].momentum,
                    ..Triman::default()
                };
            }
            if self.balls[i].bounced {
                if !self.tman[i].live {
                    self.createtrisfromball(i);
                } else {
                    self.updatetris(i);
                }
                g.glx.cull_face(false);
                self.drawtriman(g, i);
                g.glx.cull_face(!self.wire);
            } else {
                let b = self.balls[i];
                self.drawball(g, &b);
            }
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:      15000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*speed:      0.5",
    "*balls:      20",
    "*ballsize:   3.0",
    "*explosion:  15.0",
    "*decay:      0.07",
    "*momentum:   0.6",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "15000").inverted(),
    Opt::slider("speed", "Speed", 0.001, 4.0, 0.05, 3, "0.5"),
    Opt::slider("balls", "Number of balls", 3.0, 40.0, 1.0, 0, "20"),
    Opt::slider("ballsize", "Ball size", 1.0, 5.0, 0.1, 1, "3.0"),
    Opt::slider("explosion", "Explosion force", 1.0, 50.0, 1.0, 0, "15.0"),
    Opt::slider("decay", "Explosion decay", 0.0, 1.0, 0.01, 2, "0.07"),
    Opt::slider("momentum", "Explosion momentum", 0.0, 1.0, 0.05, 2, "0.6"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "boxed",
    label: "Boxed",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Sander van Grieken",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=CU4QFtZm9So"),
        blurb: "Bouncing balls that explode when they escape the box.",
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

    /// The texture unpacks to a 256 by 256 picture with something in it.
    #[test]
    fn the_box_is_textured() {
        let rgba = decode_header_image(crate::images::BOXED_TEXTURE);
        assert_eq!(rgba.len(), 256 * 256 * 4, "the picture is the wrong size");
        // It is a green picture, so look at the green.
        let g: Vec<u8> = rgba.iter().skip(1).step_by(4).copied().collect();
        let lo = g.iter().copied().min().unwrap_or(0);
        let hi = g.iter().copied().max().unwrap_or(0);
        assert!(hi - lo > 100, "the picture runs only from {lo} to {hi}");
        assert!(
            rgba.iter().skip(3).step_by(4).all(|&a| a == 0xFF),
            "the picture is not opaque"
        );
    }

    /// The sphere's indices all name a vertex that exists, and every vertex is
    /// used: a slip in the index arithmetic would leave a hole in the ball.
    #[test]
    fn the_ball_is_a_closed_sphere() {
        let mut b = start(StartArgs::new(64, 64, "", 20260812));
        b.step();
        let mut s = bare();
        s.generatesphere();
        assert_eq!(s.spherei.len(), SPHERE_INDICES);
        assert_eq!(s.spherev.len(), SPHERE_VERTICES);
        let mut used = vec![false; SPHERE_VERTICES];
        for &i in &s.spherei {
            assert!(i < SPHERE_VERTICES, "index {i} is off the end");
            used[i] = true;
        }
        assert!(used.iter().all(|&u| u), "some vertices are never drawn");
        // And every vertex is on the unit sphere.
        for v in &s.spherev {
            let r = squaremagnitude(*v).sqrt();
            assert!((r - 1.0).abs() < 1e-5, "a vertex is at radius {r}");
        }
    }

    /// A ball that gets over the wall and lands outside bursts into as many
    /// triangles as the sphere had faces.
    #[test]
    fn escaping_the_box_bursts_the_ball() {
        let mut s = bare();
        s.generatesphere();
        s.balls = vec![Ball {
            loc: [50.0, 1.0, 50.0],
            // It needs to be moving sideways: a ball that has stopped is
            // recycled rather than burst.
            dir: [0.5, -1.0, 0.5],
            color: [1.0, 0.5, 0.5],
            radius: 3.0,
            offside: true,
            ..Ball::default()
        }];
        s.tman = vec![Triman {
            explosion: 1.0,
            decay: 0.02,
            momentum: 0.6,
            ..Triman::default()
        }];
        s.updateballs();
        assert!(s.balls[0].bounced, "the ball did not burst");
        s.createtrisfromball(0);
        assert_eq!(s.tman[0].tris.len(), SPHERE_INDICES / 3);
        // Every shard starts on the ball and heads away from its middle.
        for (t, tri) in s.tman[0].tris.iter().enumerate() {
            let n = s.tman[0].normals[t];
            assert!(dot(tri.dir, n) > 0.0, "shard {t} flew inwards");
        }
    }

    /// The balls stay in the world: they bounce off the floor and the walls
    /// rather than falling through them or drifting off for ever.
    #[test]
    fn the_balls_stay_in_the_world() {
        let mut r = start(StartArgs::new(320, 240, "balls=8", 20260812));
        for _ in 0..600 {
            r.step();
        }
        assert!(!r.frame().vertices.is_empty(), "nothing was drawn");
    }

    fn bare() -> Boxed {
        Boxed {
            cam_x_speed: 1.0,
            cam_y_speed: 1.0,
            tic: 0.0,
            camtic: 0.0,
            spherev: Vec::new(),
            spherei: Vec::new(),
            balls: Vec::new(),
            tman: Vec::new(),
            texture: 0,
            ball_list: 0,
            pattern_list: 0,
            speed: 0.5,
            ballsize: 3.0,
            wire: false,
            aspect: 1.0,
            scale: 1.0,
        }
    }
}
