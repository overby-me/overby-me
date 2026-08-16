//! Port of `hacks/glx/dangerball.c`.
//!
//! ```text
//! dangerball, Copyright (c) 2001-2017 Jamie Zawinski <jwz@jwz.org>
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
//! A ball with spikes, which grow out of it, stop, and pull back in, and then
//! grow again somewhere else. Ouch.
//!
//! It is a sphere and thirty cones, and all of the character is in two lines.
//! The first is how the spikes grow: `pos` runs from 0 up to 1 and then jumps
//! to -1 and runs back up to 0, so the same one addition drives both the growth
//! and the retraction, and the moment of reversal is a hard flip at the apex
//! rather than a turn. The second is what the length is a function of:
//!
//! ```text
//! pos = (asin (0.5 + pos/2) - 0.5) * 2
//! ```
//!
//! which is an arcsine over its steep end, so a spike shoots out and then
//! slows into its full length rather than arriving at a constant rate. It also
//! never quite reaches zero, which is why the ball keeps a stubble of points
//! rather than going smooth between rounds.
//!
//! Where the spikes go is quantised: a random latitude and longitude, each
//! rounded down to a multiple of 22 degrees. Thirty of them over sixteen by
//! eight positions means collisions, and the doubled-up spikes are part of the
//! look rather than a bug in it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::tube::cone;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    random_below, unit_sphere,
};

/// How densely to render spikes.
const SPIKE_FACES: i32 = 12;
const SMOOTH_SPIKES: bool = true;
/// How densely to render spheres.
const SPHERE_SLICES: i32 = 32;
const SPHERE_STACKS: i32 = 16;

/// The spike positions are rounded to multiples of this many degrees, which is
/// what makes them look placed rather than sprinkled.
const ROT_SCALE: i32 = 22;

struct Ball {
    rot: Rotator,
    trackball: Trackball,

    ball_list: u32,
    spike_list: u32,

    /// How far out the spikes are: 0 to 1 growing, -1 to 0 pulling back in.
    pos: f32,
    /// Two angles per spike, in degrees: about y, then about z.
    spikes: Vec<i32>,

    colors: Vec<XColor>,
    ccolor: usize,
    /// How far round the colormap the spikes are from the ball. Usually none,
    /// so they usually match.
    color_shift: usize,

    count: usize,
    speed: f32,
    wireframe: bool,
}

impl Ball {
    fn randomize_spikes(&mut self) {
        self.pos = 0.0;
        for i in 0..self.count {
            self.spikes[i * 2] = random_below(360) - 180;
            self.spikes[i * 2 + 1] = random_below(180) - 90;
        }
        for s in &mut self.spikes {
            *s = (*s / ROT_SCALE) * ROT_SCALE;
        }

        let n = self.colors.len();
        self.color_shift = if random().is_multiple_of(3) && n >= 2 {
            random() as usize % (n / 2)
        } else {
            0
        };
    }

    fn draw_spikes(&self, g: &mut Gl) {
        let diam = 0.2;
        let mut pos = self.pos;
        if pos < 0.0 {
            pos = -pos;
        }
        pos = ((0.5 + pos / 2.0).asin() - 0.5) * 2.0;

        for i in 0..self.count {
            g.glx.push_matrix();
            g.glx.rotate(self.spikes[i * 2] as f32, 0.0, 1.0, 0.0);
            g.glx.rotate(self.spikes[i * 2 + 1] as f32, 0.0, 0.0, 1.0);
            g.glx.translate(0.7, 0.0, 0.0);
            g.glx.rotate(-90.0, 0.0, 0.0, 1.0);
            g.glx.scale(diam, pos, diam);
            g.glx.call_list(self.spike_list);
            g.glx.pop_matrix();
        }
    }

    fn move_spikes(&mut self) {
        if self.pos >= 0.0 {
            /* moving outward */
            self.pos += self.speed;
            if self.pos >= 1.0 {
                /* reverse gears at apex */
                self.pos = -1.0;
            }
        } else {
            /* moving inward */
            self.pos += self.speed;
            if self.pos >= 0.0 {
                /* stop at end */
                self.randomize_spikes();
            }
        }
    }
}

impl Hack3d for Ball {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
        }
        g.glx.clear();

        g.glx.push_matrix();
        g.glx.scale(1.1, 1.1, 1.1);

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 8.0,
            (z as f32 - 0.5) * 15.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        let n = self.colors.len();
        let c = &self.colors[self.ccolor.min(n - 1)];
        let bcolor = [
            f32::from(c.red) / 65536.0,
            f32::from(c.green) / 65536.0,
            f32::from(c.blue) / 65536.0,
            1.0,
        ];
        let c = &self.colors[(self.ccolor + self.color_shift) % n];
        let scolor = [
            f32::from(c.red) / 65536.0,
            f32::from(c.green) / 65536.0,
            f32::from(c.blue) / 65536.0,
            1.0,
        ];
        self.ccolor = (self.ccolor + 1) % n;

        g.glx.scale(2.0, 2.0, 2.0);

        self.move_spikes();

        if self.wireframe {
            g.glx.color4f(bcolor[0], bcolor[1], bcolor[2], 1.0);
        } else {
            // The ball is glossy and the spikes are matt, which is the whole of
            // the difference between them: same colour, same light, and one
            // reads as polished and the other as bone.
            g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
            g.glx.material_shininess(128.0);
            g.glx.material_ambient_diffuse(bcolor);
        }
        g.glx.call_list(self.ball_list);

        if self.wireframe {
            g.glx.color4f(scolor[0], scolor[1], scolor[2], 1.0);
        } else {
            g.glx.material_specular([0.0, 0.0, 0.0, 1.0]);
            g.glx.material_shininess(0.0);
            g.glx.material_ambient_diffuse(scolor);
        }
        self.draw_spikes(g);
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
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let spin = g.res.bool("spin");
    let wander = g.res.bool("wander");
    let count = g.res.int("count").clamp(1, 100) as usize;

    let spin_speed = 10.0;
    let wander_speed = 0.12;
    let spin_accel = 2.0;

    let mut st = Ball {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            spin_accel,
            if wander { wander_speed } else { 0.0 },
            true,
        ),
        trackball: Trackball::new(),
        ball_list: 0,
        spike_list: 0,
        pos: 0.0,
        spikes: vec![0; count * 2],
        colors: make_smooth_colormap(128),
        ccolor: 0,
        color_shift: 0,
        count,
        speed: g.res.float("speed") as f32,
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire {
        // Set after the reshape, so the light stays where the camera is rather
        // than turning with the ball. The specular colour is cyan, which is why
        // the highlight on the ball is not the colour of anything else in the
        // picture.
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
    }

    st.ball_list = g.glx.gen_lists(1);
    g.glx.new_list(st.ball_list);
    unit_sphere(&mut g.glx, SPHERE_STACKS, SPHERE_SLICES, wire);
    g.glx.end_list();

    st.spike_list = g.glx.gen_lists(1);
    g.glx.new_list(st.spike_list);
    cone(
        &mut g.glx,
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        1.0,
        0.0,
        SPIKE_FACES,
        SMOOTH_SPIKES,
        false,
        wire,
    );
    g.glx.end_list();

    st.randomize_spikes();
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        30",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        0.05",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Spike growth", 0.001, 0.25, 0.001, 3, "0.05"),
    Opt::slider("count", "Number of spikes", 1.0, 100.0, 1.0, 0, "30"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "dangerball",
    label: "Danger Ball",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=QU0aPwWwHbg"),
        blurb: "A spiky ball. Ouch!",
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
    use crate::runtime::StartArgs;

    /// The spikes are the point of it. Each one is its own call of the cone
    /// list under its own matrix, so a frame has to hold a batch per spike as
    /// well as the ball's.
    #[test]
    fn there_are_spikes() {
        let mut r = start(StartArgs::new(640, 480, "count=30", 20260811));
        for _ in 0..10 {
            r.step();
        }
        assert!(
            r.frame().batches.len() > 30,
            "{} batches, so the spikes are missing",
            r.frame().batches.len()
        );
    }

    /// And they reach outside the ball: a spike that never leaves the sphere it
    /// grows from is invisible, which is what the arcsine is there to prevent.
    #[test]
    fn the_spikes_come_out_of_the_ball() {
        let mut r = start(StartArgs::new(640, 480, "count=30", 20260811));
        let mut out = 0;
        for _ in 0..40 {
            r.step();
            let f = r.frame();
            // The ball is the first batch, and its matrix is the one the
            // spikes hang off, so anything further than 1 from that origin in
            // its own units is outside the sphere.
            let Some(ball) = f.batches.first() else {
                continue;
            };
            // Both are in eye space, where the ball's radius is the 2.2 the
            // saver scaled it up by.
            let o = ball.modelview.transform([0.0, 0.0, 0.0]);
            for b in &f.batches[1..] {
                for v in &f.vertices[b.first..b.first + b.count] {
                    let p = b.modelview.transform(v.pos);
                    let d = ((p[0] - o[0]).powi(2) + (p[1] - o[1]).powi(2) + (p[2] - o[2]).powi(2))
                        .sqrt();
                    if d > 2.3 {
                        out += 1;
                    }
                }
            }
        }
        assert!(out > 100, "only {out} spike vertices left the ball");
    }
}
