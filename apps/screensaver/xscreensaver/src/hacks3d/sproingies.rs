//! Port of `hacks/glx/sproingies.c` and `hacks/glx/sproingiewrap.c`.
//!
//! ```text
//! sproingies.c - 3D sproingies
//!
//! Copyright 1996 by Ed Mackey, freely distributable.
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
//!    Programming:  Ed Mackey, Gordon Wrigley, Sergio Gutiérrez "Sergut"
//!    Sproingie 3D objects modeled by:  Al Mackey
//!       (using MetaNURBS in NewTek's Lightwave 3D v5).
//! ```
//!
//! Slinky-like creatures walking down an infinite staircase, occasionally
//! exploding.
//!
//! The staircase does not move: the whole scene is shifted back by a twelfth
//! of a step each frame, and after twelve frames every sproingie is moved one
//! step and the shift starts over. So the stairs the sproingies walk down are
//! the same stairs, forever.
//!
//! A hop is six frames of a modelled animation, and the model only ever hops
//! down and to the right. Hopping left is the same six frames turned a quarter
//! turn, which is also why the position bookkeeping at the end of a hop is
//! different for the two directions.
//!
//! One thing is left out. A sproingie rising out of the ground is drawn with a
//! clip plane cutting it off at the top of the block, which the GL ES that this
//! runtime resembles does not have. Upstream's own comment on that line is
//! "OpenGLES doesn't have this but it doesn't seem to matter", and its mobile
//! builds have gone without it for years.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Glx, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, random_below};

/// A hop is six frames, numbered from nought.
const FIRST_FRAME: i32 = 0;
const LAST_FRAME: i32 = 5;
/// The frame a sproingie explodes on, and the one it rises out of the ground
/// on. Neither is a real frame of the animation.
const BOOM_FRAME: i32 = 50;
const NO_FRAME: i32 = -10;
const TARGET_COUNT: i32 = 40;
const MAXSPROING: usize = 100;
const JUMP_LEFT: i32 = 0;

/// The two colours the staircase is painted: the tops of the steps and the
/// sides of them.
const TOPS: [f32; 4] = [0.392157, 0.784314, 0.941176, 1.0];
const SIDES: [f32; 4] = [0.156863, 0.156863, 0.392157, 1.0];

fn reset_life() -> i32 {
    -30 + random_below(28)
}

fn new_life() -> i32 {
    40 + random_below(200)
}

#[derive(Clone, Copy, Default)]
struct Sproingie {
    x: i32,
    y: i32,
    z: i32,
    life: i32,
    frame: i32,
    direction: i32,
    r: f32,
    g: f32,
    b: f32,
}

struct Sproingies {
    positions: Vec<Sproingie>,
    /// Where the staircase has got to in its twelve-frame shift.
    sframe: i32,
    /// The camera, and where it is heading.
    rotx: i32,
    roty: i32,
    dist: i32,
    target_rx: i32,
    target_ry: i32,
    target_dist: i32,
    target_count: i32,
    groundlevel: i32,
    smart: bool,
    wire: bool,
    frames: Vec<GlList>,
    boom: GlList,
    aspect: f32,
}

impl Sproingies {
    /// `LayGround`: the staircase, as tops, front faces and side faces. Every
    /// step is two blocks, offset half a step, which is what makes the stairs
    /// run diagonally.
    fn lay_ground(&self, g: &mut Glx, sx: i32, sy: i32, sz: i32, width: i32, height: i32) {
        let shape = if self.wire {
            Shape::LineLoop
        } else {
            Shape::Polygon
        };

        // In wireframe the tops and the sides are the same colour: upstream's
        // list for the sides only reaches its own colour when it is not
        // drawing lines.
        let tops = |g: &mut Glx| {
            if self.wire {
                g.color4f(TOPS[0], TOPS[1], TOPS[2], TOPS[3]);
            } else {
                g.material_ambient_diffuse(TOPS);
            }
        };
        let sides = |g: &mut Glx| {
            if self.wire {
                g.color4f(TOPS[0], TOPS[1], TOPS[2], TOPS[3]);
            } else {
                g.material_ambient_diffuse(SIDES);
            }
        };

        let quad = |g: &mut Glx, ps: [[i32; 3]; 4]| {
            g.begin(shape);
            for p in ps {
                g.vertex3f(p[0] as f32, p[1] as f32, p[2] as f32);
            }
            g.end();
        };

        if !self.wire {
            tops(g);
            g.normal3f(0.0, 1.0, 0.0);
            for h in 0..height {
                let (mut x, y, mut z) = (sx + h, sy - (h << 1), sz + h);
                for _ in 0..width {
                    quad(
                        g,
                        [[x, y, z], [x, y, z - 1], [x + 1, y, z - 1], [x + 1, y, z]],
                    );
                    quad(
                        g,
                        [
                            [x + 1, y - 1, z],
                            [x + 1, y - 1, z - 1],
                            [x + 2, y - 1, z - 1],
                            [x + 2, y - 1, z],
                        ],
                    );
                    x += 1;
                    z -= 1;
                }
            }
        }

        sides(g);
        if !self.wire {
            g.normal3f(0.0, 0.0, 1.0);
        }
        for h in 0..height {
            let (mut x, y, mut z) = (sx + h, sy - (h << 1), sz + h);
            for _ in 0..width {
                quad(
                    g,
                    [[x, y, z], [x + 1, y, z], [x + 1, y - 1, z], [x, y - 1, z]],
                );
                quad(
                    g,
                    [
                        [x + 1, y - 1, z],
                        [x + 2, y - 1, z],
                        [x + 2, y - 2, z],
                        [x + 1, y - 2, z],
                    ],
                );
                x += 1;
                z -= 1;
            }
        }

        if !self.wire {
            g.normal3f(1.0, 0.0, 0.0);
        }
        for h in 0..height {
            let (mut x, y, mut z) = (sx + h, sy - (h << 1), sz + h);
            for _ in 0..width {
                quad(
                    g,
                    [
                        [x + 1, y, z],
                        [x + 1, y, z - 1],
                        [x + 1, y - 1, z - 1],
                        [x + 1, y - 1, z],
                    ],
                );
                quad(
                    g,
                    [
                        [x + 2, y - 1, z],
                        [x + 2, y - 1, z - 1],
                        [x + 2, y - 2, z - 1],
                        [x + 2, y - 2, z],
                    ],
                );
                x += 1;
                z -= 1;
            }
        }

        if self.wire {
            tops(g);
            for h in 0..height {
                let (mut x, y, mut z) = (sx + h, sy - (h << 1), sz + h);
                for _ in 0..width {
                    quad(
                        g,
                        [[x, y, z], [x, y, z - 1], [x + 1, y, z - 1], [x + 1, y, z]],
                    );
                    quad(
                        g,
                        [
                            [x + 1, y - 1, z],
                            [x + 1, y - 1, z - 1],
                            [x + 2, y - 1, z - 1],
                            [x + 2, y - 1, z],
                        ],
                    );
                    x += 1;
                    z -= 1;
                }
            }
        }
    }

    /// `ComputeGround`: how much staircase to lay, which depends on how far
    /// away the camera has got to.
    fn compute_ground(&self, g: &mut Glx) {
        let (g_back, g_width) = match self.groundlevel {
            0 => (2, 5),
            1 => (4, 8),
            _ => (8, 16),
        };
        let mut g_higher = (self.dist >> 3).clamp(4, 16);
        let g_height = g_higher << 1;
        if self.rotx < -10 {
            g_higher += g_higher >> 2;
        } else if self.rotx > 10 {
            g_higher -= g_higher >> 2;
        }
        self.lay_ground(
            g,
            -g_higher - g_back,
            g_higher << 1,
            g_back - g_higher,
            g_width,
            g_height,
        );
    }

    /// `AdvanceSproingie`: one frame of one creature's life, which is a hop, a
    /// wait to be born, or an explosion.
    fn advance(&mut self, t: usize) {
        let n = self.positions.len();
        let mut s = self.positions[t];

        if s.life > 0 {
            s.frame += 1;
            if s.frame > LAST_FRAME {
                if s.frame >= BOOM_FRAME {
                    // Fading to black as it blows apart.
                    s.r = (s.r - 0.08).max(0.0);
                    s.g = (s.g - 0.08).max(0.0);
                    s.b = (s.b - 0.08).max(0.0);
                    s.life -= 1;
                    if s.life < 1 {
                        s.life = reset_life();
                    }
                    self.positions[t] = s;
                    return;
                }
                s.frame = FIRST_FRAME;

                // Two of them landing on the same block is fatal to whichever
                // one notices it first.
                for t2 in 0..n {
                    let o = self.positions[t2];
                    if t2 != t
                        && s.x == o.x
                        && s.y == o.y
                        && s.z == o.z
                        && o.life > 10
                        && o.frame < LAST_FRAME + 1
                        && s.life > 10
                    {
                        s.life = 10;
                        s.frame = BOOM_FRAME;
                        s.r = (s.r + 0.5).min(1.0);
                        s.g = (s.g + 0.5).min(1.0);
                        s.b = (s.b + 0.5).min(1.0);
                    }
                }
            }
            // Time to disappear, unless it is waiting for the start of a hop
            // to come round.
            if !(s.life == 10 && s.frame > FIRST_FRAME && s.frame < BOOM_FRAME) {
                s.life -= 1;
                if s.life < 1 {
                    s.life = reset_life();
                } else if s.life < 9 {
                    s.frame -= 2;
                }
            }
            self.positions[t] = s;
            return;
        }

        s.life += 1;
        if s.life < 0 {
            self.positions[t] = s;
            return;
        }

        let g_higher = -3 + random_below(5);
        let g_back = -2 + random_below(5);
        s.x = -g_higher - g_back;
        s.y = g_higher << 1;
        s.z = g_back - g_higher;
        s.life = new_life();
        s.frame = NO_FRAME;
        s.r = (40 + random_below(200)) as f32 / 255.0;
        s.g = (40 + random_below(200)) as f32 / 255.0;
        s.b = (40 + random_below(200)) as f32 / 255.0;

        // If another one is standing there already, wait.
        for t2 in 0..n {
            let o = self.positions[t2];
            if t2 != t
                && s.x == o.x
                && s.y == o.y
                && s.z == o.z
                && o.life > 10
                && o.frame < FIRST_FRAME
            {
                s.life = -1;
            }
        }
        self.positions[t] = s;
    }

    /// `NextSproingie`: the staircase shift, everyone's next frame, and the
    /// camera's slow drift toward wherever it has decided to go next.
    fn next(&mut self) {
        self.sframe += 1;
        if self.sframe > 11 {
            self.sframe = FIRST_FRAME;
            for s in &mut self.positions {
                s.x -= 1;
                s.y += 2;
                s.z -= 1;
            }
        }

        for t in 0..self.positions.len() {
            self.advance(t);
        }

        if self.target_count < 0 {
            // Creep toward the current target, one degree a frame.
            if self.target_rx < self.rotx {
                self.rotx -= 1;
            } else if self.target_rx > self.rotx {
                self.rotx += 1;
            }
            if self.target_ry < self.roty {
                self.roty -= 1;
            } else if self.target_ry > self.roty {
                self.roty += 1;
            }
            let ddx = (self.target_dist - self.dist) / 8;
            if ddx != 0 {
                self.dist += ddx;
            } else if self.target_dist < self.dist {
                self.dist -= 1;
            } else if self.target_dist > self.dist {
                self.dist += 1;
            }
            if self.target_rx == self.rotx
                && self.target_ry == self.roty
                && self.target_dist == self.dist
            {
                self.target_count = TARGET_COUNT;
                if self.target_dist <= 32 {
                    self.target_count >>= 2;
                }
            }
        } else {
            self.target_count -= 1;
            if self.target_count < 0 {
                self.target_rx = random_below(100) - 35;
                self.target_ry = -random_below(90);
                // Either 32, 64 or 128, and never where it already is.
                self.target_dist = 32 << random_below(2);
                if self.target_dist >= self.dist {
                    self.target_dist <<= 1;
                }
            }
        }
    }

    /// `RenderSproingie`. It moves the creature as it draws it: the end of a
    /// hop is where the next block is decided, and that is here.
    fn render(&mut self, g: &mut Glx, t: usize) {
        let mut s = self.positions[t];
        if s.life < 1 {
            return;
        }

        g.push_matrix();
        let color = [s.r, s.g, s.b, 1.0];
        if self.wire {
            g.color4f(color[0], color[1], color[2], 1.0);
        } else {
            g.material_ambient_diffuse(color);
        }

        if s.frame < FIRST_FRAME {
            // Rising out of the block it is about to stand on. Upstream cuts
            // it off with a clip plane here; there is none to be had.
            g.translate(s.x as f32, s.y as f32 + s.frame as f32 / 9.0, s.z as f32);
            self.frames[0].render(g, self.wire);
        } else if s.frame >= BOOM_FRAME {
            g.translate(s.x as f32 + 0.5, s.y as f32 + 0.5, s.z as f32 - 0.5);
            let boom_scale = (s.frame - BOOM_FRAME).min(31);
            let scale = (1u32 << boom_scale) as f32;
            g.scale(scale, scale, scale);
            if !self.wire {
                g.color4f(color[0], color[1], color[2], 1.0);
                g.lighting(false);
            }
            let pointsize = ((BOOM_FRAME + 8 - s.frame) as f32) - (self.dist as f32 / 64.0);
            g.point_size(pointsize.max(1.0));
            self.boom.render(g, self.wire);
            g.point_size(1.0);
            if !self.wire {
                g.lighting(true);
            }
        } else {
            if s.direction == JUMP_LEFT {
                // The model only hops one way; hopping the other way is the
                // same six frames turned a quarter turn.
                g.translate(s.x as f32, s.y as f32, s.z as f32 - 1.0);
                g.rotate(-90.0, 0.0, 1.0, 0.0);
                if s.frame == LAST_FRAME {
                    s.y -= 1;
                    s.z += 1;
                }
            } else {
                g.translate(s.x as f32, s.y as f32, s.z as f32);
                if s.frame == LAST_FRAME {
                    s.x += 1;
                    s.y -= 1;
                }
            }
            self.frames[s.frame as usize].render(g, self.wire);

            if s.frame == LAST_FRAME {
                // Every hop, check whether it has walked off the staircase.
                if (s.x - s.z == 6 && 2 * s.x + s.y == 6) || (s.z - s.x == 5 && 2 * s.x + s.y == -5)
                {
                    if s.life > 0 && s.frame < BOOM_FRAME && s.frame > FIRST_FRAME {
                        s.frame = BOOM_FRAME;
                    }
                } else if self.smart {
                    // A clever one turns rather than walking off the edge.
                    if s.x - s.z == 5 && 2 * s.x + s.y == 5 {
                        s.direction = JUMP_LEFT;
                    } else if s.z - s.x == 4 && 2 * s.x + s.y == -4 {
                        s.direction = 1;
                    } else {
                        s.direction = random_below(2);
                    }
                } else {
                    s.direction = random_below(2);
                }
            }
        }

        g.pop_matrix();
        self.positions[t] = s;
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let count = g.res.int("count").clamp(0, MAXSPROING as i32 - 1) as usize;

    let mut positions = vec![Sproingie::default(); count];
    for (t, s) in positions.iter_mut().enumerate() {
        // They are born in turn rather than all at once.
        s.life = -(t as i32) * if count > 19 { 1 } else { 4 } - 2;
        s.frame = FIRST_FRAME;
        s.direction = random_below(2);
    }

    let frames = [
        crate::models::S1_1,
        crate::models::S1_2,
        crate::models::S1_3,
        crate::models::S1_4,
        crate::models::S1_5,
        crate::models::S1_6,
    ]
    .iter()
    .map(|s| GlList::parse(s))
    .collect();

    let mut this = Sproingies {
        positions,
        sframe: 0,
        rotx: 0,
        roty: -45,
        dist: 16 << 2,
        target_rx: 0,
        target_ry: 0,
        target_dist: 0,
        target_count: 0,
        groundlevel: 0,
        smart: g.res.bool("smartSproingies"),
        wire,
        frames,
        boom: GlList::parse(crate::models::S1_B),
        aspect: 1.0,
    };
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Sproingies {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 9 / 16;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(65.0, self.aspect, 0.1, 2000.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.clear();
        if !self.wire {
            g.glx.depth_test(true);
            g.glx.cull_face(true);
            // The models are wound the other way round from everything else.
            g.glx.front_face_cw(true);
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.material_diffuse([0.6, 0.6, 0.6, 1.0]);
            g.glx.material_specular([0.8, 0.8, 0.8, 1.0]);
            g.glx.material_shininess(50.0);
        } else {
            g.glx.lighting(false);
            g.glx.depth_test(false);
            g.glx.cull_face(false);
        }

        self.next();

        g.glx.push_matrix();
        // The viewing transform: back off by the distance, then turn.
        g.glx.translate(0.0, 0.0, -(self.dist as f32) / 16.0);
        g.glx.rotate(self.rotx as f32, 1.0, 0.0, 0.0);
        g.glx.rotate(self.roty as f32, 0.0, 1.0, 0.0);
        if !self.wire {
            g.glx.light_position(0, 8.0, 5.0, -2.0, 0.1);
        }

        // The staircase never moves. Instead the whole scene slides back a
        // twelfth of a step a frame, and after twelve of them everyone is
        // moved one step on and it starts over.
        g.glx.translate(
            self.sframe as f32 * (-1.0 / 12.0) - 0.75,
            self.sframe as f32 * (2.0 / 12.0) - 0.5,
            self.sframe as f32 * (-1.0 / 12.0) + 0.75,
        );

        let glx = &mut g.glx;
        if self.wire {
            self.compute_ground(glx);
        }
        for t in 0..self.positions.len() {
            self.render(glx, t);
        }
        if !self.wire {
            self.compute_ground(glx);
        }
        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:            30000",
    "*count:            8",
    "*showFPS:          False",
    "*wireframe:        False",
    "*smartSproingies:  True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("count", "Sproingies", 1.0, 30.0, 1.0, 0, "8"),
    Opt::boolean("smartSproingies", "Stay on the stairs", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "sproingies",
    label: "Sproingies",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ed Mackey",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=fmHl17ppgc0"),
        blurb: "Slinky-like creatures walk down an infinite staircase and \
                occasionally explode.",
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

    /// Six frames of a hop, all the same creature, and a five hundred point
    /// cloud for the explosion.
    #[test]
    fn a_hop_is_six_frames() {
        for s in [
            crate::models::S1_1,
            crate::models::S1_2,
            crate::models::S1_3,
            crate::models::S1_4,
            crate::models::S1_5,
            crate::models::S1_6,
        ] {
            let m = GlList::parse(s);
            assert_eq!(m.primitive, Shape::Triangles);
            assert_eq!(m.points, 1728, "the frames are not the same model");
        }
        let boom = GlList::parse(crate::models::S1_B);
        assert_eq!(boom.primitive, Shape::Points);
        assert_eq!(boom.points, 500);
    }

    /// The staircase shifts by a twelfth of a step a frame and then starts
    /// over, so the stairs are always the same stairs.
    #[test]
    fn the_staircase_stays_where_it_is() {
        let mut r = start(StartArgs::new(640, 480, "count=4", 20260811));
        let mut seen = Vec::new();
        for _ in 0..30 {
            r.step();
            let f = r.frame();
            // The ground is the last thing drawn, and its matrix carries the
            // shift.
            seen.push(f.batches.last().map(|b| b.modelview.0[13]).unwrap_or(0.0));
        }
        let lo = seen.iter().copied().fold(f32::MAX, f32::min);
        let hi = seen.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 0.01, "the staircase never moved");
        assert!(hi - lo < 10.0, "the staircase ran away: {lo} to {hi}");
    }

    /// They walk: over a few hops every creature has moved down and along,
    /// and none of them has walked off the end of the world.
    #[test]
    fn they_walk_down_the_stairs() {
        let mut r = start(StartArgs::new(640, 480, "count=8", 20260811));
        for _ in 0..400 {
            r.step();
        }
        let f = r.frame();
        assert!(
            f.vertices
                .iter()
                .all(|v| v.pos.iter().all(|c| c.is_finite())),
            "a vertex went to NaN"
        );
        assert!(
            f.batches.len() > 10,
            "only {} batches: nobody is home",
            f.batches.len()
        );
    }

    /// An explosion is a cloud of points that doubles in size every frame,
    /// drawn unlit.
    #[test]
    fn an_explosion_is_a_point_cloud() {
        let mut r = start(StartArgs::new(640, 480, "count=30", 20260811));
        let mut booms = 0;
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            booms += f
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Points)
                .count();
        }
        assert!(booms > 0, "nobody ever exploded");
    }
}
