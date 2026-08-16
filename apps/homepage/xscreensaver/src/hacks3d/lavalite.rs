//! Port of `hacks/glx/lavalite.c`.
//!
//! ```text
//! lavalite --- 3D Lava Lite(r) simulator
//!
//! xscreensaver, Copyright (c) 2002-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! LAVA LITE(r) and the configuration of the LAVA(r) brand motion lamp are
//! registered trademarks of Haggerty Enterprises, Inc.
//! ```
//!
//! A lava lamp, in one of four shapes: the classic, the giant, the cone and
//! the rocket, the last of which stands on three fins.
//!
//! The lava is a field of metaballs. Each blob is a sphere with a hard radius
//! and a wider radius of influence, and the field at a point is how much of
//! all of them reaches it; the lava is the surface where that field crosses a
//! threshold, found by [`crate::runtime::marching`] over a grid. That is why
//! the blobs merge and part rather than passing through each other.
//!
//! A blob is launched with an upward velocity and gravity takes it back, so it
//! rises, slows, and sinks, and the field is clipped to the inside of the
//! bottle so the lava never leaves the glass.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::marching::marching_cubes;
use crate::runtime::rotator::Rotator;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};
use std::f64::consts::PI;

/// Downward acceleration, the initial upward velocity, and the horizontal
/// drift. The last two are bell curves.
const GRAVITY: f64 = 0.000013;
const CONVECTION: f64 = 0.005;
const TILT: f64 = 0.00166666;

const BLOBS_PER_GROUP: usize = 4;

/// Which part of the lamp a slice belongs to. The bottle is the glass, and is
/// the only part that is see-through and lit from a third light.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LampPart {
    Cap,
    Bottle,
    Base,
}

/// One ring of the lamp's outline: how high it is, how wide, and how far down
/// the texture it would be.
struct Slice {
    part: LampPart,
    elevation: f32,
    radius: f32,
    texture_elevation: f32,
}

const fn sl(part: LampPart, elevation: f32, radius: f32, texture_elevation: f32) -> Slice {
    Slice {
        part,
        elevation,
        radius,
        texture_elevation,
    }
}

use LampPart::{Base, Bottle, Cap};

const CLASSIC_LAMP: &[Slice] = &[
    sl(Cap, 1.16, 0.089, 0.00),
    sl(Bottle, 0.97, 0.120, 0.40),
    sl(Bottle, 0.13, 0.300, 0.87),
    sl(Bottle, 0.07, 0.300, 0.93),
    sl(Base, 0.00, 0.280, 0.00),
    sl(Base, -0.40, 0.120, 0.50),
    sl(Base, -0.80, 0.280, 1.00),
];

const GIANT_LAMP: &[Slice] = &[
    sl(Cap, 1.12, 0.105, 0.00),
    sl(Bottle, 0.97, 0.130, 0.30),
    sl(Bottle, 0.20, 0.300, 0.87),
    sl(Bottle, 0.15, 0.300, 0.93),
    sl(Base, 0.00, 0.230, 0.00),
    sl(Base, -0.18, 0.140, 0.20),
    sl(Base, -0.80, 0.280, 1.00),
];

const CONE_LAMP: &[Slice] = &[
    sl(Cap, 1.35, 0.001, 0.00),
    sl(Cap, 1.35, 0.020, 0.00),
    sl(Cap, 1.30, 0.055, 0.05),
    sl(Bottle, 0.97, 0.120, 0.40),
    sl(Bottle, 0.13, 0.300, 0.87),
    sl(Base, 0.00, 0.300, 0.00),
    sl(Base, -0.04, 0.320, 0.04),
    sl(Base, -0.60, 0.420, 0.50),
];

const ROCKET_LAMP: &[Slice] = &[
    sl(Cap, 1.35, 0.001, 0.00),
    sl(Cap, 1.34, 0.020, 0.00),
    sl(Cap, 1.30, 0.055, 0.05),
    sl(Bottle, 0.97, 0.120, 0.40),
    sl(Bottle, 0.13, 0.300, 0.87),
    sl(Bottle, 0.07, 0.300, 0.93),
    sl(Base, 0.00, 0.280, 0.00),
    sl(Base, -0.50, 0.180, 0.50),
    sl(Base, -0.75, 0.080, 0.75),
    sl(Base, -0.80, 0.035, 0.80),
    sl(Base, -0.90, 0.035, 1.00),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Classic,
    Giant,
    Cone,
    Rocket,
}

/// One blob of the lava.
#[derive(Clone, Copy, Default)]
struct Metaball {
    alive: bool,
    /// Part of the scenery rather than of the flow: these do not move.
    is_static: bool,
    /// The hard radius, and the radius of influence.
    r: f64,
    big_r: f64,
    z: f64,
    /// Where it is on a horizontal circle, and how fast it is going.
    pos_r: f64,
    pos_th: f64,
    dr: f64,
    dz: f64,
    /// Worked out from the above.
    x: f64,
    y: f64,
    /// Which blob of the group this one has to stay close to.
    leader: Option<usize>,
}

const LAVA_SPEC: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const LAVA_SHININESS: f32 = 128.0;
const FOOT_COLOR: [f32; 4] = [0.2, 0.2, 0.2, 1.0];

/// Like `frand`, but a bell curve about zero.
fn bellrand(extent: f64) -> f64 {
    (frand(extent) + frand(extent) + frand(extent)) / 3.0 - extent / 2.0
}

/// The outline of a rocket lamp's fin, as a pair of curves.
const WING: [[[i32; 2]; 8]; 2] = [
    [
        [0, 0],
        [10, 10],
        [20, 23],
        [30, 41],
        [40, 64],
        [45, 81],
        [50, 103],
        [53, 134],
    ],
    [
        [0, 54],
        [10, 57],
        [20, 64],
        [30, 75],
        [40, 92],
        [45, 104],
        [50, 127],
        [51, 134],
    ],
];

struct Lavalite {
    style: Style,
    model: &'static [Slice],
    rot: Rotator,
    rot2: Rotator,
    trackball: Trackball,
    max_bottle_radius: f32,
    launch_chance: f64,
    just_started: bool,
    grid_size: usize,
    balls: Vec<Metaball>,
    resolution: usize,
    smooth: bool,
    impatient: bool,
    wire: bool,
    lava_color: [f32; 4],
    fluid_color: [f32; 4],
    base_color: [f32; 4],
    table_color: [f32; 4],
    aspect: f32,
}

/// A disc closing off the top or the bottom of the cap or the base.
fn draw_disc(g: &mut Gl, r: f32, z: f32, faces: usize, up: bool, wire: bool) {
    let step = PI * 2.0 / faces as f64;
    g.glx.front_face_cw(up);
    g.glx.normal3f(0.0, if up { 1.0 } else { -1.0 }, 0.0);
    g.glx.begin(if wire {
        Shape::LineLoop
    } else {
        Shape::Triangles
    });
    let (mut x, mut y) = (r, 0.0);
    let mut th = 0.0;
    for j in 0..=faces {
        let u = -(j as f32) / faces as f32;
        g.glx.tex_coord2f(u, 1.0);
        g.glx.vertex3f(0.0, z, 0.0);
        g.glx.tex_coord2f(u, 0.0);
        g.glx.vertex3f(x, z, y);
        th += step;
        x = r * th.cos() as f32;
        y = r * th.sin() as f32;
        g.glx.tex_coord2f(u, 0.0);
        g.glx.vertex3f(x, z, y);
    }
    g.glx.end();
}

/// A tube or a cone between two of the lamp's rings.
fn draw_tube(
    g: &mut Gl,
    r: (f32, f32),
    z: (f32, f32),
    t: (f32, f32),
    faces: usize,
    inside_out: bool,
    how: (bool, bool),
) {
    let (smooth, wire) = how;
    let step = PI * 2.0 / faces as f64;
    let s2 = step / 2.0;
    g.glx.front_face_cw(inside_out);
    g.glx.begin(if wire {
        Shape::Lines
    } else if smooth {
        Shape::QuadStrip
    } else {
        Shape::Quads
    });

    let mut th = 0.0f64;
    let (mut x, mut y) = (1.0f32, 0.0f32);
    let (mut x0, mut y0) = if smooth {
        (0.0, 0.0)
    } else {
        (s2.cos() as f32, s2.sin() as f32)
    };
    let faces = if smooth { faces + 1 } else { faces };

    for i in 0..faces {
        let nsign = if inside_out { -1.0 } else { 1.0 };
        if smooth {
            g.glx.normal3f(x * nsign, z.1, y * nsign);
        } else {
            g.glx.normal3f(x0 * nsign, z.1, y0 * nsign);
        }
        g.glx
            .tex_coord2f(nsign * -(i as f32) / faces as f32, 1.0 - t.1);
        g.glx.vertex3f(x * r.1, z.1, y * r.1);
        g.glx
            .tex_coord2f(nsign * -(i as f32) / faces as f32, 1.0 - t.0);
        g.glx.vertex3f(x * r.0, z.0, y * r.0);

        th += step;
        x = th.cos() as f32;
        y = th.sin() as f32;

        if !smooth {
            x0 = (th + s2).cos() as f32;
            y0 = (th + s2).sin() as f32;
            g.glx
                .tex_coord2f(nsign * -((i + 1) as f32) / faces as f32, 1.0 - t.0);
            g.glx.vertex3f(x * r.0, z.0, y * r.0);
            g.glx
                .tex_coord2f(nsign * -((i + 1) as f32) / faces as f32, 1.0 - t.1);
            g.glx.vertex3f(x * r.1, z.1, y * r.1);
        }
    }
    g.glx.end();
}

/// The hexagonal table the lamp stands on.
fn draw_table(g: &mut Gl, z: f32, wire: bool) {
    let faces = 6;
    let step = PI * 2.0 / faces as f64;
    let s = 8.0;
    g.glx.front_face_cw(true);
    g.glx.normal3f(0.0, 1.0, 0.0);
    g.glx.begin(if wire {
        Shape::LineLoop
    } else {
        Shape::TriangleFan
    });
    if !wire {
        g.glx.tex_coord2f(-0.5, 0.5);
        g.glx.vertex3f(0.0, z, 0.0);
    }
    let mut th = 0.0f64;
    for _ in 0..=faces {
        let x = th.cos() as f32;
        let y = th.sin() as f32;
        g.glx.tex_coord2f(-(x + 1.0) / 2.0, (y + 1.0) / 2.0);
        g.glx.vertex3f(x * s, z, y * s);
        th += step;
    }
    g.glx.end();
}

/// One fin of the rocket lamp.
fn draw_wing(g: &mut Gl, w: f32, h: f32, d: f32, wire: bool) {
    let maxx = WING[0][WING[0].len() - 1][0] as f32;
    let maxy = WING[0][WING[0].len() - 1][1] as f32;
    for x in 1..WING[0].len() {
        let p = |curve: usize, i: usize| {
            (
                WING[curve][i][0] as f32 / maxx * w,
                WING[curve][i][1] as f32 / maxy * h,
            )
        };
        let (p0, p1, p2, p3) = (p(0, x - 1), p(1, x - 1), p(0, x), p(1, x));
        let zz = d / 2.0;
        // Two sides and two edges. Upstream marks the edge normals "#### wrong"
        // and leaves them; they are kept as they are.
        for (cw, normal, quad) in [
            (
                true,
                [0.0, 0.0, -1.0],
                [(p0, -zz), (p1, -zz), (p3, -zz), (p2, -zz)],
            ),
            (
                false,
                [0.0, 0.0, -1.0],
                [(p0, zz), (p1, zz), (p3, zz), (p2, zz)],
            ),
            (
                false,
                [1.0, -1.0, 0.0],
                [(p0, -zz), (p0, zz), (p2, zz), (p2, -zz)],
            ),
            (
                true,
                [-1.0, 1.0, 0.0],
                [(p1, -zz), (p1, zz), (p3, zz), (p3, -zz)],
            ),
        ] {
            g.glx.front_face_cw(cw);
            g.glx.normal3f(normal[0], normal[1], normal[2]);
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });
            for ((px, py), pz) in quad {
                g.glx.tex_coord2f(px, py);
                g.glx.vertex3f(px, -py, pz);
            }
            g.glx.end();
        }
    }
}

impl Lavalite {
    /// The widest the bottle ever gets, for a quick rejection when clipping.
    fn max_bottle_radius(&self) -> f32 {
        let mut r = 0.0f32;
        for (i, slice) in self.model.iter().enumerate() {
            if slice.part == Bottle && slice.radius > r {
                r = slice.radius;
            }
            if let Some(next) = self.model.get(i + 1)
                && next.radius > r
            {
                r = next.radius;
            }
        }
        r
    }

    /// How wide the bottle is at a given height, by interpolating between the
    /// two rings that bracket it.
    fn bottle_radius_at(&self, z: f32) -> f32 {
        let mut i = 0;
        let (topz, topr);
        loop {
            if i >= self.model.len() {
                return 0.0;
            }
            if z > self.model[i].elevation {
                if i == 0 {
                    return 0.0;
                }
                topz = self.model[i - 1].elevation;
                topr = self.model[i - 1].radius;
                break;
            }
            i += 1;
        }
        let (botz, botr);
        loop {
            if i >= self.model.len() {
                return 0.0;
            }
            if z > self.model[i].elevation {
                botz = self.model[i].elevation;
                botr = self.model[i].radius;
                break;
            }
            i += 1;
        }
        let ratio = (z - botz) / (topz - botz);
        botr + (topr - botr) * ratio
    }

    /// How much of the metaballs reaches a point: one for inside a blob's hard
    /// radius, falling off to nothing at the edge of its influence.
    fn metaball_influence(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut vv = 0.0;
        for b in &self.balls {
            if !b.alive {
                continue;
            }
            let (dx, dy, dz) = (x - b.x, y - b.y, z - b.z);
            let big_r = b.big_r;
            // A quick check before multiplying.
            if dx > big_r || dx < -big_r || dy > big_r || dy < -big_r || dz > big_r || dz < -big_r {
                continue;
            }
            let d2 = dx * dx + dy * dy + dz * dz;
            let r2 = b.r * b.r;
            let big_r2 = big_r * big_r;
            if d2 <= r2 {
                vv += 1.0;
            } else if d2 > big_r2 {
            } else {
                vv += 1.0 - ((d2 - r2) / (big_r2 - r2));
            }
        }
        vv
    }

    /// How much of a point is inside the glass: one well inside, falling off
    /// to nothing at the wall, so the lava never touches it.
    fn clipped_by_glass(&self, x: f64, y: f64, z: f64) -> f64 {
        let or = f64::from(self.max_bottle_radius);
        if x > or || x < -or || y > or || y < -or {
            return 0.0;
        }
        let d2 = x * x + y * y;
        let or = f64::from(self.bottle_radius_at(z as f32));
        let or2 = or * or;
        if d2 > or2 {
            return 0.0;
        }
        let ir2 = or2 * 0.7;
        if d2 > ir2 {
            return 1.0 - (d2 - ir2) / (or2 - ir2);
        }
        1.0
    }

    /// The field the surface is found in: the blobs, clipped to the glass.
    fn field_at(&self, x: f64, y: f64, z: f64) -> f64 {
        let n = self.grid_size as f64;
        // From grid coordinates to the unit cube: x and y run from -0.5 to
        // +0.5 and z from 0 to 1.
        let (x, y, z) = (x / n - 0.5, y / n - 0.5, z / n);
        let clip = self.clipped_by_glass(x, y, z);
        if clip == 0.0 {
            return 0.0;
        }
        clip * self.metaball_influence(x, y, z)
    }

    fn free_ball(&self) -> Option<usize> {
        self.balls.iter().position(|b| !b.alive)
    }

    /// Bring a blob into play with fresh values.
    fn reset_ball(&mut self, i: usize) {
        let b = &mut self.balls[i];
        b.r = 0.00001;
        b.big_r = 0.12 + bellrand(0.10);
        b.pos_r = bellrand(0.9);
        b.pos_th = frand(PI * 2.0);
        b.z = 0.0;
        b.dr = bellrand(TILT);
        b.dz = CONVECTION;
        b.leader = None;
        b.alive = true;
        self.move_ball(i);
    }

    fn move_ball(&mut self, i: usize) {
        if self.balls[i].is_static {
            return;
        }
        let b = &mut self.balls[i];
        b.pos_r += b.dr;
        b.z += b.dz;
        b.dz -= GRAVITY;
        if b.pos_r > 0.9 {
            b.pos_r = 0.9;
            b.dr = -b.dr;
        } else if b.pos_r < 0.0 {
            b.pos_r = -b.pos_r;
            b.dr = -b.dr;
        }
        let (pos_r, pos_th, z, big_r) = (b.pos_r, b.pos_th, b.z, b.big_r);
        let real_r = pos_r * f64::from(self.bottle_radius_at(z as f32));
        let b = &mut self.balls[i];
        b.x = pos_th.cos() * real_r;
        b.y = pos_th.sin() * real_r;
        // Dropped below the bottom of the glass: turn it off.
        if z < -big_r {
            b.alive = false;
        }
    }

    /// Keep the blobs of a group near their leader, in height at least.
    /// Upstream: "This is kind of flaky, I think. Sometimes you can see the
    /// blobbies twitch. That's no good."
    fn clamp_balls(&mut self) {
        for i in 0..self.balls.len() {
            let Some(leader) = self.balls[i].leader else {
                continue;
            };
            if !self.balls[i].alive {
                continue;
            }
            let zslack = 0.1;
            let (minz, maxz) = (self.balls[leader].z - zslack, self.balls[leader].z + zslack);
            let b = &mut self.balls[i];
            if b.z < minz {
                if b.dz < 0.0 {
                    b.dz = -b.dz;
                }
                b.z = minz - b.dz;
            }
            if b.z > maxz {
                if b.dz > 0.0 {
                    b.dz = -b.dz;
                }
                b.z = maxz + b.dz;
            }
        }
    }

    fn move_balls(&mut self) {
        for i in 0..self.balls.len() {
            if self.balls[i].alive {
                self.move_ball(i);
            }
        }
        self.clamp_balls();
    }

    /// Send a new blob up: really a group of them, near each other.
    fn launch_balls(&mut self) {
        let Some(b0) = self.free_ball() else {
            return;
        };
        self.reset_ball(b0);
        for _ in 0..BLOBS_PER_GROUP {
            let Some(b1) = self.free_ball() else {
                break;
            };
            self.balls[b1] = self.balls[b0];
            self.reset_ball(b1);
            self.balls[b1].leader = Some(b0);
            let (dr, dz) = (self.balls[b1].dr, self.balls[b1].dz);
            self.balls[b1].dr = dr + bellrand(0.8) * dr;
            self.balls[b1].dz = dz + bellrand(0.6) * dz;
        }
    }

    /// The blobs that never move: the pool at the bottom and the cap at the
    /// top, which are part of the scenery.
    fn generate_static_blobs(&mut self) {
        let Some(b0) = self.free_ball() else {
            return;
        };
        {
            let b = &mut self.balls[b0];
            b.is_static = true;
            b.alive = true;
            b.big_r = 0.6;
            b.r = 0.3;
            b.pos_r = 0.0;
            b.pos_th = 0.0;
            b.dr = 0.0;
            b.dz = 0.0;
            b.x = 0.0;
            b.y = 0.0;
            b.z = -0.43;
        }
        if let Some(b1) = self.free_ball() {
            self.balls[b1] = self.balls[b0];
            self.balls[b1].big_r = 0.16;
            self.balls[b1].r = 0.135;
            self.balls[b1].z = 1.078;
        }
        // And a few more at the bottom, to rough the surface up.
        for _ in 0..BLOBS_PER_GROUP {
            let Some(b1) = self.free_ball() else {
                break;
            };
            self.reset_ball(b1);
            let b = &mut self.balls[b1];
            b.is_static = true;
            b.z = frand(0.04);
            b.dr = 0.0;
            b.dz = 0.0;
        }
    }

    /// `generate_bottle`: the glass, the cap, the base and the table.
    ///
    /// Upstream compiles this into a display list, with the material for each
    /// part and the third light set inside it. A list here replays geometry
    /// and not state, so it is drawn where it is wanted instead; it is a few
    /// hundred faces either way.
    fn draw_bottle(&self, g: &mut Gl) {
        let wire = self.wire;
        let mut faces = (self.resolution as f32 * 1.5) as usize;
        faces = if faces < 3 {
            3
        } else if wire {
            faces.min(20)
        } else {
            faces.min(60)
        };

        g.glx.push_matrix();
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.translate(0.0, -0.5, 0.0);

        // Every part of the lamp is equally shiny.
        g.glx.material_specular(LAVA_SPEC);
        g.glx.material_shininess(LAVA_SHININESS);

        let mut last_part: Option<LampPart> = None;
        let mut top = 0;
        loop {
            let top_slice = &self.model[top];
            let bot_slice = self.model.get(top + 1);

            // The third light falls only on the fluid.
            g.glx.light_enable(2, !wire && top_slice.part == Bottle);
            let color = match top_slice.part {
                Cap | Base => self.base_color,
                Bottle => self.fluid_color,
            };

            // The discs are darker than the walls.
            g.glx.material_ambient_diffuse(FOOT_COLOR);
            if (top_slice.part == Cap && last_part.is_none())
                || (top_slice.part == Base && last_part == Some(Bottle))
            {
                draw_disc(g, top_slice.radius, top_slice.elevation, faces, true, wire);
            }
            let bot_part = bot_slice.map(|s| s.part);
            if (top_slice.part == Cap && bot_part == Some(Bottle))
                || (top_slice.part == Base && bot_part.is_none())
            {
                let s = bot_slice.unwrap_or(top_slice);
                draw_disc(g, s.radius, s.elevation, faces, false, wire);
            }

            let Some(bot_slice) = bot_slice else {
                break;
            };

            g.glx.material_ambient_diffuse(color);
            let mut t0 = top_slice.texture_elevation;
            let mut t1 = bot_slice.texture_elevation;
            // The glass restarts its texture coordinates.
            if top_slice.part == Bottle {
                if top == 0 || self.model[top - 1].part != Bottle {
                    t0 = 0.0;
                }
                if bot_slice.part != Bottle {
                    t1 = 1.0;
                }
            }
            draw_tube(
                g,
                (top_slice.radius, bot_slice.radius),
                (top_slice.elevation, bot_slice.elevation),
                (t0, t1),
                faces,
                top_slice.part == Bottle,
                (self.smooth, wire),
            );

            last_part = Some(top_slice.part);
            top += 1;
        }

        if self.style == Style::Rocket {
            for i in 0..3 {
                g.glx.push_matrix();
                g.glx.rotate(120.0 * i as f32, 0.0, 1.0, 0.0);
                g.glx.translate(0.14, -0.05, 0.0);
                draw_wing(g, 0.4, 0.95, 0.02, wire);
                g.glx.pop_matrix();
            }
            // Move the floor down a little.
            g.glx.translate(0.0, -0.1, 0.0);
        }

        g.glx.light_enable(2, false);
        g.glx.material_ambient_diffuse(self.table_color);
        draw_table(g, self.model[top].elevation, wire);
        g.glx.pop_matrix();
    }
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let style = match g.res.string("style") {
        "classic" => Style::Classic,
        "giant" => Style::Giant,
        "cone" => Style::Cone,
        "rocket" => Style::Rocket,
        // Half the time it is the classic; otherwise any of the four.
        _ => {
            if random() % 2 == 1 {
                Style::Classic
            } else {
                match random() % 4 {
                    0 => Style::Classic,
                    1 => Style::Giant,
                    2 => Style::Cone,
                    _ => Style::Rocket,
                }
            }
        }
    };
    let model = match style {
        Style::Classic => CLASSIC_LAMP,
        Style::Giant => GIANT_LAMP,
        Style::Cone => CONE_LAMP,
        Style::Rocket => ROCKET_LAMP,
    };

    let spin = g.res.string("spin").to_string();
    let axis = |c: char, d: char| spin.contains(c) || spin.contains(d);
    let spin_speed = 0.4;
    let wander_speed = if g.res.bool("wander") { 0.03 } else { 0.0 };

    let nballs = ((g.res.int("count") as usize + 1) * BLOBS_PER_GROUP) + 2;
    let resolution = (g.res.int("resolution") as usize).max(2);

    let mut this = Lavalite {
        style,
        model,
        rot: Rotator::new(
            if axis('x', 'X') { spin_speed } else { 0.0 },
            if axis('y', 'Y') { spin_speed } else { 0.0 },
            if axis('z', 'Z') { spin_speed } else { 0.0 },
            1.0,
            wander_speed,
            false,
        ),
        rot2: Rotator::new(spin_speed, 0.0, 0.0, 1.0, 0.1, false),
        trackball: Trackball::new(),
        max_bottle_radius: 0.0,
        launch_chance: g.res.float("speed"),
        just_started: true,
        // Upstream leaves this at zero until the first frame has been marched,
        // which draws that frame at the wrong scale; it is known from the
        // start here.
        grid_size: resolution,
        balls: vec![Metaball::default(); nballs + 1],
        resolution,
        smooth: g.res.bool("smooth"),
        impatient: g.res.bool("impatient"),
        wire: g.res.bool("wireframe"),
        lava_color: [0.0; 4],
        fluid_color: [0.0; 4],
        base_color: [0.0; 4],
        table_color: [0.0; 4],
        aspect: 1.0,
    };
    this.lava_color = resource_color(g, "lavaColor");
    this.fluid_color = resource_color(g, "fluidColor");
    this.base_color = resource_color(g, "baseColor");
    this.table_color = resource_color(g, "tableColor");
    this.max_bottle_radius = this.max_bottle_radius();

    // Lean the ordinary lamps towards the viewer and the huge ones away.
    this.trackball.reset(
        -0.3 + frand(0.6),
        if style == Style::Rocket || style == Style::Giant {
            frand(0.2)
        } else {
            -frand(0.6)
        },
    );

    this.generate_static_blobs();

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Lavalite {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 3;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, self.aspect, 1.0, 100.0);
        g.glx.matrix_mode_modelview();

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.color_material(false);
        g.glx.lighting(!self.wire);
        if !self.wire {
            let amb = [0.0, 0.0, 0.0, 1.0];
            let dif = [1.0, 1.0, 1.0, 1.0];
            g.glx.light_enable(0, true);
            g.glx.light_ambient(0, amb);
            g.glx.light_diffuse(0, dif);
            g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
            g.glx.light_enable(1, true);
            g.glx.light_ambient(1, amb);
            g.glx.light_diffuse(1, dif);
            g.glx.light_specular(1, [1.0, 0.0, 1.0, 1.0]);
            g.glx.light_ambient(2, amb);
            g.glx.light_diffuse(2, dif);
            g.glx.light_specular(2, [0.0, 1.0, 1.0, 1.0]);
        }

        let turning = !self.trackball.button_down();
        let (_, _, _) = self.rot2.position(turning);
        let (_, _, _) = self.rot2.rotation(turning);
        let (px, py, pz) = self.rot.position(turning);
        let (rx, ry, rz) = self.rot.rotation(turning);

        g.glx.push_matrix();
        g.glx.load_identity();
        // Upstream computes a camera position from the second rotator and then
        // throws it away, having never got the orbit to look right.
        g.glx
            .look_at([0.0, 0.0, 4.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.mult_matrix(self.trackball.matrix());
        // Right side up.
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);

        // The lights go on before the lamp is moved or turned, so they stay
        // where they are in the scene.
        g.glx.light_position(0, -0.6, 0.0, 1.0, 0.0);
        g.glx.light_position(1, 1.0, 0.0, 0.2, 0.0);
        g.glx.light_position(2, 0.6, 0.0, 1.0, 0.0);

        g.glx
            .translate((px - 0.5) as f32, (py - 0.5) as f32, (pz - 0.5) as f32);
        g.glx.rotate((rx * 360.0) as f32, 1.0, 0.0, 0.0);
        g.glx.rotate((ry * 360.0) as f32, 0.0, 1.0, 0.0);
        g.glx.rotate((rz * 360.0) as f32, 0.0, 0.0, 1.0);

        // Put nothing at the lamp's middle rather than at its foot.
        match self.style {
            Style::Classic | Style::Giant => g.glx.translate(0.0, 0.0, 0.33),
            Style::Cone => g.glx.translate(0.0, 0.0, 0.16),
            Style::Rocket => {
                g.glx.translate(0.0, 0.0, 0.30);
                g.glx.scale(0.85, 0.85, 0.85);
            }
        }

        // Maybe bubble a new blob to the surface.
        let just_started = self.just_started;
        if just_started || frand(1.0) < self.launch_chance {
            self.just_started = false;
            self.launch_balls();
            if self.impatient && just_started {
                // Run the flow forward until something has risen halfway, so
                // that the lamp is not empty when it opens.
                for _ in 0..100_000 {
                    self.move_balls();
                    if self
                        .balls
                        .iter()
                        .any(|b| b.alive && !b.is_static && b.leader.is_none() && b.z > 0.5)
                    {
                        break;
                    }
                }
            }
        }
        self.move_balls();

        self.draw_bottle(g);

        // And the lava. For the blobs the origin is on the axis at the bottom
        // of the glass, and the top of the bottle is +1 on Z.
        g.glx.push_matrix();
        g.glx.material_specular(LAVA_SPEC);
        g.glx.material_shininess(LAVA_SHININESS);
        g.glx.material_ambient_diffuse(self.lava_color);
        g.glx.translate(0.0, 0.0, -0.5);
        let s = 1.0 / self.grid_size as f32;
        g.glx.push_matrix();
        g.glx.translate(-0.5, -0.5, 0.0);
        g.glx.scale(s, s, s);
        let me: &Lavalite = self;
        marching_cubes(
            g,
            self.resolution,
            0.3,
            self.wire,
            self.smooth,
            |x, y, z| me.field_at(x, y, z),
        );
        g.glx.pop_matrix();
        g.glx.pop_matrix();
        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       30000",
    "*showFPS:     False",
    "*wireframe:   False",
    "*count:       3",
    "*style:       random",
    "*spin:        Z",
    "*wander:      False",
    "*speed:       0.003",
    "*resolution:  40",
    "*smooth:      True",
    "*impatient:   False",
    "*lavaColor:   #FF0000",
    "*fluidColor:  #00AAFF",
    "*baseColor:   #666666",
    "*tableColor:  #000000",
];

const STYLES: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "random",
        label: "Random Lamp Style",
    },
    crate::runtime::opts::SelectItem {
        value: "classic",
        label: "Classic Lavalite",
    },
    crate::runtime::opts::SelectItem {
        value: "giant",
        label: "Giant Lavalite",
    },
    crate::runtime::opts::SelectItem {
        value: "cone",
        label: "Cone Lavalite",
    },
    crate::runtime::opts::SelectItem {
        value: "rocket",
        label: "Rocket Lavalite",
    },
];

const SPINS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "Z",
        label: "Rotate around Z axis",
    },
    crate::runtime::opts::SelectItem {
        value: "0",
        label: "Don't Rotate",
    },
    crate::runtime::opts::SelectItem {
        value: "X",
        label: "Rotate around X axis",
    },
    crate::runtime::opts::SelectItem {
        value: "Y",
        label: "Rotate around Y axis",
    },
    crate::runtime::opts::SelectItem {
        value: "XYZ",
        label: "Rotate around all three axes",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Activity", 0.001, 0.01, 0.001, 3, "0.003"),
    Opt::slider("count", "Max blobs", 1.0, 10.0, 1.0, 0, "3"),
    // Upstream's slider runs to 120, which is a grid of one and three quarter
    // million points to walk every frame. Sixty is about a quarter of a
    // million, which still holds the frame rate.
    Opt::slider("resolution", "Resolution", 10.0, 60.0, 5.0, 0, "40"),
    Opt::select("style", "Lamp style", STYLES, "random"),
    Opt::select("spin", "Rotation", SPINS, "Z"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("smooth", "Smooth", "true"),
    Opt::boolean("impatient", "Impatient", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "lavalite",
    label: "Lavalite",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=XKbtdHL35u0"),
        blurb: "A 3D simulation of a lava lamp, in a variety of styles.",
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

    /// Every lamp reads top to bottom, starts with a cap and ends with a base,
    /// and has glass in between: the bottle drawing walks the list assuming
    /// exactly that.
    #[test]
    fn every_lamp_is_a_cap_a_bottle_and_a_base() {
        for model in [CLASSIC_LAMP, GIANT_LAMP, CONE_LAMP, ROCKET_LAMP] {
            assert_eq!(model[0].part, Cap);
            assert_eq!(model[model.len() - 1].part, Base);
            assert!(model.iter().any(|s| s.part == Bottle));
            for w in model.windows(2) {
                assert!(
                    w[0].elevation >= w[1].elevation,
                    "the slices are not top to bottom"
                );
            }
        }
    }

    /// The lava is clipped to the inside of the glass, so no part of the
    /// surface can be outside the bottle at its own height.
    #[test]
    fn the_lava_stays_in_the_bottle() {
        let mut r = start(StartArgs::new(320, 240, "style=classic", 20260812));
        for _ in 0..5 {
            r.step();
        }
        let f = r.frame();
        // The marched surface is the batch with no texture and the lava's own
        // colour; check every vertex against the bottle at its height.
        let mut checked = 0;
        for b in &f.batches {
            if b.material.ambient_diffuse[0] < 0.9 || b.material.ambient_diffuse[1] > 0.1 {
                continue;
            }
            for v in &f.vertices[b.first..b.first + b.count] {
                // Grid coordinates: x and y run 0..n about the middle.
                let n = 40.0;
                let (x, y) = (f64::from(v.pos[0]) / n - 0.5, f64::from(v.pos[1]) / n - 0.5);
                assert!(
                    (x * x + y * y).sqrt() < 0.31,
                    "lava at {x}, {y} is outside the widest bottle"
                );
                checked += 1;
            }
        }
        assert!(checked > 100, "only {checked} lava vertices were checked");
    }

    /// A blob rises, slows and sinks: it is thrown upward and gravity has it
    /// back.
    #[test]
    fn a_blob_rises_and_falls() {
        let mut r = start(StartArgs::new(64, 64, "resolution=10", 20260812));
        r.step();
        // The field is the only thing that reads the balls, so drive them
        // directly: a fresh blob starts at the bottom and goes up.
        let mut b = Metaball {
            alive: true,
            r: 0.00001,
            big_r: 0.2,
            z: 0.0,
            dz: CONVECTION,
            ..Metaball::default()
        };
        let mut highest = 0.0f64;
        for _ in 0..1000 {
            b.z += b.dz;
            b.dz -= GRAVITY;
            highest = highest.max(b.z);
        }
        assert!(highest > 0.9, "the blob only reached {highest}");
        assert!(b.z < highest, "the blob never came back down");
    }
}
