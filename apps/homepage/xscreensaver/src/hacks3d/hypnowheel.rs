//! Port of `hacks/glx/hypnowheel.c`.
//!
//! ```text
//! hypnowheel, Copyright (c) 2008 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Draws overlapping spirals, where the tightness of the spirals changes.
//! Nice settings:
//!
//! -layers 7 -wander
//! -count 3 -layers 50
//! -twistiness 0.2 -layers 20
//! -count 3 -layers 2 -speed 20 -twist 10 -wander
//! ```
//!
//! Several transparent spiral discs, stacked and turning at slightly different
//! rates. Each disc is a pinwheel of `count` arms; the twist of the arms is
//! driven by one axis of that disc's own rotator, so an arm winds up tight,
//! unwinds through straight, and winds up the other way.
//!
//! The whole effect is the blending. Everything is drawn with `GL_ONE, GL_ONE`,
//! so where two discs overlap their colours add and go towards white, and the
//! moiré between two pinwheels at different twists is the picture. There is no
//! depth testing and no lighting: this is a stack of coloured light, not a
//! solid.
//!
//! Alternate discs twist in opposite directions, which is what makes the moiré
//! sweep across rather than sit still.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, XEvent, frand,
    screenhack_event_helper,
};

/// One layer of the stack: a spiral disc with its own colour, twist and spin.
struct Disc {
    color: usize,
    twist: f32,
    alpha: f32,
    rot: Rotator,
}

struct Hypnowheel {
    rot: Rotator,
    discs: Vec<Disc>,
    colors: Vec<XColor>,
    /// Arms per disc.
    count: i32,
    twistiness: f32,
    symmetric: bool,
    wireframe: bool,
}

impl Hypnowheel {
    /// `init_hypnowheel`, which is also what a keypress runs again.
    fn build(g: &mut Gl) -> Self {
        let speed = g.res.float("speed");
        let ncolors = 1024;
        let nlayers = g.res.int("layers").clamp(1, 50) as usize;
        let count = g.res.int("count").max(2);

        let discs = (0..nlayers)
            .map(|i| {
                let mut spin_speed = speed * 0.2;
                let mut wander_speed = speed * 0.0012;
                let spin_accel = 0.2;
                spin_speed += frand(spin_speed / 5.0);
                wander_speed += frand(wander_speed * 3.0);
                Disc {
                    twist: 0.0,
                    alpha: 1.0,
                    color: i * ncolors / nlayers,
                    rot: Rotator::new(
                        spin_speed,
                        spin_speed,
                        spin_speed,
                        spin_accel,
                        if g.res.bool("wander") {
                            wander_speed
                        } else {
                            0.0
                        },
                        true,
                    ),
                }
            })
            .collect();

        Hypnowheel {
            rot: Rotator::new(0.0, 0.0, 0.0, 0.0, speed * 0.0025, false),
            discs,
            colors: make_smooth_colormap(ncolors),
            count,
            twistiness: g.res.float("twistiness") as f32,
            symmetric: g.res.bool("symmetric"),
            wireframe: g.res.bool("wireframe"),
        }
    }

    /// One disc: `count` arms, each a quad strip spiralling out from the
    /// centre, wound by this disc's twist.
    fn draw_spiral(&self, g: &mut Gl, color: usize, twist: f32, alpha: f32) {
        let wire = self.wireframe;
        let rr = 0.5f32;
        let n = self.count;
        // More steps for fewer arms, since a wide arm needs more segments to
        // look like a curve.
        let steps = n * if wire {
            3
        } else if n < 5 {
            60
        } else if n < 10 {
            20
        } else {
            10
        };
        let dth = std::f32::consts::PI * 2.0 / n as f32;
        let dr = rr / steps as f32;
        let dtwist = std::f32::consts::PI * 2.0 * twist / steps as f32;

        // Upstream divides the colour down as the layers pile up, because with
        // additive blending a tall stack washes out to white.
        let mut cscale = 65536.0;
        let layers = self.discs.len() as f64;
        if layers > 3.0 && !wire {
            cscale *= layers - 2.0;
        }
        let c = &self.colors[color.min(self.colors.len() - 1)];
        g.glx.color4f(
            (f64::from(c.red) / cscale) as f32,
            (f64::from(c.green) / cscale) as f32,
            (f64::from(c.blue) / cscale) as f32,
            alpha,
        );

        let mut th = 0.0f32;
        while th < std::f32::consts::PI * 2.0 {
            let mut th1 = th;
            g.glx.begin(if wire {
                Shape::LineStrip
            } else {
                Shape::QuadStrip
            });
            let mut r = 0.0f32;
            while r <= rr {
                let th2 = th1 + dth / 2.0 + dtwist;
                th1 += dtwist;
                g.glx.vertex3f(r * th1.cos(), r * th1.sin(), 0.0);
                if !wire {
                    g.glx.vertex3f(r * th2.cos(), r * th2.sin(), 0.0);
                }
                r += dr;
            }
            g.glx.end();
            th += dth;
        }
    }
}

impl Hack3d for Hypnowheel {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.lighting(false);
        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.blend(if self.wireframe {
            Blend::Off
        } else {
            Blend::Add
        });

        g.glx.push_matrix();
        let (x, y, _) = self.rot.position(true);
        g.glx
            .translate((x as f32 - 0.5) * 8.0, (y as f32 - 0.5) * 8.0, 0.0);
        let (x, y, z) = self.rot.rotation(true);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(45.0, 45.0, 45.0);

        let ncolors = self.colors.len();
        for i in 0..self.discs.len() {
            // In symmetric mode a pair of discs shares one rotator, so they
            // move together and only their twists differ.
            let source = if self.symmetric { i & !0x1 } else { i };
            let tick = !self.symmetric || i == 0;

            g.glx.push_matrix();
            self.discs[i].color = (self.discs[i].color + 1) % ncolors.max(1);

            let (mut x, mut y, z) = self.discs[source].rot.position(tick);
            x = (x - 0.5) * 0.1;
            y = (y - 0.5) * 0.1;
            g.glx.translate(x as f32, y as f32, 0.0);
            self.discs[i].twist = z as f32 * self.twistiness * if i & 1 != 0 { 1.0 } else { -1.0 };

            let (_, _, z) = self.discs[source].rot.rotation(tick);
            g.glx.rotate(360.0 * z as f32, 0.0, 0.0, 1.0); /* rotation of this disc */

            let (color, twist, alpha) = {
                let d = &self.discs[i];
                (d.color, d.twist, d.alpha)
            };
            self.draw_spiral(g, color, twist, alpha);
            g.glx.pop_matrix();
        }
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 3 {
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

    /// Poking it starts the whole thing over with new colours and new spins,
    /// which is what upstream's calling `init_hypnowheel` again amounts to.
    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            *self = Hypnowheel::build(g);
            let (w, h) = (g.width(), g.height());
            self.reshape(g, w, h);
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut st = Hypnowheel::build(g);
    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*count:        13",
    "*showFPS:      False",
    "*fpsSolid:     True",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*wander:       False",
    "*symmetric:    False",
    "*speed:        1.0",
    "*twistiness:   4.0",
    "*layers:       4",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("count", "Arms", 2.0, 50.0, 1.0, 0, "13"),
    Opt::slider("layers", "Layers", 1.0, 50.0, 1.0, 0, "4"),
    Opt::slider("speed", "Speed", 0.1, 20.0, 0.1, 1, "1.0"),
    Opt::slider("twistiness", "Twistiness", 0.2, 10.0, 0.1, 1, "4.0"),
    Opt::boolean("symmetric", "Symmetric twisting", "false"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hypnowheel",
    label: "Hypnowheel",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2008",
        video: Some("https://www.youtube.com/watch?v=QcJnc9EKJrI"),
        blurb: "Overlapping, translucent spirals, twisting and untwisting.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
