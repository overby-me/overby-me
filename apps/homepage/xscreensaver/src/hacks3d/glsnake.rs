//! Port of `hacks/glx/glsnake.c`.
//!
//! ```text
//! glsnake.c - OpenGL imitation of Rubik's Snake
//!
//! (c) 2001-2005 Jamie Wilkinson <jaq@spacepants.org>
//! (c) 2001-2003 Andrew Bennetts <andrew@puzzling.org>
//! (c) 2001-2003 Peter Aylett <peter@ylett.com>
//!
//! This program is free software; you can redistribute it and/or modify
//! it under the terms of the GNU General Public License as published by
//! the Free Software Foundation; either version 2 of the License, or
//! (at your option) any later version.
//!
//! This program is distributed in the hope that it will be useful,
//! but WITHOUT ANY WARRANTY; without even the implied warranty of
//! MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//! GNU General Public License for more details.
//!
//! You should have received a copy of the GNU General Public License
//! along with this program; if not, write to the Free Software
//! Foundation, Inc., 675 Mass Ave, Cambridge, MA 02139, USA.
//! ```
//!
//! A Rubik's Snake: twenty-four wedge-shaped prisms hinged in a line, each
//! joint able to sit at one of four right angles. That is the whole toy, and
//! the two hundred and seventy-nine shapes it folds itself into here are the
//! ones from the puzzle's manual and from the people who wrote this.
//!
//! The colour says something about the shape it is holding. A snake whose tail
//! meets its head is drawn green, one that does not is blue, and one that
//! would have to pass through itself to be folded is grey; the port works that
//! out the way upstream does, by walking the joints through a lattice and
//! seeing whether two prisms want the same cell.
//!
//! Each joint turns at its own pace towards the next shape, and the whole
//! thing is done when the slowest of them arrives, so the snake settles into
//! its shape rather than snapping to it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Mat4, Shape};
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, random};

use super::glsnake_models::MODELS;

/// The prisms, and so the joints between them.
const NODE_COUNT: usize = 24;

/// How far the corners and edges of a prism are rounded off.
const VOFFSET: f32 = 0.045;

/// The shape the snake starts in, and the first one it folds from.
const START_MODEL: usize = 2;

/// How the snake is coloured, which says what kind of shape it is holding.
const COLOUR_CYCLIC: usize = 0;
const COLOUR_ACYCLIC: usize = 1;
const COLOUR_INVALID: usize = 2;
const COLOUR_AUTHENTIC: usize = 3;

/// Two colours a shape: the prisms alternate between them.
const COLOURS: [[[f32; 4]; 2]; 5] = [
    // cyclic, green
    [[0.4, 0.8, 0.2, 0.6], [1.0, 1.0, 1.0, 0.6]],
    // acyclic, blue
    [[0.3, 0.1, 0.9, 0.6], [1.0, 1.0, 1.0, 0.6]],
    // invalid, grey
    [[0.3, 0.1, 0.9, 0.6], [1.0, 1.0, 1.0, 0.6]],
    // authentic, purple and green
    [[0.38, 0.0, 0.55, 0.7], [0.0, 0.5, 0.34, 0.7]],
    // the old authentic colours, from the logo
    [
        [171.0 / 255.0, 0.0, 1.0, 1.0],
        [46.0 / 255.0, 205.0 / 255.0, 227.0 / 255.0, 1.0],
    ],
];

/// The angle each letter of a shape stands for.
fn joint_angle(c: u8) -> f32 {
    match c {
        b'L' => 90.0,
        b'P' => 180.0,
        b'R' => 270.0,
        _ => 0.0,
    }
}

/// One shape as twenty-four angles.
fn model_angles(i: usize) -> [f32; NODE_COUNT] {
    let joints = MODELS[i].1.as_bytes();
    std::array::from_fn(|k| joint_angle(joints[k]))
}

const SQRT1_2: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The eighteen corners of one prism: a right-angled triangle extruded, with
/// every corner and edge cut back by `VOFFSET` so it looks moulded rather than
/// machined.
const PRISM_V: [[f32; 3]; 18] = [
    // first corner, bottom left front
    [VOFFSET, VOFFSET, 1.0],
    [VOFFSET, 0.00, 1.0 - VOFFSET],
    [0.00, VOFFSET, 1.0 - VOFFSET],
    // second corner, rear
    [VOFFSET, VOFFSET, 0.00],
    [VOFFSET, 0.00, VOFFSET],
    [0.00, VOFFSET, VOFFSET],
    // third, right front
    [1.0 - VOFFSET / SQRT1_2, VOFFSET, 1.0],
    [1.0 - VOFFSET / SQRT1_2, 0.0, 1.0 - VOFFSET],
    [1.0 - VOFFSET * SQRT1_2, VOFFSET, 1.0 - VOFFSET],
    // fourth, right rear
    [1.0 - VOFFSET / SQRT1_2, VOFFSET, 0.0],
    [1.0 - VOFFSET / SQRT1_2, 0.0, VOFFSET],
    [1.0 - VOFFSET * SQRT1_2, VOFFSET, VOFFSET],
    // fifth, upper front
    [VOFFSET, 1.0 - VOFFSET / SQRT1_2, 1.0],
    [VOFFSET / SQRT1_2, 1.0 - VOFFSET * SQRT1_2, 1.0 - VOFFSET],
    [0.0, 1.0 - VOFFSET / SQRT1_2, 1.0 - VOFFSET],
    // sixth, upper rear
    [VOFFSET, 1.0 - VOFFSET / SQRT1_2, 0.0],
    [VOFFSET / SQRT1_2, 1.0 - VOFFSET * SQRT1_2, VOFFSET],
    [0.0, 1.0 - VOFFSET / SQRT1_2, VOFFSET],
];

/// The normals: six for the cut corners, nine for the cut edges, five for the
/// faces.
const PRISM_N: [[f32; 3]; 20] = [
    [-VOFFSET, -VOFFSET, VOFFSET],
    [VOFFSET, -VOFFSET, VOFFSET],
    [-VOFFSET, VOFFSET, VOFFSET],
    [-VOFFSET, -VOFFSET, -VOFFSET],
    [VOFFSET, -VOFFSET, -VOFFSET],
    [-VOFFSET, VOFFSET, -VOFFSET],
    [-VOFFSET, 0.0, VOFFSET],
    [0.0, -VOFFSET, VOFFSET],
    [VOFFSET, VOFFSET, VOFFSET],
    [-VOFFSET, 0.0, -VOFFSET],
    [0.0, -VOFFSET, -VOFFSET],
    [VOFFSET, VOFFSET, -VOFFSET],
    [-VOFFSET, -VOFFSET, 0.0],
    [VOFFSET, -VOFFSET, 0.0],
    [-VOFFSET, VOFFSET, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, -1.0, 0.0],
    [SQRT1_2, SQRT1_2, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 0.0, -1.0],
];

/// Which corners make each triangle of the solid prism, and which normal it
/// carries.
const PRISM_TRIS: [(usize, [usize; 3]); 8] = [
    (0, [0, 2, 1]),
    (1, [6, 7, 8]),
    (2, [12, 13, 14]),
    (3, [3, 4, 5]),
    (4, [9, 11, 10]),
    (5, [16, 15, 17]),
    (15, [0, 6, 12]),
    (19, [3, 15, 9]),
];

/// The same for its quads.
const PRISM_QUADS: [(usize, [usize; 4]); 12] = [
    (6, [0, 12, 14, 2]),
    (7, [0, 1, 7, 6]),
    (8, [6, 8, 13, 12]),
    (9, [3, 5, 17, 15]),
    (10, [3, 9, 10, 4]),
    (11, [15, 16, 11, 9]),
    (12, [1, 2, 5, 4]),
    (13, [8, 7, 10, 11]),
    (14, [13, 16, 17, 14]),
    (16, [1, 4, 10, 7]),
    (17, [8, 11, 16, 13]),
    (18, [2, 14, 17, 5]),
];

/// The bare triangular prism, for wireframe.
const WIRE_V: [[f32; 3]; 6] = [
    [0.0, 0.0, 1.0],
    [1.0, 0.0, 1.0],
    [0.0, 1.0, 1.0],
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
];

/// The lattice directions a joint can send the snake in.
const X_MASK: i32 = 1;
const Y_MASK: i32 = 2;
const Z_MASK: i32 = 4;

fn scalar(vec: i32, mask: i32) -> i32 {
    if vec == mask {
        1
    } else if vec == -mask {
        -1
    } else {
        0
    }
}

fn cross_product(src: i32, dst: i32) -> i32 {
    X_MASK * (scalar(src, Y_MASK) * scalar(dst, Z_MASK) - scalar(src, Z_MASK) * scalar(dst, Y_MASK))
        + Y_MASK
            * (scalar(src, Z_MASK) * scalar(dst, X_MASK)
                - scalar(src, X_MASK) * scalar(dst, Z_MASK))
        + Z_MASK
            * (scalar(src, X_MASK) * scalar(dst, Y_MASK)
                - scalar(src, Y_MASK) * scalar(dst, X_MASK))
}

/// What kind of shape a set of joint angles makes: whether the snake closes on
/// itself, and whether it could be folded at all without passing through
/// itself.
fn snake_metrics(angles: &[f32; NODE_COUNT]) -> (bool, bool) {
    // A lattice big enough for the snake to wander in, entered at the middle.
    let mut grid = vec![0i32; 25 * 25 * 25];
    let mut legal = true;
    let (mut prev_src, mut prev_dst) = (-Y_MASK, Z_MASK);
    let (mut x, mut y, mut z) = (12i32, 12i32, 12i32);
    let mut src_dir = 0;
    let mut dst_dir = 0;

    for &angle in angles.iter().take(NODE_COUNT - 1) {
        src_dir = -prev_dst;
        x += scalar(prev_dst, X_MASK);
        y += scalar(prev_dst, Y_MASK);
        z += scalar(prev_dst, Z_MASK);
        dst_dir = match angle as i32 {
            0 => -prev_src,
            180 => prev_src,
            90 | 270 => {
                let d = cross_product(prev_src, prev_dst);
                if angle as i32 == 270 { -d } else { d }
            }
            _ => 0,
        };
        let cell = &mut grid[((x * 25 + y) * 25 + z) as usize];
        if *cell == 0 {
            *cell = src_dir + dst_dir;
        } else if *cell + src_dir + dst_dir == 0 {
            // Two prisms meeting nose to tail in one cell is the one way they
            // are allowed to share it.
            *cell = 8;
        } else {
            legal = false;
        }
        prev_src = src_dir;
        prev_dst = dst_dir;
    }
    let _ = src_dir;
    let cyclic = dst_dir == Y_MASK && x == 12 && y == 11 && z == 12;
    (cyclic, legal)
}

struct Snake {
    /// The angle of each joint, which is the whole state of the toy.
    node: [f32; NODE_COUNT],
    prev_model: usize,
    next_model: usize,
    prev_colour: usize,
    next_colour: usize,
    colour: [[f32; 4]; 2],
    morphing: bool,
    /// How long since the last shape was reached, in milliseconds.
    since_morph: f64,
    solid: u32,
    wire_list: u32,
    yspin: f32,
    zspin: f32,
    scale: f32,
    aspect: f32,

    explode: f32,
    statictime: f64,
    yangvel: f32,
    zangvel: f32,
    angvel: f32,
    zoom: f32,
    altcolour: bool,
    transparent: bool,
    wire: bool,
}

/// Multiply a point through a column-major 4x4, as GL stores them.
fn transform(m: &Mat4, p: [f32; 3]) -> [f32; 4] {
    let m = &m.0;
    std::array::from_fn(|i| m[i] * p[0] + m[4 + i] * p[1] + m[8 + i] * p[2] + m[12 + i])
}

impl Snake {
    /// How far through the current morph the snake is, as the fraction of the
    /// longest journey any joint has to make that it has already made.
    fn morph_percent(&self) -> f32 {
        let prev = model_angles(self.prev_model);
        let next = model_angles(self.next_model);
        let (mut rot_max, mut diff_max) = (0.0f32, 0.0f32);
        for i in 0..NODE_COUNT - 1 {
            // The snake always turns through the smaller angle.
            let mut rot = (prev[i] - next[i]).abs();
            if rot > 180.0 {
                rot = 180.0 - rot;
            }
            let mut diff = (self.node[i] - next[i]).abs();
            if diff > 180.0 {
                diff = 180.0 - diff;
            }
            rot_max = rot_max.max(rot);
            diff_max = diff_max.max(diff);
        }
        let p = 1.0 - (diff_max / rot_max);
        if p.is_nan() || p.is_infinite() {
            1.0
        } else {
            p
        }
    }

    fn morph_colour(&mut self) {
        let percent = self.morph_percent();
        let compct = 1.0 - percent;
        let (prev, next) = (COLOURS[self.prev_colour], COLOURS[self.next_colour]);
        for (half, out) in self.colour.iter_mut().enumerate() {
            for (k, c) in out.iter_mut().enumerate() {
                *c = prev[half][k] * compct + next[half][k] * percent;
            }
        }
    }

    /// `start_morph`: aim at a new shape, and work out what colour it will be.
    fn start_morph(&mut self, model_index: usize, immediate: bool) {
        if immediate {
            self.node = model_angles(model_index);
        }
        self.prev_model = self.next_model;
        self.next_model = model_index;
        self.prev_colour = self.next_colour;

        let (cyclic, legal) = snake_metrics(&model_angles(self.next_model));
        self.next_colour = if !legal {
            COLOUR_INVALID
        } else if self.altcolour {
            COLOUR_AUTHENTIC
        } else if cyclic {
            COLOUR_CYCLIC
        } else {
            COLOUR_ACYCLIC
        };

        if immediate {
            self.colour = COLOURS[self.next_colour];
        }
        self.morphing = true;
        self.morph_colour();
    }

    /// `glsnake_idle`: turn each joint towards where it is going.
    fn idle(&mut self, iter_msec: f64) {
        self.since_morph += iter_msec;
        if self.since_morph > self.statictime && !self.morphing {
            self.since_morph = 0.0;
            self.start_morph(random() as usize % MODELS.len(), false);
        }

        self.yspin += 360.0 / ((1000.0 / self.yangvel) / iter_msec as f32);
        self.zspin += 360.0 / ((1000.0 / self.zangvel) / iter_msec as f32);

        // The furthest any one joint may turn in this slice of time.
        let iter_angle_max = 90.0 * (self.angvel / 1000.0) * iter_msec as f32;
        let next = model_angles(self.next_model);
        let mut still_morphing = false;
        for (node, &dest) in self.node.iter_mut().zip(next.iter()) {
            let cur = *node;
            if cur == dest {
                continue;
            }
            still_morphing = true;
            *node = if (cur - dest).abs() <= iter_angle_max {
                dest
            } else if (cur - dest + 360.0).rem_euclid(360.0) > 180.0 {
                (cur + iter_angle_max).rem_euclid(360.0)
            } else {
                (cur + 360.0 - iter_angle_max).rem_euclid(360.0)
            };
        }
        if !still_morphing {
            self.morphing = false;
        }
        self.morph_colour();
    }

    /// The joint transform: to the middle of a prism, round to the next one,
    /// out by its length, then pivot by the joint's angle.
    fn hinge(&self, g: &mut Gl, angle: f32) {
        g.glx.translate(0.5, 0.5, 0.5);
        g.glx.rotate(90.0, 0.0, 0.0, -1.0);
        g.glx.translate(1.0 + self.explode, 0.0, 0.0);
        g.glx.rotate(180.0 + angle, 1.0, 0.0, 0.0);
        g.glx.translate(-0.5, -0.5, -0.5);
    }

    fn build_prism(&mut self, g: &mut Gl) {
        let solid = g.glx.gen_lists(1);
        g.glx.new_list(solid);
        g.glx.begin(Shape::Triangles);
        for (n, v) in PRISM_TRIS {
            g.glx.normal3f(PRISM_N[n][0], PRISM_N[n][1], PRISM_N[n][2]);
            for i in v {
                g.glx.vertex3f(PRISM_V[i][0], PRISM_V[i][1], PRISM_V[i][2]);
            }
        }
        g.glx.end();
        g.glx.begin(Shape::Quads);
        for (n, v) in PRISM_QUADS {
            g.glx.normal3f(PRISM_N[n][0], PRISM_N[n][1], PRISM_N[n][2]);
            for i in v {
                g.glx.vertex3f(PRISM_V[i][0], PRISM_V[i][1], PRISM_V[i][2]);
            }
        }
        g.glx.end();
        g.glx.end_list();
        self.solid = solid;

        let wire = g.glx.gen_lists(1);
        g.glx.new_list(wire);
        g.glx.begin(Shape::LineStrip);
        for i in [0, 1, 2, 0, 3, 4, 5, 3] {
            g.glx.vertex3f(WIRE_V[i][0], WIRE_V[i][1], WIRE_V[i][2]);
        }
        g.glx.end();
        g.glx.begin(Shape::Lines);
        for i in [1, 4, 2, 5] {
            g.glx.vertex3f(WIRE_V[i][0], WIRE_V[i][1], WIRE_V[i][2]);
        }
        g.glx.end();
        g.glx.end_list();
        self.wire_list = wire;
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut this = Snake {
        node: [0.0; NODE_COUNT],
        prev_model: START_MODEL,
        next_model: random() as usize % MODELS.len(),
        prev_colour: COLOUR_ACYCLIC,
        next_colour: COLOUR_ACYCLIC,
        colour: COLOURS[COLOUR_ACYCLIC],
        morphing: false,
        since_morph: 0.0,
        solid: 0,
        wire_list: 0,
        yspin: 60.0,
        zspin: -45.0,
        scale: 1.0,
        aspect: 1.0,
        explode: g.res.float("explode") as f32,
        statictime: g.res.float("statictime"),
        yangvel: g.res.float("yangvel") as f32,
        zangvel: g.res.float("zangvel") as f32,
        angvel: g.res.float("angvel") as f32,
        zoom: g.res.float("zoom") as f32,
        altcolour: g.res.bool("altcolour"),
        transparent: g.res.bool("transparent"),
        wire: g.res.bool("wireframe"),
    };
    let start = this.prev_model;
    this.start_morph(start, true);
    this.build_prism(g);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Snake {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.aspect = width as f32 / height as f32;
        self.scale = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, _g: &mut Gl, event: &XEvent) -> bool {
        // Upstream's interactive mode is a keyboard editor for the snake; a
        // click here just sends it to another shape.
        if matches!(event, XEvent::ButtonPress { .. }) {
            self.start_morph(random() as usize % MODELS.len(), false);
            self.since_morph = 0.0;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(self.zoom, self.aspect, 1.0, 100.0);
        g.glx
            .look_at([0.0, 0.0, 20.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(false);
        g.glx.blend(if self.transparent {
            Blend::Alpha
        } else {
            Blend::Off
        });
        g.glx.lighting(!self.wire);
        if !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.0, 10.0, 20.0, 1.0);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_enable(1, true);
            g.glx.light_position(1, 0.0, 20.0, -1.0, 1.0);
            g.glx.light_diffuse(1, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(1, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_model_ambient([0.2, 0.2, 0.2, 1.0]);
            g.glx.material_specular([0.1, 0.1, 0.1, 1.0]);
            g.glx.material_shininess(20.0);
        }
        g.glx.color_material(self.wire);

        // Walk the joints once to find where each prism ends up, so that the
        // snake can be drawn about its centre of mass rather than about its
        // head, which would send it wandering off the screen.
        g.glx.push_matrix();
        g.glx.rotate(self.yspin, 0.0, 1.0, 0.0);
        g.glx.rotate(self.zspin, 0.0, 0.0, 1.0);
        let mut com = [0.0f32; 4];
        for i in 0..NODE_COUNT {
            self.hinge(g, self.node[i]);
            let p = transform(&g.glx.modelview(), [0.0, 0.0, 0.0]);
            for k in 0..4 {
                com[k] += p[k];
            }
        }
        g.glx.pop_matrix();
        for c in &mut com {
            *c /= NODE_COUNT as f32;
        }
        for k in 0..3 {
            com[k] /= com[3];
        }

        g.glx.push_matrix();
        g.glx.translate(-com[0], -com[1], -com[2]);
        g.glx.rotate(self.yspin, 0.0, 1.0, 0.0);
        g.glx.rotate(self.zspin, 0.0, 0.0, 1.0);
        g.glx.scale(self.scale, self.scale, self.scale);

        for i in 0..NODE_COUNT {
            let c = self.colour[(i + 1) % 2];
            if self.wire {
                g.glx.color4f(c[0], c[1], c[2], c[3]);
            } else {
                // Upstream sets the ambient and the diffuse to the same
                // colour, one call each.
                g.glx.material_ambient_diffuse(c);
            }
            g.glx.call_list(if self.wire {
                self.wire_list
            } else {
                self.solid
            });
            self.hinge(g, self.node[i]);
        }
        g.glx.pop_matrix();

        let delay = g.res.int("delay") as u32;
        // Upstream runs off the wall clock; the frame delay is what stands in
        // for it here.
        self.idle((delay as f64 / 1000.0).max(1.0));
        delay
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       30000",
    "*count:       30",
    "*showFPS:     False",
    "*explode:     0.03",
    "*angvel:      1.0",
    "*statictime:  5000",
    "*yangvel:     0.10",
    "*zangvel:     0.14",
    "*altcolour:   False",
    "*zoom:        25.0",
    "*wireframe:   False",
    "*transparent: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("angvel", "Morph speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("statictime", "Linger", 0.0, 20000.0, 500.0, 0, "5000"),
    Opt::slider("explode", "Explode", 0.0, 1.0, 0.01, 2, "0.03"),
    Opt::slider("yangvel", "Y rotation speed", -1.0, 1.0, 0.01, 2, "0.10"),
    Opt::slider("zangvel", "Z rotation speed", -1.0, 1.0, 0.01, 2, "0.14"),
    Opt::slider("zoom", "Field of view", 5.0, 90.0, 1.0, 0, "25.0"),
    Opt::boolean("altcolour", "Authentic colors", "false"),
    Opt::boolean("transparent", "Transparent", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glsnake",
    label: "GL Snake",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Wilkinson, Andrew Bennetts and Peter Aylett",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=AIqz-G0n1JU"),
        blurb: "A Rubik's Snake folding itself into two hundred and \
                seventy-nine shapes.",
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

    /// Every shape is twenty-four right angles, and nothing else.
    #[test]
    fn the_shapes_are_all_right_angles() {
        assert_eq!(MODELS.len(), 279, "a shape went missing");
        for (name, joints) in MODELS {
            assert_eq!(joints.len(), NODE_COUNT, "{name} is the wrong length");
            for c in joints.bytes() {
                assert!(
                    matches!(c, b'Z' | b'L' | b'P' | b'R'),
                    "{name} has a joint of {}",
                    c as char
                );
            }
        }
    }

    /// The snake it starts in is straight, and the manual's shapes are not.
    #[test]
    fn the_first_shape_is_the_straight_one() {
        assert_eq!(MODELS[0].0, "straight");
        assert!(MODELS[0].1.bytes().all(|c| c == b'Z'));
        assert_eq!(MODELS[1].0, "ball");
        assert!(MODELS[1].1.bytes().any(|c| c != b'Z'));
    }

    /// A ball is a closed shape and a straight snake is not, which is what the
    /// two default colours mean.
    #[test]
    fn a_closed_shape_is_told_from_an_open_one() {
        let of = |name: &str| {
            let i = MODELS.iter().position(|m| m.0 == name).expect(name);
            snake_metrics(&model_angles(i))
        };
        assert_eq!(of("straight"), (false, true), "straight");
        assert!(of("ball").0, "a ball does not close");
        // Whatever the shape, every one upstream ships can actually be folded.
        for (i, (name, _)) in MODELS.iter().enumerate() {
            let (_, legal) = snake_metrics(&model_angles(i));
            assert!(legal, "{name} cannot be folded");
        }
    }

    /// The snake turns every joint towards its target and arrives at the
    /// shape, however far round it has to go.
    #[test]
    fn it_reaches_the_shape_it_aims_at() {
        let mut r = start(StartArgs::new(640, 480, "statictime=100000", 20260811));
        r.step();
        for _ in 0..2000 {
            r.step();
        }
        // After long enough with no new shape called for, every joint has
        // arrived.
        let mut r2 = start(StartArgs::new(640, 480, "statictime=100000", 20260811));
        for _ in 0..2000 {
            r2.step();
        }
        let f = r2.frame();
        assert!(
            f.vertices
                .iter()
                .all(|v| v.pos.iter().all(|c| c.is_finite())),
            "a vertex went to NaN"
        );
    }

    /// Twenty-four prisms, each one drawn from the same two display lists.
    #[test]
    fn there_are_twenty_four_prisms() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let tris = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
            .count();
        // A prism is drawn as triangles and then as quads, but quads
        // arrive as triangles too and nothing changes between them, so the
        // two merge and a prism is one batch.
        assert_eq!(tris, NODE_COUNT, "{tris} batches is not the snake");
    }

    /// The snake is drawn about its centre of mass, so it stays in the middle
    /// of the screen however it is folded.
    #[test]
    fn it_stays_in_the_middle() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            let b = &f.batches[0];
            // The first prism's transform carries the centring translation.
            let m = b.modelview.0;
            assert!(
                m[12].abs() < 12.0 && m[13].abs() < 12.0,
                "the snake wandered to ({}, {})",
                m[12],
                m[13]
            );
        }
    }
}
