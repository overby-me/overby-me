//! Port of `hacks/glx/antmaze.c`.
//!
//! ```text
//! antmaze --- ant maze
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
//! Copyright 2005 Blair Tennessy
//! ```
//!
//! Ants walk a maze, seen three ways at once: a camera circling the board, an
//! overhead view in the corner, and one ant turning on the spot beside them.
//!
//! The maze is not solved. The board starts solid and the path each ant will
//! take is cut out of it, a step at a time towards the far corner, so the ants
//! are walking corridors that were made for them. When the last of a dozen has
//! gone through, the board fades out, another is cut, and it fades back in.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::shapes::unit_sphere;
use crate::runtime::tube::cone;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random, random_below,
};
use std::f64::consts::PI;

const ANTCOUNT: usize = 5;
const EPSILON: f64 = 0.01;
const BOARDSIZE: usize = 10;
const BOARDCOUNT: usize = 2;
const PARTS: usize = 20;
const CHECK_SIZE: usize = 64;
const MAX_MAGNIFICATION: i32 = 10;

const MATERIAL_GRAY: [f32; 4] = [0.2, 0.2, 0.2, 1.0];
const MATERIAL_GRAY_B: [f32; 4] = [0.1, 0.1, 0.1, 0.5];
// Upstream also carries a rotator it never reads and two greys it only uses in
// its monochrome branch, which no longer arises.
const MATERIAL_GRAY6: [f32; 4] = [0.6, 0.6, 0.6, 1.0];
const MATERIAL_RED: [f32; 4] = [0.6, 0.0, 0.0, 1.0];
const MATERIAL_GRAY35: [f32; 4] = [0.30, 0.30, 0.30, 1.0];
const MATERIAL_GRAY4: [f32; 4] = [0.40, 0.40, 0.40, 1.0];
const MATERIAL_ORANGE: [f32; 4] = [1.0, 0.69, 0.00, 1.0];
const MATERIAL_GREEN: [f32; 4] = [0.1, 0.4, 0.2, 1.0];

/// The colours an ant can be repainted when it comes round again.
const MATERIALS: [[f32; 4]; 4] = [
    MATERIAL_RED,
    MATERIAL_GRAY35,
    MATERIAL_GRAY4,
    MATERIAL_ORANGE,
];

const DIFFUSE: [f32; 4] = [0.8, 0.8, 0.8, 1.0];
const AMBIENT: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

/// A filled sphere, as `mySphere`.
fn my_sphere(g: &mut Gl, radius: f32, stacks: i32, slices: i32) {
    g.glx.push_matrix();
    g.glx.scale(radius, radius, radius);
    g.glx.rotate(90.0, 1.0, 0.0, 0.0);
    unit_sphere(&mut g.glx, stacks, slices, false);
    g.glx.pop_matrix();
}

/// A filled cone, as `myCone`.
fn my_cone(g: &mut Gl, radius: f32) {
    cone(
        &mut g.glx,
        [0.0, 0.0, 0.0],
        [0.0, 0.0, radius * 2.0],
        radius,
        0.0,
        8,
        true,
        true,
        false,
    );
}

/// Which body parts a call to `draw_ant` is to draw: whether the spheres are
/// textured, and whether the cones (the mandibles) are drawn at all.
#[derive(Clone, Copy)]
struct AntStyle {
    textured: bool,
    mandibles: bool,
    /// A shadow is flat and gets no highlights on its feet.
    shadow: bool,
}

struct Antmaze {
    step: f64,
    trackball: Trackball,
    focus: usize,
    currentboard: usize,

    antdirection: [f64; ANTCOUNT],
    antposition: [[f64; 3]; ANTCOUNT],
    anton: [bool; ANTCOUNT],
    antvelocity: [f64; ANTCOUNT],
    antsize: [f64; ANTCOUNT],
    antmaterial: [[f32; 4]; ANTCOUNT],
    board: [[[bool; BOARDSIZE]; BOARDSIZE]; BOARDCOUNT],

    part: [usize; ANTCOUNT],
    antpath: [[[f64; 2]; PARTS]; ANTCOUNT],
    antpathlength: [usize; ANTCOUNT],

    checktexture: u32,
    brushedtexture: u32,
    elevator: f64,

    ant_step: f64,
    first_ant_step: f64,
    started: bool,
    introduced: i32,
    entroducing: i32,

    fadeout: f64,
    fadeoutspeed: f64,
    mag: i32,

    win_w: i32,
    win_h: i32,
}

fn near(a: [f64; 3], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.5 && (a[1] - b[1]).abs() < 0.5
}

fn sign(d: f64) -> f64 {
    if d < 0.0 { -1.0 } else { 1.0 }
}

impl Antmaze {
    /// `makeCheckImage`: a chequerboard with a little noise in each square.
    fn make_check_image(g: &mut Gl) -> u32 {
        let mut data = Vec::with_capacity(CHECK_SIZE * CHECK_SIZE * 4);
        for i in 0..CHECK_SIZE {
            for j in 0..CHECK_SIZE {
                let c = if ((i & 0x8) == 0) ^ ((j & 0x8) != 0) {
                    102 + random_below(32)
                } else {
                    153 + random_below(32)
                } as u8;
                data.extend_from_slice(&[c, c, c, 0xff]);
            }
        }
        let t = g.glx.gen_texture();
        g.glx.bind_texture(t);
        g.glx.tex_nearest(false);
        g.glx.tex_clamp(false);
        g.glx
            .tex_image_2d(CHECK_SIZE as i32, CHECK_SIZE as i32, data);
        t
    }

    /// `makeBrushedImage`: a fine cross-hatch, for the ants' shells.
    fn make_brushed_image(g: &mut Gl) -> u32 {
        let mut data = Vec::with_capacity(CHECK_SIZE * CHECK_SIZE * 4);
        for i in 0..CHECK_SIZE {
            for j in 0..CHECK_SIZE {
                let s = (2.0 * i as f64 / PI).sin() * (2.0 * j as f64 / PI).sin();
                let c = (102.0 + 102.0 * s.abs()) as i32 + random_below(51);
                let c = c.clamp(0, 255) as u8;
                data.extend_from_slice(&[c, c, c, 0xff]);
            }
        }
        let t = g.glx.gen_texture();
        g.glx.bind_texture(t);
        g.glx.tex_nearest(false);
        g.glx.tex_clamp(false);
        g.glx
            .tex_image_2d(CHECK_SIZE as i32, CHECK_SIZE as i32, data);
        t
    }

    /// `build_board`: a solid board with one corridor cut through it for each
    /// ant, wandering towards the far corner.
    fn build_board(&mut self, b: usize) {
        for row in &mut self.board[b] {
            for cell in row.iter_mut() {
                *cell = true;
            }
        }
        self.board[b][BOARDSIZE - 1][1] = false;
        self.board[b][0][BOARDSIZE - 2] = false;

        if self.currentboard != b {
            return;
        }
        for i in 0..ANTCOUNT {
            let mut sx = BOARDSIZE - 2;
            let mut sy = 1;
            let mut j = 0;
            loop {
                self.board[b][sx][sy] = false;
                self.antpath[i][j][0] = sy as f64 - 5.0;
                self.antpath[i][j][1] = sx as f64 - 5.0;
                // Either west or south, whichever is still open.
                let west_first = random() % 2 == 1;
                let moved = if west_first {
                    if sx > 1 {
                        sx -= 1;
                        true
                    } else if sy < BOARDSIZE - 2 {
                        sy += 1;
                        true
                    } else {
                        false
                    }
                } else if sy < BOARDSIZE - 2 {
                    sy += 1;
                    true
                } else if sx > 1 {
                    sx -= 1;
                    true
                } else {
                    false
                };
                if !moved {
                    break;
                }
                j += 1;
                if j >= PARTS - 1 {
                    break;
                }
            }
            j += 1;
            self.antpath[i][j][0] = BOARDSIZE as f64 - 7.0;
            self.antpath[i][j][1] = -7.0;
            self.antpathlength[i] = j;
        }
    }

    /// `draw_board`: the floor, and a box on every square that is still wall.
    /// A wall's side is only drawn where it meets an open square.
    fn draw_board(&self, g: &mut Gl) {
        let h = 0.5;
        let stf = 0.0625;
        g.glx.bind_texture(self.checktexture);
        g.glx.begin(Shape::Quads);
        for i in 0..BOARDSIZE {
            for j in 0..BOARDSIZE {
                let (x, z) = (i as f32, j as f32);
                if self.board[self.currentboard][j][i] {
                    g.glx.normal3f(0.0, 1.0, 0.0);
                    for (u, v, p) in [
                        (0.0 + stf, 0.0 + stf, [x - 0.5, h, z + 0.5]),
                        (1.0 + stf, 0.0 + stf, [x + 0.5, h, z + 0.5]),
                        (1.0 + stf, 1.0 + stf, [x + 0.5, h, z - 0.5]),
                        (0.0 + stf, 1.0 + stf, [x - 0.5, h, z - 0.5]),
                    ] {
                        g.glx.tex_coord2f(u, v);
                        g.glx.vertex3f(p[0], p[1], p[2]);
                    }

                    let open = |di: isize, dj: isize| {
                        let (ni, nj) = (i as isize + di, j as isize + dj);
                        if !(0..BOARDSIZE as isize).contains(&ni)
                            || !(0..BOARDSIZE as isize).contains(&nj)
                        {
                            return true;
                        }
                        !self.board[self.currentboard][nj as usize][ni as usize]
                    };

                    // South, north, east and west, each only where it shows.
                    let faces: [(bool, [f32; 3], [[f32; 3]; 4]); 4] = [
                        (
                            open(0, 1),
                            [0.0, 0.0, 1.0],
                            [
                                [x - 0.5, 0.0, z + 0.5],
                                [x + 0.5, 0.0, z + 0.5],
                                [x + 0.5, h, z + 0.5],
                                [x - 0.5, h, z + 0.5],
                            ],
                        ),
                        (
                            open(0, -1),
                            [0.0, 0.0, -1.0],
                            [
                                [x + 0.5, 0.0, z - 0.5],
                                [x - 0.5, 0.0, z - 0.5],
                                [x - 0.5, h, z - 0.5],
                                [x + 0.5, h, z - 0.5],
                            ],
                        ),
                        (
                            open(1, 0),
                            [1.0, 0.0, 0.0],
                            [
                                [x + 0.5, 0.0, z + 0.5],
                                [x + 0.5, 0.0, z - 0.5],
                                [x + 0.5, h, z - 0.5],
                                [x + 0.5, h, z + 0.5],
                            ],
                        ),
                        (
                            open(-1, 0),
                            [-1.0, 0.0, 0.0],
                            [
                                [x - 0.5, 0.0, z - 0.5],
                                [x - 0.5, 0.0, z + 0.5],
                                [x - 0.5, h, z + 0.5],
                                [x - 0.5, h, z - 0.5],
                            ],
                        ),
                    ];
                    for (show, n, quad) in faces {
                        if !show {
                            continue;
                        }
                        g.glx.normal3f(n[0], n[1], n[2]);
                        for (k, p) in quad.into_iter().enumerate() {
                            let (u, v) = match k {
                                0 => (0.0 + stf, 0.0 + stf),
                                1 => (1.0 + stf, 0.0 + stf),
                                2 => (1.0 + stf, h + stf),
                                _ => (0.0 + stf, h + stf),
                            };
                            g.glx.tex_coord2f(u, v);
                            g.glx.vertex3f(p[0], p[1], p[2]);
                        }
                    }
                } else {
                    let tx = 2.0;
                    g.glx.normal3f(0.0, 1.0, 0.0);
                    for (u, v, p) in [
                        (0.0, 0.0, [x - 0.5, 0.0, z + 0.5]),
                        (tx, 0.0, [x + 0.5, 0.0, z + 0.5]),
                        (tx, tx, [x + 0.5, 0.0, z - 0.5]),
                        (0.0, tx, [x - 0.5, 0.0, z - 0.5]),
                    ] {
                        g.glx.tex_coord2f(u, v);
                        g.glx.vertex3f(p[0], p[1], p[2]);
                    }
                }
            }
        }
        g.glx.end();
    }

    /// `draw_ant`: a body of three spheres, two mandibles, and legs and
    /// antennae drawn as lines with the lights off.
    fn draw_ant(&self, g: &mut Gl, material: [f32; 4], ant_step: f64, style: AntStyle) {
        let cos = |k: f64| (ant_step + k * 2.0 * PI / 3.0).cos() as f32;
        let sin = |k: f64| (ant_step + k * 2.0 * PI / 3.0).sin() as f32;
        let (cos1, cos2, cos3) = (cos(0.0), cos(1.0), cos(2.0));
        let (sin1, sin2, sin3) = (sin(0.0), sin(1.0), sin(2.0));

        g.glx.material_diffuse(material);
        let (stacks, slices) = if style.textured { (32, 16) } else { (16, 16) };

        g.glx.push_matrix();
        g.glx.scale(1.0, 1.3, 1.0);
        my_sphere(g, 0.18, stacks, slices);
        g.glx.scale(1.0, 1.0 / 1.3, 1.0);
        g.glx.translate(0.00, 0.30, 0.00);
        my_sphere(g, 0.2, stacks, slices);

        g.glx.translate(-0.05, 0.17, 0.05);
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(-25.0, 0.0, 1.0, 0.0);
        if style.mandibles {
            my_cone(g, 0.05);
        }
        g.glx.translate(0.00, 0.10, 0.00);
        if style.mandibles {
            my_cone(g, 0.05);
        }
        g.glx.rotate(25.0, 0.0, 1.0, 0.0);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);

        g.glx.scale(1.0, 1.3, 1.0);
        g.glx.translate(0.15, -0.65, 0.05);
        my_sphere(g, 0.25, stacks, slices);
        g.glx.scale(1.0, 1.0 / 1.3, 1.0);
        g.glx.pop_matrix();

        g.glx.lighting(false);
        g.glx.texturing(false);

        // The antennae.
        g.glx.begin(Shape::Lines);
        for z in [0.40, -0.40] {
            g.glx
                .color4f(material[0], material[1], material[2], material[3]);
            g.glx.vertex3f(0.00, 0.30, 0.00);
            g.glx.color4f(
                MATERIAL_GRAY[0],
                MATERIAL_GRAY[1],
                MATERIAL_GRAY[2],
                MATERIAL_GRAY[3],
            );
            g.glx.vertex3f(0.40, 0.70, z);
        }
        g.glx.end();

        if !style.shadow {
            g.glx.color4f(
                MATERIAL_RED[0],
                MATERIAL_RED[1],
                MATERIAL_RED[2],
                MATERIAL_RED[3],
            );
            g.glx.begin(Shape::Points);
            g.glx.vertex3f(0.40, 0.70, 0.40);
            g.glx.vertex3f(0.40, 0.70, -0.40);
            g.glx.end();
        }

        // Six legs: three a side, a third of a stride apart.
        let feet = [
            [-0.20 + 0.05 * cos1, 0.25 + 0.1 * sin1, 0.45],
            [-0.20 + 0.05 * cos2, 0.00 + 0.1 * sin2, 0.45],
            [-0.20 + 0.05 * cos3, -0.25 + 0.1 * sin3, 0.45],
            [-0.20 - 0.05 * sin1, 0.25 + 0.1 * cos1, -0.45],
            [-0.20 - 0.05 * sin2, 0.00 + 0.1 * cos2, -0.45],
            [-0.20 - 0.05 * sin3, -0.25 + 0.1 * cos3, -0.45],
        ];
        let knees = [
            [0.00, 0.05, 0.18, 0.35 + 0.05 * cos1, 0.15, 0.25],
            [0.00, 0.00, 0.18, 0.35 + 0.05 * cos2, 0.00, 0.25],
            [0.00, -0.05, 0.18, 0.35 + 0.05 * cos3, -0.15, 0.25],
            [0.00, 0.05, -0.18, 0.35 - 0.05 * sin1, 0.15, -0.25],
            [0.00, 0.00, -0.18, 0.35 - 0.05 * sin2, 0.00, -0.25],
            [0.00, -0.05, -0.18, 0.35 - 0.05 * sin3, -0.15, -0.25],
        ];
        for (k, foot) in feet.into_iter().enumerate() {
            g.glx.begin(Shape::LineStrip);
            g.glx
                .color4f(material[0], material[1], material[2], material[3]);
            g.glx.vertex3f(knees[k][0], knees[k][1], knees[k][2]);
            g.glx.vertex3f(knees[k][3], knees[k][4], knees[k][5]);
            g.glx.color4f(
                MATERIAL_GRAY[0],
                MATERIAL_GRAY[1],
                MATERIAL_GRAY[2],
                MATERIAL_GRAY[3],
            );
            g.glx.vertex3f(foot[0], foot[1], foot[2]);
            g.glx.end();
        }

        if !style.shadow {
            g.glx.color4f(
                MATERIAL_GRAY35[0],
                MATERIAL_GRAY35[1],
                MATERIAL_GRAY35[2],
                MATERIAL_GRAY35[3],
            );
            g.glx.begin(Shape::Points);
            for foot in feet {
                g.glx.vertex3f(foot[0], foot[1], foot[2]);
            }
            g.glx.end();
        }

        g.glx.lighting(true);
    }

    /// `draw_antmaze_strip`: the board and every ant on it, each with a flat
    /// shadow under it. Called once per panel, and it is what advances the
    /// ants' stride.
    fn draw_strip(&mut self, g: &mut Gl) {
        g.glx.light_enable(0, true);
        g.glx.light_enable(1, true);

        if self.elevator < 1.0 {
            g.glx.texturing(true);
            g.glx.material_diffuse(MATERIAL_GRAY6);
            g.glx.translate(
                -((BOARDSIZE - 1) as f32) / 2.0,
                0.0,
                -((BOARDSIZE - 1) as f32) / 2.0,
            );
            self.draw_board(g);
            g.glx
                .translate(BOARDSIZE as f32 / 2.0, 0.0, BOARDSIZE as f32 / 2.0);
            g.glx.texturing(false);
        }

        self.introduced -= 1;
        g.glx.translate(0.0, -0.1, 0.0);

        for i in 0..ANTCOUNT {
            if !self.anton[i] {
                continue;
            }
            let slow = i == 0 && self.part[i] == self.antpathlength[i];
            let step = if slow {
                self.first_ant_step
            } else {
                self.ant_step
            };
            let size = self.antsize[i] as f32;
            let pos = self.antposition[i];
            let dir = 180.0 + self.antdirection[i] * 180.0 / PI;

            // The shadow: the same ant, squashed flat and drawn dark.
            g.glx.push_matrix();
            g.glx.translate(0.0, 0.01, 0.0);
            g.glx.translate(pos[0] as f32, pos[2] as f32, pos[1] as f32);
            g.glx.scale(0.6, 0.01, 0.6);
            g.glx.rotate(dir as f32, 0.0, 1.0, 0.0);
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            g.glx.lighting(false);
            g.glx.blend(Blend::Alpha);
            g.glx.scale(size, size, size);
            self.draw_ant(
                g,
                MATERIAL_GRAY_B,
                step,
                AntStyle {
                    textured: false,
                    mandibles: true,
                    shadow: true,
                },
            );
            g.glx.pop_matrix();
            g.glx.blend(Blend::Off);
            g.glx.lighting(true);

            // And the ant itself.
            g.glx.push_matrix();
            g.glx.translate(0.0, 0.12, 0.0);
            g.glx.translate(pos[0] as f32, pos[2] as f32, pos[1] as f32);
            g.glx.rotate(dir as f32, 0.0, 1.0, 0.0);
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            g.glx.scale(0.6, 0.6, 0.6);
            g.glx.scale(size, size, size);
            if slow && self.elevator > 0.0 {
                g.glx.light_diffuse(0, DIFFUSE);
                g.glx.light_diffuse(1, DIFFUSE);
                self.draw_ant(
                    g,
                    self.antmaterial[i],
                    self.first_ant_step,
                    AntStyle {
                        textured: false,
                        mandibles: true,
                        shadow: true,
                    },
                );
            } else {
                g.glx.texturing(true);
                g.glx.bind_texture(self.brushedtexture);
                self.draw_ant(
                    g,
                    self.antmaterial[i],
                    self.ant_step,
                    AntStyle {
                        textured: true,
                        mandibles: true,
                        shadow: true,
                    },
                );
                g.glx.texturing(false);
            }
            g.glx.pop_matrix();
        }

        self.ant_step += 0.18;
        if self.ant_step > 5.0 * PI {
            self.started = true;
        }
    }

    /// `update_ants`: fade the board in and out, bring ants on, and walk each
    /// one towards the next corner of its path.
    fn update_ants(&mut self, g: &mut Gl) {
        let df = [
            (0.8 * self.fadeout) as f32,
            (0.8 * self.fadeout) as f32,
            (0.8 * self.fadeout) as f32,
            1.0,
        ];
        if self.fadeoutspeed < -0.00001 {
            if self.fadeout <= 0.0 {
                // Switch boards: cut a new one and fade it back in.
                self.currentboard = (self.currentboard + 1) % BOARDCOUNT;
                let b = self.currentboard;
                self.build_board(b);
                self.fadeoutspeed = 0.02;
            }
            self.fadeout += self.fadeoutspeed;
            g.glx.light_diffuse(0, df);
            g.glx.light_diffuse(1, df);
        }
        if self.fadeoutspeed > 0.0001 {
            self.fadeout += self.fadeoutspeed;
            if self.fadeout >= 1.0 {
                self.fadeout = 1.0;
                self.fadeoutspeed = 0.0;
                self.entroducing = 12;
            }
            g.glx.light_diffuse(0, df);
            g.glx.light_diffuse(1, df);
        }

        for i in 0..ANTCOUNT {
            if !self.anton[i] && self.elevator < 1.0 {
                if self.entroducing > 0 && self.introduced <= 0 && random().is_multiple_of(100) {
                    self.anton[i] = true;
                    self.part[i] = 0;
                    self.antsize[i] = 0.0;
                    self.antposition[i][0] = -4.0;
                    self.antposition[i][1] = 5.0;
                    self.antdirection[i] = PI / 2.0;
                    self.introduced = 300;
                    self.entroducing -= 1;
                }
                continue;
            }

            // Growing on, and shrinking away again at the end.
            if self.part[i] == 0 && self.antsize[i] < 1.0 {
                self.antsize[i] += 0.02;
                continue;
            }
            if self.part[i] > self.antpathlength[i] && self.antsize[i] > 0.0 {
                self.antsize[i] -= 0.02;
                self.antvelocity[i] = (self.antvelocity[i] - 0.02).max(0.0);
                continue;
            }
            if self.part[i] > self.antpathlength[i] && self.antsize[i] <= 0.0 {
                self.antvelocity[i] = 0.02;
                self.antmaterial[i] = MATERIALS[random() as usize % MATERIALS.len()];
                self.antdirection[i] = PI / 2.0;
                self.part[i] = 0;
                self.antsize[i] = 0.0;
                self.anton[i] = false;
                self.antposition[i][0] = -4.0;
                self.antposition[i][1] = 5.0;

                // When the last of them has gone, fade the board out.
                if self.entroducing <= 0 && !self.anton.iter().any(|&on| on) {
                    self.fadeoutspeed = -0.02;
                }
            }

            if near(
                self.antposition[i],
                self.antpath[i][self.part[i].min(PARTS - 1)],
            ) {
                self.part[i] += 1;
            } else {
                // Turn towards the next corner, a couple of degrees at a time.
                let goal = self.antpath[i][self.part[i].min(PARTS - 1)];
                let dx = goal[0] - self.antposition[i][0];
                let dz = -goal[1] + self.antposition[i][1];
                let theta = if dz > EPSILON {
                    (dz / dx).atan()
                } else if dx > EPSILON {
                    0.0
                } else {
                    PI
                };
                let mut ideal = theta - self.antdirection[i];
                if ideal < -PI / 2.0 {
                    ideal += PI;
                }
                let dt = sign(ideal) * ideal.abs().min(PI / 90.0);
                self.antdirection[i] += dt;
                if self.antdirection[i] > 2.0 * PI {
                    self.antdirection[i] = 0.0;
                }
            }

            self.antposition[i][0] += self.antvelocity[i] * self.antdirection[i].cos();
            self.antposition[i][1] += self.antvelocity[i] * (-self.antdirection[i]).sin();
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut this = Antmaze {
        step: f64::from(random_below(90)),
        trackball: Trackball::new(),
        focus: 0,
        currentboard: 0,
        antdirection: [PI / 2.0, PI / 2.0, 0.0, PI / 2.0, PI / 2.0],
        // Upstream sets each ant's second coordinate twice over, so the third
        // is never set and every ant walks at zero; that is what it looks like.
        antposition: [
            [-4.0, 0.15, 0.0],
            [-4.0, 0.15, 0.0],
            [-1.0, 0.15, 0.0],
            [-3.9, 0.15, 0.0],
            [2.0, 0.15, 0.0],
        ],
        anton: [false; ANTCOUNT],
        antvelocity: [0.02; ANTCOUNT],
        antsize: [1.0; ANTCOUNT],
        antmaterial: [
            MATERIAL_RED,
            MATERIAL_GRAY35,
            MATERIAL_GRAY4,
            MATERIAL_ORANGE,
            MATERIAL_GREEN,
        ],
        board: [[[true; BOARDSIZE]; BOARDSIZE]; BOARDCOUNT],
        part: [0, 1, 5, 1, 3],
        antpath: [[[0.0; 2]; PARTS]; ANTCOUNT],
        antpathlength: [0; ANTCOUNT],
        checktexture: 0,
        brushedtexture: 0,
        elevator: 0.0,
        ant_step: 0.0,
        first_ant_step: 0.0,
        started: false,
        introduced: 0,
        entroducing: 12,
        fadeout: 1.0,
        fadeoutspeed: 0.0,
        mag: 4,
        win_w: g.width(),
        win_h: g.height(),
    };
    this.checktexture = Antmaze::make_check_image(g);
    this.brushedtexture = Antmaze::make_brushed_image(g);
    this.build_board(0);
    this.build_board(1);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Antmaze {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        self.win_w = width;
        self.win_h = height;
        // Thicker lines and bigger points on a bigger window: the ants' legs
        // are lines and their feet are points.
        let size = (width / 512 + 1) as f32;
        g.glx.line_width(size);
        g.glx.point_size(size);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        match *event {
            XEvent::ButtonPress { button: 3, .. } => {
                self.focus = (self.focus + 1) % ANTCOUNT;
                true
            }
            XEvent::ButtonPress { button: 4, .. } => {
                self.mag = (self.mag - 1).max(1);
                true
            }
            XEvent::ButtonPress { button: 5, .. } => {
                self.mag = (self.mag + 1).min(MAX_MAGNIFICATION);
                true
            }
            _ => false,
        }
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let (w, h) = (self.win_w, self.win_h);
        let ratio = h as f32 / w as f32;

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(false);
        g.glx.color_material(false);
        g.glx.lighting(true);
        // Upstream gives both lights a distance falloff, which there is no
        // equivalent of here; they are plain positional lights.
        for i in 0..2 {
            g.glx.light_enable(i, true);
            g.glx.light_ambient(i, AMBIENT);
            g.glx.light_diffuse(i, DIFFUSE);
        }
        g.glx.material_shininess(60.0);
        g.glx.material_specular([0.8, 0.8, 0.8, 1.0]);

        // The main view: a camera circling the board and leaning in and out.
        g.glx.viewport(w / 32, h / 8, 9 * w / 16, 3 * h / 4);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, 1.0 / ratio, 1.0, 25.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.light_position(0, 1.0, 5.0, 1.0, 1.0);
        g.glx.light_position(1, -1.0, -5.0, 1.0, 1.0);
        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -(self.mag as f32) - 5.0);
        g.glx.rotate(
            20.0 + 5.0 * (self.ant_step / 40.0).sin() as f32,
            1.0,
            0.0,
            0.0,
        );
        g.glx.mult_matrix(self.trackball.matrix());
        g.glx.rotate((self.ant_step * 0.6) as f32, 0.0, 1.0, 0.0);
        self.draw_strip(g);
        g.glx.pop_matrix();

        // The overhead view in the corner.
        let ratio2 = (3 * h / 8) as f32 / (w / 2) as f32;
        g.glx.viewport(17 * w / 32, h / 2, w / 2, 3 * h / 8);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, 1.0 / ratio2, 1.0, 25.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -16.0);
        g.glx.rotate(60.0, 1.0, 0.0, 0.0);
        g.glx
            .rotate(-15.0 + (self.ant_step / 10.0) as f32, 0.0, 1.0, 0.0);
        g.glx.mult_matrix(self.trackball.matrix());
        self.draw_strip(g);
        g.glx.pop_matrix();

        // And one ant turning on the spot, with no mandibles.
        g.glx.viewport(5 * w / 8, h / 8, 11 * w / 32, 3 * h / 8);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, 1.0 / ratio2, 1.0, 25.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -1.6);
        g.glx.rotate(30.0, 1.0, 0.0, 0.0);
        g.glx.rotate(self.ant_step as f32, 0.0, 1.0, 0.0);
        g.glx.rotate(90.0, 0.0, 0.0, 1.0);
        g.glx.texturing(true);
        g.glx.light_diffuse(0, DIFFUSE);
        g.glx.light_diffuse(1, DIFFUSE);
        g.glx.bind_texture(self.brushedtexture);
        let step = self.ant_step / 2.0;
        self.draw_ant(
            g,
            MATERIAL_GRAY35,
            step,
            AntStyle {
                textured: true,
                mandibles: false,
                shadow: true,
            },
        );
        g.glx.texturing(false);
        g.glx.pop_matrix();

        self.update_ants(g);
        self.step += 0.025;

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:         20000",
    "*showFPS:       False",
    "*fpsSolid:      True",
    // Upstream declares both of these and then never reads them.
    "*solidantmaze:  False",
    "*noants:        False",
];

const OPTS: &[Opt] =
    &[Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted()];

pub static DEF: SaverDef = SaverDef {
    slug: "antmaze",
    label: "Ant Maze",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Blair Tennessy",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=Bwa5-n6UUj8"),
        blurb: "Ants walk around a simple maze.",
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

    /// The scene is drawn three times over, in three viewports: the camera
    /// following the ants, the overhead view, and the one ant on its own.
    #[test]
    fn the_board_is_drawn_three_ways() {
        let mut r = start(StartArgs::new(800, 600, "", 20260812));
        for _ in 0..30 {
            r.step();
        }
        let f = r.frame();
        let mut views: Vec<[i32; 4]> = Vec::new();
        for b in &f.batches {
            if !views.contains(&b.viewport) {
                views.push(b.viewport);
            }
        }
        assert_eq!(views.len(), 3, "{views:?} is not three panels");
        // And none of them is the whole window.
        for v in &views {
            assert!(v[2] < 800 || v[3] < 600, "{v:?} covers the window");
        }
    }

    /// The path cut for each ant runs from one corner of the board towards the
    /// other, and every step of it is one square from the last.
    #[test]
    fn each_ant_gets_a_corridor() {
        let mut r = start(StartArgs::new(320, 240, "", 20260812));
        r.step();
        // The paths are cut at startup; walk them off the board state by
        // rebuilding one and checking its shape.
        let mut m = Antmaze {
            currentboard: 0,
            board: [[[true; BOARDSIZE]; BOARDSIZE]; BOARDCOUNT],
            antpath: [[[0.0; 2]; PARTS]; ANTCOUNT],
            antpathlength: [0; ANTCOUNT],
            ..dummy()
        };
        m.build_board(0);
        for i in 0..ANTCOUNT {
            let n = m.antpathlength[i];
            assert!(n > 3, "ant {i} got a path of {n} steps");
            // The first step is the corner it starts from.
            assert_eq!(m.antpath[i][0], [1.0 - 5.0, (BOARDSIZE - 2) as f64 - 5.0]);
            for j in 1..n {
                let d = (m.antpath[i][j][0] - m.antpath[i][j - 1][0]).abs()
                    + (m.antpath[i][j][1] - m.antpath[i][j - 1][1]).abs();
                assert_eq!(d, 1.0, "step {j} of ant {i} jumps {d} squares");
            }
        }
        assert!(!r.frame().vertices.is_empty(), "nothing was drawn");
    }

    /// Only the board the ants are actually on gets corridors cut in it; the
    /// other is left solid but for its two doorways, ready to be carved when
    /// it is that one's turn.
    #[test]
    fn only_the_board_in_play_is_carved() {
        let mut m = Antmaze {
            currentboard: 0,
            ..dummy()
        };
        m.build_board(1);
        let open = m.board[1].iter().flatten().filter(|&&c| !c).count();
        assert_eq!(open, 2, "{open} squares are open on the spare board");

        m.build_board(0);
        let carved = m.board[0].iter().flatten().filter(|&&c| !c).count();
        assert!(
            carved > 10,
            "only {carved} squares were carved for the ants"
        );
    }

    /// Enough of an `Antmaze` to exercise the board, with no display behind it.
    fn dummy() -> Antmaze {
        Antmaze {
            step: 0.0,
            trackball: Trackball::new(),
            focus: 0,
            currentboard: 0,
            antdirection: [0.0; ANTCOUNT],
            antposition: [[0.0; 3]; ANTCOUNT],
            anton: [false; ANTCOUNT],
            antvelocity: [0.02; ANTCOUNT],
            antsize: [1.0; ANTCOUNT],
            antmaterial: [MATERIAL_RED; ANTCOUNT],
            board: [[[true; BOARDSIZE]; BOARDSIZE]; BOARDCOUNT],
            part: [0; ANTCOUNT],
            antpath: [[[0.0; 2]; PARTS]; ANTCOUNT],
            antpathlength: [0; ANTCOUNT],
            checktexture: 0,
            brushedtexture: 0,
            elevator: 0.0,
            ant_step: 0.0,
            first_ant_step: 0.0,
            started: false,
            introduced: 0,
            entroducing: 12,
            fadeout: 1.0,
            fadeoutspeed: 0.0,
            mag: 4,
            win_w: 320,
            win_h: 240,
        }
    }
}
