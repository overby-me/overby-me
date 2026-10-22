//! Port of `hacks/glx/cubestack.c`.
//!
//! ```text
//! cubestack, Copyright © 2016-2025 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! A cube builds itself out of five square frames, one face at a time, each
//! swinging up on the hinge it shares with the one before it. When the last
//! one closes, the whole stack shifts down and the next cube starts on top of
//! it, so it is forever assembling itself upwards while the finished ones
//! recede.
//!
//! A face is not a solid square: it is four struts around the edge with a
//! little tick pointing inwards at each corner, and the whole thing is drawn
//! translucent and additively, so the stack behind shows through and the
//! overlaps brighten. There is no lighting and no depth testing at all, which
//! is what lets every cube in the stack be visible through every other.
//!
//! The swing is eased rather than linear ([`Ease::InOutSine`]), and that is
//! most of why it reads as a hinge rather than a slide: it starts slowly,
//! swings, and settles.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Ease, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, ease,
};

/// How many cubes deep the stack goes before the oldest is forgotten.
const MAX_LENGTH: i32 = 20;

/// The six steps of a cube: the bottom, then four sides swinging up, then the
/// top closing over.
const STATES: f32 = 6.0;

struct CubeStack {
    rot: Rotator,
    trackball: Trackball,
    /// How far through building the newest cube, 0 to 6.
    state: f32,
    /// The spin about the stack's own axis, in degrees.
    r: f32,
    length: i32,
    colors: Vec<XColor>,
    ccolor: usize,
    speed: f32,
    thickness: f32,
    opacity: f32,
    wireframe: bool,
}

impl CubeStack {
    /// One edge of a face: a bar along the bottom, and a tick pointing in at
    /// the corner if there is room for one.
    fn draw_strut(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let t = self.thickness;
        g.glx.push_matrix();
        g.glx.normal3f(0.0, 0.0, -1.0);
        g.glx.translate(-0.5, -0.5, 0.0);
        g.glx.begin(if wire {
            Shape::LineLoop
        } else {
            Shape::TriangleFan
        });
        g.glx.vertex3f(0.0, 0.0, 0.0);
        g.glx.vertex3f(1.0, 0.0, 0.0);
        g.glx.vertex3f(1.0 - t, t, 0.0);
        g.glx.vertex3f(t, t, 0.0);
        g.glx.end();

        let h = 0.5 - t;
        if h >= 0.25 {
            g.glx.begin(if wire {
                Shape::LineLoop
            } else {
                Shape::TriangleFan
            });
            g.glx.vertex3f(0.5, 0.5, 0.0);
            g.glx.vertex3f(0.5 - t / 2.0, 0.5 - t / 2.0, 0.0);
            g.glx.vertex3f(0.5 - t / 2.0, 0.5 - h / 2.0, 0.0);
            g.glx.vertex3f(0.5 + t / 2.0, 0.5 - h / 2.0, 0.0);
            g.glx.vertex3f(0.5 + t / 2.0, 0.5 - t / 2.0, 0.0);
            g.glx.end();
        }
        g.glx.pop_matrix();
    }

    /// The four struts of one square, each a quarter turn from the last.
    fn draw_face(&self, g: &mut Gl) {
        for _ in 0..4 {
            self.draw_strut(g);
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
        }
    }

    /// Set the colour, fading it in as its face swings up. With lighting off
    /// the vertex colour is what shows; the material is set alongside it
    /// because upstream sets both.
    fn colorize(&self, g: &mut Gl, color: [f32; 4], r: f32) {
        let c = [color[0], color[1], color[2], color[3] * r];
        g.glx.material_ambient_diffuse(c);
        g.glx.color4f(c[0], c[1], c[2], c[3]);
    }

    /// One cube, drawn as far through its assembly as `state` says.
    ///
    /// A negative state is the bottom face fading in before anything swings;
    /// each whole number after that is another face finished, and the
    /// fractional part is the one currently on its way up.
    fn draw_cube_1(&self, g: &mut Gl, state: f32, color: [f32; 4], bottom: bool) {
        let istate = state as i32;
        let r = ease(Ease::InOutSine, f64::from(state - istate as f32)) as f32;

        if bottom {
            let r2 = if state < 0.0 { 1.0 + state } else { 1.0 };
            self.colorize(g, color, r2);
            self.draw_face(g); /* Bottom */
        }

        if state >= 0.0 {
            /* Left */
            let r2 = if istate == 0 { r } else { 1.0 };
            self.colorize(g, color, r2);
            g.glx.push_matrix();
            g.glx.translate(-0.5, 0.5, 0.0);
            g.glx.rotate(-r2 * 90.0, 0.0, 1.0, 0.0);
            g.glx.translate(0.5, -0.5, 0.0);
            self.draw_face(g);
            g.glx.pop_matrix();
        }

        if state >= 1.0 {
            /* Back */
            let r2 = if istate == 1 { r } else { 1.0 };
            self.colorize(g, color, r2);
            g.glx.push_matrix();
            g.glx.translate(-0.5, 0.5, 0.0);
            g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            g.glx.rotate(-90.0, 0.0, 0.0, 1.0);
            g.glx.rotate(-r2 * 90.0, 0.0, 1.0, 0.0);
            g.glx.translate(0.5, -0.5, 0.0);
            self.draw_face(g);
            g.glx.pop_matrix();
        }

        if state >= 2.0 {
            /* Right */
            let r2 = if istate == 2 { r } else { 1.0 };
            self.colorize(g, color, r2);
            g.glx.push_matrix();
            g.glx.translate(0.5, 0.5, 0.0);
            g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            g.glx.rotate(-90.0, 0.0, 0.0, 1.0);
            g.glx.rotate(-90.0, 0.0, 1.0, 0.0);
            g.glx.rotate(-r2 * 90.0, 0.0, 1.0, 0.0);
            g.glx.translate(-0.5, -0.5, 0.0);
            self.draw_face(g);
            g.glx.pop_matrix();
        }

        if state >= 3.0 {
            /* Front */
            let r2 = if istate == 3 { r } else { 1.0 };
            self.colorize(g, color, r2);
            g.glx.push_matrix();
            g.glx.translate(0.5, 0.5, 0.0);
            g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            g.glx.rotate(-90.0, 0.0, 0.0, 1.0);
            g.glx.rotate(-180.0, 0.0, 1.0, 0.0);
            g.glx.translate(-1.0, 0.0, 0.0);
            g.glx.rotate(-r2 * 90.0, 0.0, 1.0, 0.0);
            g.glx.translate(0.5, -0.5, 0.0);
            self.draw_face(g);
            g.glx.pop_matrix();
        }

        if state >= 4.0 {
            /* Top */
            let r2 = if istate == 4 { r } else { 1.0 };
            self.colorize(g, color, r2);
            g.glx.push_matrix();
            g.glx.translate(0.0, 0.0, 1.0);
            g.glx.rotate(-90.0, 0.0, 0.0, 1.0);
            g.glx.translate(0.5, 0.5, 0.0);
            g.glx.rotate(-90.0, 0.0, 1.0, 0.0);
            g.glx.rotate(r2 * 90.0, 0.0, 1.0, 0.0);
            g.glx.translate(-0.5, -0.5, 0.0);
            self.draw_face(g);
            g.glx.pop_matrix();
        }
    }

    /// The whole stack: the finished cubes below, then the one being built.
    fn draw_cubes(&self, g: &mut Gl) {
        let z = self.state / STATES;
        let n = self.colors.len();
        let c0 = self.ccolor;

        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -1.5 - z);
        g.glx.translate(0.0, 0.0, -self.length as f32);

        for i in (0..self.length).rev() {
            // Each cube down the stack is one step back around the colormap.
            let c1 = (c0 + n - ((i as usize + 1) % n)) % n;
            let c = &self.colors[c1];
            let color = [
                f32::from(c.red) / 65536.0,
                f32::from(c.green) / 65536.0,
                f32::from(c.blue) / 65536.0,
                self.opacity,
            ];
            g.glx.translate(0.0, 0.0, 1.0);
            self.draw_cube_1(g, 5.0, color, i == self.length - 1);
        }

        let c = &self.colors[c0.min(n - 1)];
        let color = [
            f32::from(c.red) / 65536.0,
            f32::from(c.green) / 65536.0,
            f32::from(c.blue) / 65536.0,
            self.opacity,
        ];
        g.glx.translate(0.0, 0.0, 1.0);
        self.draw_cube_1(g, self.state, color, self.length == 0);
        g.glx.pop_matrix();
    }
}

impl Hack3d for CubeStack {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        if !self.wireframe {
            g.glx.lighting(false);
            g.glx.cull_face(false);
            // Depth testing off and additive blending on: every cube in the
            // stack shows through every other, which is the whole look.
            g.glx.depth_test(false);
            g.glx.blend(Blend::AlphaAdd);
        }

        g.glx.push_matrix();
        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 4.0,
            (y as f32 - 0.5) * 4.0,
            (z as f32 - 0.5) * 2.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        g.glx.scale(6.0, 6.0, 6.0);
        g.glx.rotate(-45.0, 1.0, 0.0, 0.0);
        g.glx.rotate(20.0, 0.0, 0.0, 1.0);
        g.glx.rotate(self.r, 0.0, 0.0, 1.0);

        self.draw_cubes(g);
        g.glx.pop_matrix();

        if !down {
            self.state += self.speed * 0.015;
            self.r += self.speed * 0.05;
            while self.r > 360.0 {
                self.r -= 360.0;
            }
            while self.state > STATES {
                self.state -= STATES;
                self.length += 1;
                self.ccolor = (self.ccolor + 1) % self.colors.len().max(1);
            }
            if self.length > MAX_LENGTH {
                self.length = MAX_LENGTH;
            }
        }

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width;
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
        if let XEvent::KeyPress { key } = event
            && (*key == ' ' || *key == '\t')
        {
            self.colors = make_smooth_colormap(32);
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wander_speed = 0.005;
    let mut st = CubeStack {
        rot: Rotator::new(
            0.0,
            0.0,
            0.0,
            0.0,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            false,
        ),
        trackball: Trackball::new(),
        /* one step of fading in before the first face swings */
        state: -1.0,
        r: 0.0,
        length: 0,
        colors: make_smooth_colormap(32),
        ccolor: 0,
        speed: g.res.float("speed") as f32,
        thickness: g.res.float("thickness").clamp(0.001, 0.5) as f32,
        opacity: g.res.float("opacity") as f32,
        wireframe: g.res.bool("wireframe"),
    };
    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*wander:       True",
    "*speed:        1.0",
    "*thickness:    0.13",
    "*opacity:      0.7",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Animation speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("thickness", "Thickness", 0.0, 0.5, 0.01, 3, "0.13"),
    Opt::slider("opacity", "Opacity", 0.01, 1.0, 0.01, 2, "0.7"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cubestack",
    label: "Cube Stack",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2016",
        video: Some("https://www.youtube.com/watch?v=rZi5yav6sRo"),
        blurb: "An endless stack of unfolding, translucent cubes.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
