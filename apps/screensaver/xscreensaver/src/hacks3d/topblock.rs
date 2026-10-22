//! Port of `hacks/glx/topblock.c`.
//!
//! ```text
//! topblock, Copyright (c) 2006-2012 rednuht <topblock.xscreensaver@jumpstation.co.uk>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! topBlock - a simple openGL 3D hack of falling blocks
//! based on jwz's dangerball hack
//!
//! The proporations of the blocks and their features is not even close to the
//! commercial building block products offered by a variety companies.
//! ```
//!
//! Interlocking plastic bricks rain onto a green baseplate and pile up.
//!
//! A brick is not simulated so much as arbitrated. Every one falls at the same
//! rate, which is what stops two of them ever meeting in mid air, and a falling
//! brick only starts looking for something to land on once it is within one
//! unit of the top of the pile. The test is not geometric either: each brick
//! covers two grid squares, worked out from its position and which of four
//! quarter turns it has, and a landing is any brick already at rest sharing one
//! of those squares at the right height. When it lands its height is snapped to
//! an exact multiple rather than left where it stopped, because otherwise the
//! rounding error accumulates until the pile stops fitting together.
//!
//! The camera does not follow the action, it drifts after it: the eye line
//! chases the height of the pile at a hundredth of the remaining distance per
//! frame, which never arrives and never jumps. Once the pile is high enough
//! that the baseplate has left the frame, the baseplate stops being drawn.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::shapes::unit_sphere;
use crate::runtime::tube::tube;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
};

const BLOCK_HEIGHT: f32 = 1.49;
const BLOCK_WIDTH: f32 = 2.0;
const TOLERANCE: f32 = 0.1;
const CYL_SIZE: f32 = 0.333_334;
const UDD_SIZE: f32 = 0.4;
/// The thickness of the carpet.
const SINGLE_THICK: f32 = 0.29;

fn get_height(a: f32) -> f32 {
    a * BLOCK_HEIGHT
}

fn get_location(a: f32) -> f32 {
    a * BLOCK_WIDTH
}

/// The eight colours a brick can be, in upstream's order. Only the first
/// `maxColors` of them are used, and the default of seven leaves out the black.
const COLORS: [[f32; 4]; 8] = [
    [1.0, 0.0, 0.0, 1.0],
    [0.0, 1.0, 0.0, 1.0],
    [0.0, 0.0, 1.0, 1.0],
    [0.95, 0.95, 0.95, 1.0],
    [1.0, 0.5, 0.0, 1.0],
    [1.0, 1.0, 0.0, 1.0],
    [0.5, 0.5, 0.5, 1.0],
    [0.05, 0.05, 0.05, 1.0],
];

const CARPET_COLOR: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

struct Block {
    color: usize,
    /// Which quarter turn it has: 0 is S-N, 1 W-E, 2 N-S, 3 E-W, held as the
    /// angle rather than the index because everything reads it as one.
    rotation: i32,
    height: f32,
    x: f32,
    y: f32,
    falling: bool,
}

struct TopBlock {
    trackball: Trackball,
    rotate_speed: f32,
    drop_speed: f32,
    max_falling: usize,
    resolution: i32,
    highest: f32,
    highest_falling: f32,
    eye_line: f32,
    eye: [f32; 3],
    cam: [f32; 3],
    carpet_width: i32,
    carpet_length: i32,
    /// Whether the camera is chasing one particular brick down.
    follow_mode: bool,
    follow_index: Option<usize>,
    follow_radius: f32,
    follow_angle: f32,
    plusheight: i32,
    rotation: f32,
    blocks: Vec<Block>,
    /// Which display list holds the brick and which the baseplate.
    block_list: u32,
    carpet_list: u32,

    wireframe: bool,
    do_rotate: bool,
    follow: bool,
    draw_carpet: bool,
    draw_nipples: bool,
    max_colors: usize,
    spawn: i32,
}

/// `quadrantCorrection`: each quarter of the circle has to be adjusted for.
fn quadrant_correction(angle: f32, cx: i32, cy: i32, x: i32, y: i32) -> f32 {
    // Upstream spells this as four quadrants tested in order, and the order
    // matters only on the axis itself: at y == cy the half-turn goes one way
    // to the right of centre and the other way to the left.
    let doubled = angle + (90.0 - (angle - 90.0) * 2.0);
    let plain = angle + 90.0;
    let angle = if x >= cx {
        if y >= cy { doubled } else { plain }
    } else if y <= cy {
        plain
    } else {
        doubled
    };
    angle - 180.0
}

impl TopBlock {
    /// `polygonPlane`: one face of the brick's body, from the shared corner and
    /// normal tables.
    fn polygon_plane(&self, g: &mut Gl, a: usize, b: usize, c: usize, d: usize, i: usize) {
        const NORMALS: [[f32; 3]; 5] = [
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
        ];
        const VERTICES: [[f32; 3]; 8] = [
            [-0.49, -2.97, -0.99],
            [0.99, -2.97, -0.99],
            [0.99, 0.99, -0.99],
            [-0.49, 0.99, -0.99],
            [-0.49, -2.97, 0.99],
            [0.99, -2.97, 0.99],
            [0.99, 0.99, 0.99],
            [-0.49, 0.99, 0.99],
        ];
        g.glx.begin(if self.wireframe {
            Shape::LineLoop
        } else {
            Shape::Polygon
        });
        let n = NORMALS[i];
        g.glx.normal3f(n[0], n[1], n[2]);
        for k in [a, b, c, d] {
            let v = VERTICES[k];
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();
    }

    /// `buildBlock`: the body, the eight studs on top and the three tubes
    /// underneath that grip the studs of the brick below.
    fn build_block(&self, g: &mut Gl) {
        let wire = self.wireframe;
        g.glx.push_matrix();
        g.glx.rotate(90.0, 0.0, 1.0, 0.0);

        self.polygon_plane(g, 0, 3, 2, 1, 0);
        self.polygon_plane(g, 2, 3, 7, 6, 1);
        self.polygon_plane(g, 1, 2, 6, 5, 2);
        self.polygon_plane(g, 4, 5, 6, 7, 3);
        self.polygon_plane(g, 0, 1, 5, 4, 4);

        if self.draw_nipples {
            // Aim the pointer ready for the cylinder, then walk the eight
            // studs in two rows of four.
            g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            g.glx.translate(0.5, 0.5, 0.99);
            for c in 0..2 {
                for _ in 0..4 {
                    tube(
                        &mut g.glx,
                        [0.0, 0.0, 0.0],
                        [0.0, 0.0, 0.25],
                        CYL_SIZE,
                        0.0,
                        self.resolution,
                        true,
                        true,
                        wire,
                    );
                    g.glx.translate(0.0, if c == 0 { -1.0 } else { 1.0 }, 0.0);
                }
                g.glx.translate(-1.0, 1.0, 0.0);
            }

            // Udders: three cylinders on the underside.
            g.glx.translate(1.5, -2.5, -1.5);
            if !wire {
                for _ in 0..3 {
                    tube(
                        &mut g.glx,
                        [0.0, 0.0, 0.1],
                        [0.0, 0.0, 1.4],
                        UDD_SIZE,
                        0.0,
                        self.resolution,
                        true,
                        true,
                        wire,
                    );
                    g.glx.translate(0.0, -1.0, 0.0);
                }
            }
        }
        g.glx.pop_matrix();
    }

    /// `buildBlobBlock`: two spheres instead of a brick, which is what the
    /// author used to check the lighting and left in as a knob.
    fn build_blob_block(&self, g: &mut Gl) {
        let wire = self.wireframe;
        g.glx.push_matrix();
        g.glx.scale(1.4, 1.4, 1.4);
        unit_sphere(&mut g.glx, self.resolution / 2, self.resolution, wire);
        g.glx.pop_matrix();
        g.glx.translate(0.0, -2.0, 0.0);
        g.glx.scale(1.4, 1.4, 1.4);
        unit_sphere(&mut g.glx, self.resolution / 2, self.resolution, wire);
    }

    /// `buildCarpet`: the baseplate, a slab with four visible edges and a grid
    /// of studs.
    fn build_carpet(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let (x, y) = (self.carpet_width as f32, self.carpet_length as f32);
        g.glx.push_matrix();

        g.glx
            .begin(if wire { Shape::LineLoop } else { Shape::Quads });
        g.glx.normal3f(0.0, 0.0, -1.0);
        for v in [[0.0, 0.0, 0.0], [x, 0.0, 0.0], [x, y, 0.0], [0.0, y, 0.0]] {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        if !wire {
            let sides: [([f32; 3], [[f32; 3]; 4]); 4] = [
                (
                    [0.0, -1.0, 0.0],
                    [
                        [0.0, 0.0, 0.0],
                        [x, 0.0, 0.0],
                        [x, 0.0, SINGLE_THICK],
                        [0.0, 0.0, SINGLE_THICK],
                    ],
                ),
                (
                    [-1.0, 0.0, 0.0],
                    [
                        [0.0, 0.0, 0.0],
                        [0.0, y, 0.0],
                        [0.0, y, SINGLE_THICK],
                        [0.0, 0.0, SINGLE_THICK],
                    ],
                ),
                (
                    [1.0, 0.0, 0.0],
                    [
                        [x, 0.0, 0.0],
                        [x, y, 0.0],
                        [x, y, SINGLE_THICK],
                        [x, 0.0, SINGLE_THICK],
                    ],
                ),
                (
                    [0.0, 1.0, 0.0],
                    [
                        [0.0, y, 0.0],
                        [x, y, 0.0],
                        [x, y, SINGLE_THICK],
                        [0.0, y, SINGLE_THICK],
                    ],
                ),
            ];
            for (n, vs) in sides {
                g.glx.normal3f(n[0], n[1], n[2]);
                for v in vs {
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
            }
        }
        g.glx.end();

        if self.draw_nipples {
            g.glx.translate(0.5, 0.5, -0.25);
            for _ in 0..self.carpet_width {
                g.glx.push_matrix();
                for _ in 0..self.carpet_length {
                    tube(
                        &mut g.glx,
                        [0.0, 0.0, -0.1],
                        [0.0, 0.0, 0.26],
                        CYL_SIZE,
                        0.0,
                        self.resolution,
                        true,
                        true,
                        wire,
                    );
                    g.glx.translate(0.0, 1.0, 0.0);
                }
                g.glx.pop_matrix();
                g.glx.translate(1.0, 0.0, 0.0);
            }
        }
        g.glx.pop_matrix();
    }

    /// The two grid squares a brick covers, which is all the collision test
    /// looks at.
    fn footprint(b: &Block) -> ([f32; 2], [f32; 2]) {
        let first = [b.x, b.y];
        let second = match b.rotation {
            0 => [b.x, b.y - 2.0],
            90 => [b.x + 2.0, b.y],
            180 => [b.x, b.y + 2.0],
            _ => [b.x - 2.0, b.y],
        };
        (first, second)
    }

    /// `generateNewBlock`: one chance in `spawn` per frame, and only while the
    /// stack of falling bricks has room above the pile.
    fn generate_new_block(&mut self) {
        if random() as i32 % self.spawn != 1 {
            return;
        }
        if self.highest_falling
            >= get_height((self.plusheight as f32 - BLOCK_HEIGHT) + self.highest)
        {
            return;
        }

        let rotation = (random() % 4) as i32 * 90;
        let (start_off_x, end_off_x, start_off_y, end_off_y) = match rotation {
            0 | 180 => (1.0, 0, 3.0, -1),
            90 => (1.0, -1, 1.0, 0),
            _ => (5.0, -1, 1.0, 0),
        };

        let half = self.carpet_length / 2;
        let rx = (random() as i32).rem_euclid((half + end_off_x).max(1));
        let ry = (random() as i32).rem_euclid((half + end_off_y).max(1));

        // At the cap the two oldest bricks go and one new one arrives, which
        // is upstream's list surgery: the head is dropped and the one after it
        // is reused for the newcomer.
        if self.blocks.len() >= self.max_falling {
            let n = 2.min(self.blocks.len());
            self.blocks.drain(0..n);
        }

        self.blocks.push(Block {
            color: (random() as usize) % self.max_colors,
            rotation,
            x: (start_off_x - half as f32) + get_location(rx as f32),
            y: (start_off_y - half as f32) + get_location(ry as f32),
            height: get_height(self.plusheight as f32 + self.highest),
            falling: true,
        });
    }

    /// `followBlock`: chase whichever brick is being watched down, in the
    /// carpet's own coordinates rather than the rotated scene's.
    fn follow_block(&mut self, g: &mut Gl) {
        if let Some(i) = self.follow_index
            && self.follow_mode
            && i < self.blocks.len()
        {
            if self.highest > self.eye_line {
                self.eye_line += (self.highest - self.eye_line) / 100.0;
            }
            let bh = self.blocks[i].height;
            if bh > self.eye[2] {
                self.eye[2] += (bh - self.eye[2]) / 100.0;
            }
            if bh < self.eye[2] {
                self.eye[2] -= (self.eye[2] - bh) / 100.0;
            }

            if self.follow_radius == 0.0 {
                let (x_len, y_len) = (self.blocks[i].x, self.blocks[i].y);
                self.follow_radius = (x_len * x_len + y_len * y_len).sqrt();
                self.follow_angle =
                    (180.0 / std::f32::consts::PI) * (x_len / self.follow_radius).asin();
                self.follow_angle = quadrant_correction(
                    self.follow_angle,
                    0,
                    0,
                    self.blocks[i].x as i32,
                    self.blocks[i].y as i32,
                );
            }
            let rangle = (self.follow_angle + self.rotation) * std::f32::consts::PI / 180.0;
            let x_target = rangle.cos() * self.follow_radius;
            let y_target = rangle.sin() * self.follow_radius;
            if self.follow_angle > 360.0 {
                self.follow_angle -= 360.0;
            }

            self.eye[0] += (x_target - self.eye[0]) / 100.0;
            self.eye[1] += (y_target - self.eye[1]) / 100.0;

            if !self.blocks[i].falling {
                self.follow_mode = false;
                self.follow_radius = 0.0;
            }
        }

        g.glx.look_at(
            [self.cam[0], self.cam[1], self.cam[2] - self.eye_line],
            [self.eye[0], self.eye[1], -self.eye[2]],
            [-1.0, 0.0, 0.0],
        );
    }

    /// Advance one falling brick, and land it if it has met the pile.
    fn settle(&mut self, i: usize) {
        if self.blocks[i].height > self.highest_falling {
            self.highest_falling = self.blocks[i].height;
        }
        // All blocks fall at the same rate to avoid mid air collisions.
        self.blocks[i].height -= self.drop_speed;
        if self.blocks[i].height <= 0.0 {
            self.blocks[i].falling = false;
            if self.highest == 0.0 {
                self.highest += BLOCK_HEIGHT;
            }
            return;
        }
        if self.blocks[i].height > self.highest + 1.0 {
            return;
        }

        let (c1, c2) = Self::footprint(&self.blocks[i]);
        for j in 0..self.blocks.len() {
            if self.blocks[j].falling || !self.blocks[i].falling {
                continue;
            }
            let (n1, n2) = Self::footprint(&self.blocks[j]);
            let overlap = c1 == n1 || c1 == n2 || c2 == n2 || c2 == n1;
            if !overlap {
                continue;
            }
            let top = self.blocks[j].height + BLOCK_HEIGHT;
            if (self.blocks[i].height - top).abs() > TOLERANCE {
                continue;
            }
            self.blocks[i].falling = false;
            // Snap to the exact height, or small errors build up until the
            // model stops fitting together.
            self.blocks[i].height = top;
            if (self.blocks[i].height - self.highest).abs() <= TOLERANCE + BLOCK_HEIGHT {
                self.highest += BLOCK_HEIGHT;
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wireframe = g.res.bool("wireframe");
    let size = g.res.int("size").clamp(1, 10);
    let spawn = g.res.int("spawn").clamp(4, 1000);

    let mut rotate_speed = g.res.float("rotateSpeed") as f32;
    rotate_speed = rotate_speed.clamp(1.0, 1000.0) / 100.0;

    let resolution = g.res.int("resolution").clamp(4, 20) * 2;
    let max_colors = g.res.int("maxColors").clamp(1, 8) as usize;

    // 10 and up produces blocks that can pass through each other.
    let drop_speed = g.res.float("dropSpeed").clamp(1.0, 9.0) as f32;
    let drop_speed = BLOCK_HEIGHT / (80.0 / drop_speed);

    let follow = g.res.bool("follow");
    let override_cam = g.res.bool("override");

    let mut this = TopBlock {
        trackball: Trackball::new(),
        rotate_speed,
        drop_speed,
        max_falling: (g.res.int("maxFalling") * size).max(1) as usize,
        resolution,
        highest: 0.0,
        highest_falling: 0.0,
        eye_line: 0.0,
        eye: [0.0, 0.0, 0.0],
        cam: [
            g.res.float("camX") as f32,
            g.res.float("camY") as f32,
            g.res.float("camZ") as f32,
        ],
        carpet_width: 8 * size,
        carpet_length: 8 * size,
        follow_mode: false,
        follow_index: None,
        follow_radius: 0.0,
        follow_angle: 0.0,
        plusheight: 30,
        rotation: 0.0,
        blocks: Vec::new(),
        block_list: 0,
        carpet_list: 0,
        wireframe,
        do_rotate: g.res.bool("rotate"),
        follow,
        draw_carpet: g.res.bool("carpet"),
        draw_nipples: g.res.bool("nipples"),
        max_colors,
        spawn,
    };

    if follow {
        this.plusheight = 100;
        this.cam[2] -= 60.0;
    } else {
        this.rotation = (random() % 360) as f32;
        this.eye[1] = 10.0;
        this.plusheight = 30;
    }

    // Tunnel mode: put the camera on the floor looking up the stack.
    if override_cam {
        this.plusheight = 100;
        this.draw_carpet = false;
        this.cam = [0.0, 1.0, 0.0];
        this.eye = [-1.0, 20.0, 0.0];
    }

    this.block_list = g.glx.gen_lists(1);
    g.glx.new_list(this.block_list);
    if g.res.bool("blob") {
        this.build_blob_block(g);
    } else {
        this.build_block(g);
    }
    g.glx.end_list();

    this.carpet_list = g.glx.gen_lists(1);
    g.glx.new_list(this.carpet_list);
    this.build_carpet(g);
    g.glx.end_list();

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for TopBlock {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let (mut height, mut y) = (height, 0);
        let mut h = height as f32 / width as f32;
        if width > height * 5 {
            // Tiny window: show the middle.
            height = (width as f32 * 1.5) as i32;
            y = -(height as f32 * 0.2) as i32;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(60.0, 1.0 / h, 1.0, 1000.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        // Upstream's debugging keys, which nudge the camera about.
        if let XEvent::KeyPress { key } = event {
            let d: &mut f32 = match key {
                'a' | 'z' => &mut self.eye[0],
                's' | 'x' => &mut self.eye[1],
                'd' | 'c' => &mut self.eye[2],
                'f' | 'v' => &mut self.cam[0],
                'g' | 'b' => &mut self.cam[1],
                'h' | 'n' => &mut self.cam[2],
                'r' => {
                    self.rotation += 1.0;
                    return true;
                }
                _ => return false,
            };
            *d += match key {
                'a' | 'x' | 'd' | 'f' | 'g' | 'h' => 1.0,
                _ => -1.0,
            };
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.generate_new_block();

        let down = self.trackball.button_down();
        if self.do_rotate && !down {
            self.rotation += self.rotate_speed;
        }
        if self.rotation >= 360.0 {
            self.rotation -= 360.0;
        }

        g.glx.clear();
        g.glx.depth_test(true);
        // All objects exhibit a reverse side.
        g.glx.cull_face(false);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 10.0, 10.0, 1.0, 0.0);
            g.glx.light_ambient(0, [0.1, 0.1, 0.1, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
        }

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        if !self.follow {
            // A smooth camera transition: the eye line never quite catches the
            // top of the pile.
            if self.highest > self.eye_line {
                self.eye_line += (self.highest - self.eye_line) / 100.0;
            }
            g.glx.look_at(
                [self.cam[0], self.cam[1] + self.eye_line, self.cam[2]],
                [self.eye[0], self.eye[1] + self.eye_line, self.eye[2]],
                [0.0, 1.0, 0.0],
            );
            g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        } else {
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            self.follow_block(g);
        }

        // Rotate the scene around a point that's a little higher up.
        g.glx.translate(0.0, 0.0, -5.0);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.translate(0.0, 0.0, 5.0);
        g.glx.rotate(self.rotation, 0.0, 0.0, 1.0);

        if self.draw_carpet {
            let (cw, cl) = (self.carpet_width as f32, self.carpet_length as f32);
            // The material lives outside the list: the recorder keeps geometry
            // in a display list and state alongside it.
            if self.wireframe {
                g.glx
                    .color3f(CARPET_COLOR[0], CARPET_COLOR[1], CARPET_COLOR[2]);
            } else {
                g.glx.material_ambient_diffuse(CARPET_COLOR);
            }
            g.glx.translate(-cw / 2.0, -cl / 2.0, 0.0);
            g.glx.call_list(self.carpet_list);
            g.glx.translate(cw / 2.0, cl / 2.0, 0.0);
            g.glx.translate(0.0, 0.0, -0.55);
        }

        self.highest_falling = 0.0;
        for i in 0..self.blocks.len() {
            g.glx.push_matrix();
            let color = COLORS[self.blocks[i].color];
            if self.wireframe {
                g.glx.color3f(color[0], color[1], color[2]);
            } else {
                g.glx.material_ambient_diffuse(color);
            }

            if self.blocks[i].falling {
                self.settle(i);
            }

            let b = &self.blocks[i];
            g.glx.translate(b.x, b.y, -b.height);
            g.glx.rotate(b.rotation as f32, 0.0, 0.0, 1.0);
            if !self.follow_mode && i + 1 == self.blocks.len() {
                self.follow_index = Some(i);
                self.follow_mode = true;
            }
            g.glx.call_list(self.block_list);
            g.glx.pop_matrix();
        }

        if self.highest > 5.0 * self.max_falling as f32 {
            self.draw_carpet = false;
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        10000",
    "*count:        30",
    "*showFPS:      False",
    "*wireframe:    False",
    "*override:     False",
    "*rotate:       True",
    "*follow:       False",
    "*carpet:       True",
    "*blob:         False",
    "*nipples:      True",
    "*rotateSpeed:  10",
    "*maxFalling:   75",
    "*maxColors:    7",
    "*size:         2",
    "*spawn:        50",
    "*resolution:   8",
    "*camX:         1",
    "*camY:         20",
    "*camZ:         25",
    "*dropSpeed:    4",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("dropSpeed", "Drop speed", 1.0, 9.0, 1.0, 0, "4"),
    Opt::slider("size", "Carpet size", 1.0, 10.0, 1.0, 0, "2"),
    // Upstream misspells this label; the panel is ours, so it is corrected here.
    Opt::slider("spawn", "Spawn likelihood", 4.0, 1000.0, 1.0, 0, "50").inverted(),
    Opt::slider("resolution", "Polygon count", 4.0, 20.0, 1.0, 0, "8"),
    // The XML offers up to 32 colours; the C clamps to the eight it has.
    Opt::slider("maxColors", "Colors", 1.0, 8.0, 1.0, 0, "7"),
    Opt::slider("rotateSpeed", "Rotation", 1.0, 1000.0, 1.0, 0, "10"),
    Opt::boolean("rotate", "Rotate", "true"),
    Opt::boolean("follow", "Follow", "false"),
    Opt::boolean("blob", "Blob mode", "false"),
    Opt::boolean("override", "Tunnel mode", "false"),
    Opt::boolean("carpet", "Carpet", "true"),
    Opt::boolean("nipples", "Nipples", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "topblock",
    label: "Top Block",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "rednuht",
        year: "2006",
        video: Some("https://www.youtube.com/watch?v=zj0FHFJgQJ8"),
        blurb: "Creates a 3D world with dropping blocks that build up and up.",
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

    #[test]
    fn bricks_arrive_from_above_and_come_to_rest() {
        let mut r = start(StartArgs::new(640, 480, "spawn=4&size=1", 20260811));
        for _ in 0..2000 {
            r.step();
        }
        let f = r.frame();
        assert!(!f.batches.is_empty());
        // The pile has to have got somewhere off the baseplate.
        let low = f
            .batches
            .iter()
            .map(|b| b.modelview.0[14])
            .fold(f32::MAX, f32::min);
        let high = f
            .batches
            .iter()
            .map(|b| b.modelview.0[14])
            .fold(f32::MIN, f32::max);
        assert!(
            high - low > 10.0,
            "everything is at one height, {low}..{high}"
        );
    }

    /// A saver with nothing in it, for testing the pile logic on its own.
    fn bare() -> TopBlock {
        TopBlock {
            trackball: Trackball::new(),
            rotate_speed: 0.1,
            drop_speed: 0.1,
            max_falling: 100,
            resolution: 8,
            highest: 0.0,
            highest_falling: 0.0,
            eye_line: 0.0,
            eye: [0.0; 3],
            cam: [1.0, 20.0, 25.0],
            carpet_width: 8,
            carpet_length: 8,
            follow_mode: false,
            follow_index: None,
            follow_radius: 0.0,
            follow_angle: 0.0,
            plusheight: 30,
            rotation: 0.0,
            blocks: Vec::new(),
            block_list: 0,
            carpet_list: 0,
            wireframe: false,
            do_rotate: true,
            follow: false,
            draw_carpet: true,
            draw_nipples: true,
            max_colors: 7,
            spawn: 50,
        }
    }

    fn brick(x: f32, y: f32, rotation: i32, height: f32, falling: bool) -> Block {
        Block {
            color: 0,
            rotation,
            height,
            x,
            y,
            falling,
        }
    }

    #[test]
    fn a_landed_brick_sits_at_an_exact_multiple_of_the_brick_height() {
        // Upstream snaps the height on landing rather than leaving it where it
        // stopped, because the rounding error otherwise accumulates until the
        // pile stops fitting together.
        let mut tb = bare();
        // The first brick to land does so at zero and takes the top of the
        // pile with it.
        tb.highest = BLOCK_HEIGHT;
        tb.blocks.push(brick(0.0, 0.0, 0, 0.0, false));
        // A second on the same square, falling, a hair above where it lands.
        tb.blocks
            .push(brick(0.0, 0.0, 0, BLOCK_HEIGHT + 0.11, true));
        tb.drop_speed = 0.02;
        for _ in 0..20 {
            if !tb.blocks[1].falling {
                break;
            }
            tb.settle(1);
        }
        assert!(!tb.blocks[1].falling, "it never landed");
        assert_eq!(tb.blocks[1].height, BLOCK_HEIGHT);
        assert_eq!(tb.highest, 2.0 * BLOCK_HEIGHT);
    }

    #[test]
    fn a_brick_only_lands_on_one_it_actually_overlaps() {
        // The test is not geometric: a brick covers two grid squares worked
        // out from its turn, and a landing needs one of them to match.
        let mut tb = bare();
        tb.highest = BLOCK_HEIGHT;
        // At rest on the far side of the plate.
        tb.blocks.push(brick(6.0, 6.0, 0, BLOCK_HEIGHT, false));
        tb.blocks.push(brick(0.0, 0.0, 0, 2.0 * BLOCK_HEIGHT, true));
        tb.drop_speed = 0.02;
        for _ in 0..20 {
            tb.settle(1);
        }
        assert!(tb.blocks[1].falling, "it landed on thin air");

        // The two squares a brick covers depend on which way round it is.
        let a = brick(0.0, 0.0, 0, 0.0, false);
        let b = brick(0.0, 0.0, 90, 0.0, false);
        assert_eq!(TopBlock::footprint(&a), ([0.0, 0.0], [0.0, -2.0]));
        assert_eq!(TopBlock::footprint(&b), ([0.0, 0.0], [2.0, 0.0]));
    }

    #[test]
    fn the_baseplate_is_drawn_underneath_and_can_be_turned_off() {
        // The baseplate is the only thing in flat green. Upstream also drops
        // it once the pile has outgrown it, which takes some five hundred
        // layers and is not worth a test.
        let carpet_drawn = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            r.frame()
                .batches
                .iter()
                .any(|b| b.material.ambient_diffuse == CARPET_COLOR)
        };
        assert!(carpet_drawn("size=1"), "the baseplate was never there");
        assert!(!carpet_drawn("size=1&carpet=false"), "it would not go away");
    }
}
