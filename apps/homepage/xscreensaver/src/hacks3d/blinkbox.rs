//! Port of `hacks/glx/blinkbox.c`.
//!
//! ```text
//! blinkbox, Copyright (c) 2003 Jeremy English <jenglish@myself.com>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! motion blur added March 2005 by John Boero <jlboero@cs.uwm.edu>
//! ```
//!
//! A ball bounces around inside a box you cannot see, and each wall lights up
//! where it was hit and fades out again over the next twenty frames. So the
//! room is drawn entirely by being hit, and if the ball ever settled into a
//! path the box would go dark.
//!
//! It does not settle, and the reason is worth reading. A bounce does not
//! simply negate the velocity along that axis: it swaps that component with a
//! held-aside one, and then replaces the magnitude with a fresh 1 or 2 while
//! keeping the sign. So every wall the ball touches changes its speed as well
//! as its direction, and the six numbers driving it keep being shuffled between
//! each other.
//!
//! The motion blur is not a buffer effect. The ball is drawn twenty-four times
//! along the path it is about to take, each one a twenty-fourth of the way
//! further on, and the alpha of each follows a half sine so the streak is
//! brightest in the middle. All of it is additive, which is why the streak
//! glows rather than smearing.
//!
//! One upstream oddity kept as it is: the top face of the cube is given the
//! normal `(1, 1, 0)` rather than `(0, 1, 0)`. It is a typo two decades old, it
//! lights that face slightly wrong, and it is what the saver looks like.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, random, unit_sphere};

/// How many frames a wall stays lit after it is hit.
const MAX_COUNT: i32 = 20;
/// How much of the wall's brightness one of those frames takes.
const ALPHA_AMT: f32 = 0.05;
/// How many copies of the ball the streak is made of.
const BLUR_DETAIL: usize = 24;

/// How densely to render spheres.
const SPHERE_SLICES: i32 = 12;
const SPHERE_STACKS: i32 = 16;

/// The room, which is never drawn.
const BBOX_TOP: [f32; 3] = [14.0, 14.0, 20.0];
const BBOX_BOTTOM: [f32; 3] = [-14.0, -14.0, -20.0];

/// One wall: whether it is lit, where it was hit, and how far through fading
/// out it is.
#[derive(Clone, Copy, Default)]
struct Side {
    hit: bool,
    pos: [f32; 3],
    counter: i32,
    color: [f32; 3],
    rot: [f32; 4],
    des_count: i32,
    alpha_count: i32,
}

/// Which wall is which. The order is the order they are drawn in.
const LEFT: usize = 0;
const RIGHT: usize = 1;
const TOP: usize = 2;
const BOTTOM: usize = 3;
const FRONT: usize = 4;
const BACK: usize = 5;

struct BlinkBox {
    ball: [f32; 3],
    /// The ball's radius, for deciding when it has touched a wall.
    ball_d: f32,

    /// Width and height of a wall tile, and its depth.
    bscale_wh: f32,
    bscale_d: f32,

    /// The velocity, and the three numbers a bounce swaps it with.
    mo: [f32; 3],
    moh: [f32; 3],

    bpos: [f32; 3],

    ball_list: u32,
    box_list: u32,
    des_amt: f32,

    sides: [Side; 6],

    do_dissolve: bool,
    do_fade: bool,
    do_blur: bool,
    wireframe: bool,
}

/// `get_rand`: 1 or 2, which is the whole of the speed the ball can have along
/// any one axis.
fn get_rand() -> f32 {
    (1 + random() % 2) as f32
}

/// `swap_mov`: swap the two, then give the first a fresh magnitude while
/// keeping whichever way it is now going.
fn swap_mov(a: &mut f32, b: &mut f32) {
    std::mem::swap(a, b);
    let j = get_rand();
    *a = if *a < 0.0 { -j } else { j };
}

impl BlinkBox {
    /// Light up a wall where the ball just touched it.
    fn strike(&mut self, side: usize, axis: usize) {
        let ball = self.ball;
        let s = &mut self.sides[side];
        s.hit = true;
        s.counter = MAX_COUNT;
        s.des_count = 1;
        s.alpha_count = 0;
        s.pos = ball;
        let mut mo = self.mo[axis];
        let mut moh = self.moh[axis];
        swap_mov(&mut mo, &mut moh);
        self.mo[axis] = mo;
        self.moh[axis] = moh;
    }

    fn hit_walls(&mut self) {
        // Order matters only in that a corner hit lights the walls in this
        // order; upstream tests top and bottom first.
        for (axis, (lo, hi)) in [(1, (BOTTOM, TOP)), (2, (BACK, FRONT)), (0, (LEFT, RIGHT))] {
            if self.ball[axis] - self.ball_d <= BBOX_BOTTOM[axis] {
                self.strike(lo, axis);
            } else if self.ball[axis] + self.ball_d >= BBOX_TOP[axis] {
                self.strike(hi, axis);
            }
        }
    }

    /// Make sure it's inside of the bounding box.
    fn check_box_pos(&mut self, bot_x: f32, top_x: f32, bot_y: f32, top_y: f32) {
        let wh = self.bscale_wh;
        if self.bpos[0] - wh < bot_x {
            self.bpos[0] = bot_x + wh;
        }
        if self.bpos[0] + wh > top_x {
            self.bpos[0] = top_x - wh;
        }
        if self.bpos[1] - wh < bot_y {
            self.bpos[1] = bot_y + wh;
        }
        if self.bpos[1] + wh > top_y {
            self.bpos[1] = top_y - wh;
        }
    }

    /// Where on the wall the mark goes, which is the hit position with two of
    /// its three coordinates read off in the wall's own frame.
    fn place_mark(&mut self, i: usize) {
        let p = self.sides[i].pos;
        let d = self.bscale_d;
        let (bpos, clamp) = match i {
            LEFT => ([-p[2], p[1], BBOX_BOTTOM[0] - d], (2, 1)),
            RIGHT => ([-p[2], p[1], BBOX_TOP[0] + d], (2, 1)),
            TOP => ([p[0], p[2], BBOX_BOTTOM[1] - d], (0, 2)),
            BOTTOM => ([p[0], p[2], BBOX_TOP[1] + d], (0, 2)),
            FRONT => ([p[1], -p[0], BBOX_TOP[2] + d], (1, 0)),
            _ => ([p[1], -p[0], BBOX_BOTTOM[2] + d], (1, 0)),
        };
        self.bpos = bpos;
        if self.sides[i].hit {
            let (a, b) = clamp;
            self.check_box_pos(BBOX_BOTTOM[a], BBOX_TOP[a], BBOX_BOTTOM[b], BBOX_TOP[b]);
        }
    }
}

/// `unit_cube`, kept face for face including the top one's wrong normal.
fn unit_cube(g: &mut Gl, wire: bool) {
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, -1.0, 0.0],
            [
                [-1.0, -1.0, -1.0],
                [1.0, -1.0, -1.0],
                [1.0, -1.0, 1.0],
                [-1.0, -1.0, 1.0],
            ],
        ),
        (
            [0.0, 0.0, 1.0],
            [
                [-1.0, -1.0, 1.0],
                [1.0, -1.0, 1.0],
                [1.0, 1.0, 1.0],
                [-1.0, 1.0, 1.0],
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [-1.0, -1.0, -1.0],
                [-1.0, 1.0, -1.0],
                [1.0, 1.0, -1.0],
                [1.0, -1.0, -1.0],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [1.0, -1.0, -1.0],
                [1.0, 1.0, -1.0],
                [1.0, 1.0, 1.0],
                [1.0, -1.0, 1.0],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [-1.0, -1.0, -1.0],
                [-1.0, -1.0, 1.0],
                [-1.0, 1.0, 1.0],
                [-1.0, 1.0, -1.0],
            ],
        ),
        (
            // Upstream's typo: the top face's normal should be (0, 1, 0).
            [1.0, 1.0, 0.0],
            [
                [-1.0, 1.0, -1.0],
                [-1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, -1.0],
            ],
        ),
    ];

    g.glx
        .begin(if wire { Shape::LineLoop } else { Shape::Quads });
    for (n, vs) in faces {
        g.glx.normal3f(n[0], n[1], n[2]);
        for v in vs {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
    }
    g.glx.end();
}

impl Hack3d for BlinkBox {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        self.hit_walls();

        // Not inside a push and pop, so it accumulates: the whole room turns a
        // quarter of a degree about each axis every frame, for ever.
        g.glx.rotate(0.25, 0.0, 0.0, 1.0);
        g.glx.rotate(0.25, 0.0, 1.0, 0.0);
        g.glx.rotate(0.25, 1.0, 0.0, 0.0);

        g.glx.push_matrix();

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        } * 0.5;
        g.glx.scale(s, s, s);

        let white = |g: &mut Gl, a: f32| {
            g.glx.color4f(1.0, 1.0, 1.0, a);
            // GL_COLOR_MATERIAL: with lighting on it is the material that is
            // shaded, so the colour has to go to both.
            g.glx.material_ambient_diffuse([1.0, 1.0, 1.0, a]);
        };
        white(g, 1.0);
        g.glx.push_matrix();

        if !self.do_blur || self.wireframe {
            for k in 0..3 {
                self.ball[k] += self.mo[k];
            }
            g.glx.translate(self.ball[0], self.ball[1], self.ball[2]);
            g.glx.scale(2.0, 2.0, 2.0);
            g.glx.call_list(self.ball_list);
        } else {
            g.glx.blend(Blend::AlphaAdd);
            g.glx.translate(self.ball[0], self.ball[1], self.ball[2]);

            for i in 0..BLUR_DETAIL {
                let n = BLUR_DETAIL as f32;
                g.glx
                    .translate(self.mo[0] / n, self.mo[1] / n, self.mo[2] / n);

                // A half sine along the streak, so it is brightest in the
                // middle and fades at both ends. Upstream leaves a comment
                // offering a flat 1/n instead, and calls it boring.
                let ball_alpha = (std::f32::consts::PI / n * i as f32).sin() / n;
                white(g, ball_alpha);

                g.glx.scale(2.0, 2.0, 2.0);
                g.glx.call_list(self.ball_list);
                g.glx.scale(0.5, 0.5, 0.5);
            }
            for k in 0..3 {
                self.ball[k] += self.mo[k];
            }
        }
        g.glx.pop_matrix();

        for i in 0..6 {
            self.place_mark(i);
            if !self.sides[i].hit {
                continue;
            }
            let s = self.sides[i];
            let c = s.color;
            let a = if self.do_fade {
                1.0 - (ALPHA_AMT * s.alpha_count as f32)
            } else {
                1.0
            };
            g.glx.color4f(c[0], c[1], c[2], a);
            g.glx.material_ambient_diffuse([c[0], c[1], c[2], a]);
            g.glx.blend(Blend::AlphaAdd);

            g.glx.push_matrix();
            g.glx.rotate(s.rot[0], s.rot[1], s.rot[2], s.rot[3]);
            g.glx.translate(self.bpos[0], self.bpos[1], self.bpos[2]);
            let wh = if self.do_dissolve {
                self.bscale_wh - self.des_amt * s.des_count as f32
            } else {
                self.bscale_wh
            };
            g.glx.scale(wh, wh, self.bscale_d);
            g.glx.call_list(self.box_list);
            g.glx.pop_matrix();

            let s = &mut self.sides[i];
            s.counter -= 1;
            s.des_count += 1;
            s.alpha_count += 1;
            if s.counter == 0 {
                s.hit = false;
            }
        }

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
        // The up vector is not up: it leans ten parts back for two parts up,
        // which is where the view down into the box comes from.
        g.glx
            .look_at([0.0, 0.0, 40.0], [0.0, 0.0, 0.0], [0.0, 2.0, 10.0]);
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let do_dissolve = g.res.bool("dissolve");

    let mut bscale_wh = g.res.float("boxsize") as f32;
    if !(1.0..=8.0).contains(&bscale_wh) {
        /* Boxsize out of range. Using default */
        bscale_wh = 2.0;
    }

    let mut sides = [Side::default(); 6];
    for s in &mut sides {
        s.counter = MAX_COUNT;
        s.des_count = 1;
        s.alpha_count = 1;
        s.rot[0] = 90.0;
    }
    sides[LEFT].color = [1.0, 0.0, 0.0]; /*Red*/
    sides[RIGHT].color = [0.0, 1.0, 0.0]; /*Green*/
    sides[TOP].color = [0.0, 0.0, 1.0]; /*Blue*/
    sides[BOTTOM].color = [1.0, 0.5, 0.0]; /*Orange*/
    sides[FRONT].color = [1.0, 1.0, 0.0]; /*Yellow*/
    sides[BACK].color = [0.5, 0.0, 1.0]; /*Purple*/

    // Each wall's mark is drawn in the xy plane and turned to face into the
    // room: the two side walls about y, the floor and ceiling about x, and the
    // two ends about z.
    sides[LEFT].rot[2] = 1.0;
    sides[RIGHT].rot[2] = 1.0;
    sides[TOP].rot[1] = 1.0;
    sides[BOTTOM].rot[1] = 1.0;
    sides[FRONT].rot[3] = 1.0;
    sides[BACK].rot[3] = 1.0;

    let mut st = BlinkBox {
        ball: [0.0, 0.0, 0.0],
        ball_d: 1.0,
        bscale_wh,
        bscale_d: 0.25,
        mo: [1.0, 1.0, 1.0],
        moh: [-1.0, -1.5, -1.5],
        bpos: [1.0, 1.0, 1.0],
        ball_list: 0,
        box_list: 0,
        des_amt: if do_dissolve {
            bscale_wh / MAX_COUNT as f32
        } else {
            1.0
        },
        sides,
        do_dissolve,
        do_fade: g.res.bool("fade"),
        do_blur: g.res.bool("blur"),
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    st.ball_list = g.glx.gen_lists(1);
    g.glx.new_list(st.ball_list);
    unit_sphere(&mut g.glx, SPHERE_STACKS, SPHERE_SLICES, wire);
    g.glx.end_list();

    st.box_list = g.glx.gen_lists(1);
    g.glx.new_list(st.box_list);
    unit_cube(g, wire);
    g.glx.end_list();

    if !wire {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(0, 20.0, 100.0, 20.0, 1.0);
        // The marks and the streak are drawn over each other, so depth testing
        // would only decide which of two transparent things wins.
        if st.do_fade || st.do_blur {
            g.glx.depth_test(false);
        } else {
            g.glx.depth_test(true);
        }
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*boxsize:      2",
    "*dissolve:     False",
    "*fade:         True",
    "*blur:         True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("boxsize", "Box size", 1.0, 8.0, 0.1, 1, "2"),
    Opt::boolean("fade", "Fade", "true"),
    Opt::boolean("blur", "Motion blur", "true"),
    Opt::boolean("dissolve", "Dissolve", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "blinkbox",
    label: "Blink Box",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jeremy English",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=lgjbHMcSd8U"),
        blurb: "A ball bounces inside a box whose tiles only appear on impact.",
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
    use crate::runtime::ya_rand_init;

    /// A bounce swaps the two numbers and gives the first a fresh magnitude
    /// without changing which way it now points.
    #[test]
    fn a_bounce_shuffles_the_speeds() {
        ya_rand_init(20260811);
        let (mut a, mut b) = (1.0f32, -1.5f32);
        swap_mov(&mut a, &mut b);
        assert_eq!(b, 1.0, "the old value is held aside");
        assert!(a < 0.0, "it kept going the way the swap pointed it");
        assert!(a == -1.0 || a == -2.0, "a fresh magnitude, {a}");
    }

    /// The ball stays in the box, whatever the shuffling does to its speed.
    #[test]
    fn the_ball_stays_in_the_room() {
        let mut r = start(StartArgs::new(640, 480, "blur=false", 20260811));
        for _ in 0..3000 {
            r.step();
            let f = r.frame();
            // The ball is the first thing drawn, and the only thing drawn as a
            // triangle strip.
            let Some(b) = f
                .batches
                .iter()
                .find(|b| b.primitive == crate::runtime::gl::Primitive::TriangleStrip)
            else {
                continue;
            };
            for v in &f.vertices[b.first..b.first + b.count] {
                let p = b.modelview.transform(v.pos);
                // Eye space, forty back. The room is 28 by 28 by 40, halved by
                // the scene scale, so this is roomy but finite.
                assert!(
                    p[0].abs() < 20.0 && p[1].abs() < 20.0,
                    "the ball escaped to {p:?}"
                );
            }
        }
    }

    /// A wall lights up when it is hit and goes out twenty frames later. It
    /// has to do both: a wall that never lights is a black screen, and one
    /// that never goes out is a box.
    #[test]
    fn the_walls_light_up_and_go_out_again() {
        let mut r = start(StartArgs::new(640, 480, "blur=false", 20260811));
        // A wall mark is the only thing drawn as triangles, since the cube is
        // quads and the ball is a strip.
        let marks = |r: &Runner3d| {
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
                .count()
        };
        let mut lit = 0;
        let mut dark = 0;
        for _ in 0..400 {
            r.step();
            if marks(&r) > 0 {
                lit += 1;
            } else {
                dark += 1;
            }
        }
        assert!(lit > 50, "the walls only lit on {lit} frames");
        assert!(dark > 10, "the walls never went out");
    }
}
