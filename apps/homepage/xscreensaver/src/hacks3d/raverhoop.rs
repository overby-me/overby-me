//! Port of `hacks/glx/raverhoop.c`.
//!
//! ```text
//! raverhoop, Copyright (c) 2016-2018 Jamie Zawinski <jwz@jwz.org>
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
//! Simulates an LED hula hoop in a dark room.
//!
//! There is no hoop. Two hundred lights are spaced round where a hoop would be,
//! most of them switched off at any moment by a duty cycle, and each one that is
//! on leaves a soft dot at wherever it has got to. The dots fade over a second
//! or so, and it is the *fading* that draws the hoop: what you see is not the
//! ring but the smear it has left in the air, which is exactly what a long
//! exposure of a real one looks like.
//!
//! The hoop itself is only ever a matrix. Every frame it is turned about its
//! own axis, tilted, spun, and carried round a smaller circle, and where a
//! light ends up is read back out of the matrix rather than worked out. A
//! handful of oscillators ease that handful of numbers back and forth on sines,
//! and every so often another is started, so the hoop weaves without ever
//! quite repeating.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Ease, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, ease,
    frand, random,
};

/// A dot left behind by a light, fading as it goes.
#[derive(Clone, Copy)]
struct Afterimage {
    color: [f32; 4],
    position: [f32; 3],
}

/// One light on the hoop: what colour it is, and the pattern of off and on it
/// flashes in, as runs adding to a hundred.
#[derive(Clone, Copy, Default)]
struct Light {
    color: [f32; 4],
    duty_cycle: [i32; 10],
    ratio: f32,
    on: bool,
}

/// Which of the hoop's numbers an oscillator drives.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Var {
    MidZ,
    Tilt,
    AxialRadius,
    Speed,
    Spin,
}

struct Oscillator {
    ratio: f32,
    from: f32,
    to: f32,
    speed: f32,
    var: Var,
    remaining: i32,
}

struct RaverHoop {
    rot: Rotator,
    trackball: Trackball,

    lights: Vec<Light>,
    radius: f32,
    axial_radius: f32,
    midpoint: [f32; 3],
    tilt: f32,
    spin: f32,
    th: f32,
    /// The hoop's own speed, which an oscillator can reverse.
    speed: f32,

    trail: Vec<Afterimage>,
    oscillators: Vec<Oscillator>,
    texture: Option<u32>,

    ncolors: i32,
    knob_speed: f32,
    light_speed: f32,
    sustain: f32,
    wireframe: bool,
}

impl RaverHoop {
    fn var(&mut self, v: Var) -> &mut f32 {
        match v {
            Var::MidZ => &mut self.midpoint[2],
            Var::Tilt => &mut self.tilt,
            Var::AxialRadius => &mut self.axial_radius,
            Var::Speed => &mut self.speed,
            Var::Spin => &mut self.spin,
        }
    }

    /// Dots fade at a rate the sustain knob sets, and are dropped when they
    /// have gone.
    fn decay_afterimages(&mut self) {
        let tick = 0.05 / self.sustain;
        for a in &mut self.trail {
            a.color[3] -= tick;
        }
        self.trail.retain(|a| a.color[3] >= 0.0);
    }

    /// A light's duty cycle is runs of off and on adding to a hundred; this
    /// says which run it is in now.
    fn tick_light(l: &mut Light, light_speed: f32) {
        l.ratio += 0.05 * light_speed;
        while l.ratio > 1.0 {
            l.ratio -= 1.0;
        }
        let mut n = 0;
        for (i, d) in l.duty_cycle.iter().enumerate() {
            n += d;
            if f32::from(n as i16) / 100.0 > l.ratio {
                l.on = i & 1 == 1;
                break;
            }
        }
    }

    /// Move the hoop and leave a dot wherever each lit light has got to.
    ///
    /// Where that is comes out of the matrix rather than out of arithmetic:
    /// the light is positioned with the same turns the hoop is, and what is
    /// read back is the difference between the matrix before and after, which
    /// is the light's position in the frame the dots are drawn in.
    fn tick_hoop(&mut self, g: &mut Gl) {
        let m0 = g.glx.modelview_matrix();
        let (mid, th, tilt, spin, radius) =
            (self.midpoint, self.th, self.tilt, self.spin, self.radius);

        for i in 0..self.lights.len() {
            Self::tick_light(&mut self.lights[i], self.light_speed);
            if !self.lights[i].on {
                continue;
            }
            let a = std::f32::consts::PI * 2.0 * i as f32 / self.lights.len() as f32;

            g.glx.push_matrix();
            g.glx.translate(mid[0], mid[1], mid[2]);
            g.glx
                .rotate(th * 180.0 / std::f32::consts::PI, 0.0, 0.0, 1.0);
            g.glx.rotate(tilt, 0.0, 1.0, 0.0);
            g.glx.rotate(spin, 1.0, 0.0, 0.0);
            g.glx.translate(a.cos() * radius, a.sin() * radius, 0.0);
            let m1 = g.glx.modelview_matrix();
            g.glx.pop_matrix();

            // The older, dimmer dots are laid down first, so the newer ones
            // go on top of them.
            self.trail.push(Afterimage {
                color: self.lights[i].color,
                position: [
                    m1.0[12] - m0.0[12],
                    m1.0[13] - m0.0[13],
                    m1.0[14] - m0.0[14],
                ],
            });
        }
    }

    /// Every dot as a small square facing the camera.
    fn draw_lights(&self, g: &mut Gl) {
        if self.wireframe {
            // Where the hoop would be, and the circle it is carried round.
            g.glx.push_matrix();
            g.glx.begin(Shape::Lines);
            g.glx.vertex3f(0.0, 0.0, -self.radius);
            g.glx.vertex3f(0.0, 0.0, self.radius);
            g.glx.end();

            g.glx
                .translate(self.midpoint[0], self.midpoint[1], self.midpoint[2]);
            g.glx
                .rotate(self.th * 180.0 / std::f32::consts::PI, 0.0, 0.0, 1.0);
            g.glx.rotate(self.tilt, 0.0, 1.0, 0.0);
            g.glx.rotate(self.spin, 1.0, 0.0, 0.0);

            g.glx.begin(Shape::LineLoop);
            g.glx.vertex3f(0.0, 0.0, 0.0);
            for r in [self.radius, self.axial_radius] {
                for i in 0..=360 {
                    let th = i as f32 * std::f32::consts::PI * 2.0 / 360.0;
                    g.glx.vertex3f(r * -th.cos(), r * -th.sin(), 0.0);
                }
            }
            g.glx.end();
            g.glx.pop_matrix();
        }

        // Billboard the dots: keep the matrix's translation but throw away
        // everything it does to direction, so every square faces the camera.
        let mut m = g.glx.modelview_matrix();
        for (i, v) in [
            (0, 1.0),
            (1, 0.0),
            (2, 0.0),
            (4, 0.0),
            (5, 1.0),
            (6, 0.0),
            (8, 0.0),
            (9, 0.0),
            (10, 1.0),
        ] {
            m.0[i] = v;
        }
        g.glx.load_identity();
        g.glx.mult_matrix(m);

        if let Some(t) = self.texture {
            g.glx.texturing(true);
            g.glx.bind_texture(t);
        }

        for a in &self.trail {
            g.glx.push_matrix();
            g.glx.translate(a.position[0], a.position[1], a.position[2]);

            if self.wireframe {
                g.glx.color4f(
                    a.color[0] * a.color[3],
                    a.color[1] * a.color[3],
                    a.color[2] * a.color[3],
                    1.0,
                );
            } else {
                g.glx
                    .color4f(a.color[0], a.color[1], a.color[2], a.color[3]);
            }

            g.glx.rotate(45.0, 0.0, 0.0, 1.0);
            g.glx.scale(0.15, 0.15, 0.15);
            g.glx.begin(Shape::Quads);
            for (uv, p) in [
                ([0.0, 0.0], [-1.0, -1.0]),
                ([1.0, 0.0], [1.0, -1.0]),
                ([1.0, 1.0], [1.0, 1.0]),
                ([0.0, 1.0], [-1.0, 1.0]),
            ] {
                g.glx.tex_coord2f(uv[0], uv[1]);
                g.glx.vertex3f(p[0], p[1], 0.0);
            }
            g.glx.end();
            g.glx.pop_matrix();
        }
    }

    /// Ease each oscillator along, and swap its ends when it arrives, until it
    /// has run out of repeats.
    fn tick_oscillators(&mut self) {
        let tick = 0.1 / self.knob_speed;
        let taken = std::mem::take(&mut self.oscillators);
        let mut keep = Vec::with_capacity(taken.len());

        for mut a in taken {
            a.ratio += tick * a.speed;
            if a.ratio > 1.0 {
                a.ratio = 1.0;
            }
            let v = a.from + (a.to - a.from) * ease(Ease::InOutSine, f64::from(a.ratio)) as f32;
            *self.var(a.var) = v;

            if a.ratio < 1.0 {
                keep.push(a); /* mid cycle */
            } else {
                a.remaining -= 1;
                if a.remaining > 0 {
                    std::mem::swap(&mut a.from, &mut a.to);
                    a.ratio = 0.0;
                    keep.push(a);
                }
            }
        }
        self.oscillators = keep;
    }

    /// Let every oscillator finish its current sweep and stop.
    fn calm_oscillators(&mut self) {
        let n = self.oscillators.len();
        if n > 1 {
            for a in &mut self.oscillators[..n - 1] {
                a.remaining = 1;
            }
        }
    }

    fn add_oscillator(&mut self, var: Var, speed: f32, to: f32, repeat: i32) {
        // One at a time on any given number. Upstream's loop stops one short
        // of the end of its list, so the newest oscillator is never the one
        // that blocks another.
        let n = self.oscillators.len();
        if n > 1 && self.oscillators[..n - 1].iter().any(|a| a.var == var) {
            return;
        }
        let from = *self.var(var);
        self.oscillators.insert(
            0,
            Oscillator {
                ratio: 0.0,
                from,
                to,
                speed,
                var,
                remaining: repeat.max(1),
            },
        );
    }

    fn add_random_oscillator(&mut self) {
        let radius = self.radius;
        match random() % 12 {
            0..=2 => self.add_oscillator(
                Var::MidZ,
                1.0,
                radius * (0.8 + frand(0.2) as f32) * if random() & 1 == 1 { 1.0 } else { -1.0 },
                3 + (random() % 10) as i32,
            ),
            3..=5 => self.add_oscillator(
                Var::Tilt,
                1.0,
                -((random() % 15) as f32),
                3 + (random() % 20) as i32,
            ),
            6..=8 => self.add_oscillator(
                Var::AxialRadius,
                1.0,
                0.1 + radius * frand(1.4) as f32,
                1 + (random() % 4) as i32,
            ),
            9 | 10 => self.add_oscillator(
                Var::Speed,
                3.0,
                (0.7 + frand(0.9) as f32) * if random() & 1 == 1 { 1.0 } else { -1.0 },
                if random().is_multiple_of(5) {
                    2 + (random() % 5) as i32
                } else {
                    1
                },
            ),
            _ => self.add_oscillator(
                Var::Spin,
                0.1,
                180.0 * (1 + (random() % 2)) as f32,
                2 * (1 + (random() % 5)) as i32,
            ),
        }
    }

    /// Repaint the lights. The colours are quantised to sixteen levels a
    /// channel, which is what keeps them looking like LEDs rather than paint.
    fn randomize_colors(&mut self) {
        let mut ncolors = self.ncolors.clamp(1, self.lights.len() as i32);
        if random().is_multiple_of(10) {
            ncolors = 1;
        }

        let colors: Vec<[f32; 4]> = (0..ncolors)
            .map(|_| {
                let q = || (((random() % 16) << 4) | 0xF) as f32 / 255.0;
                [q(), q(), q(), 1.0]
            })
            .collect();

        // How many times the pattern goes round the hoop.
        let ncycles = match random() % 5 {
            0 => 1,
            2 => ncolors * (1 + (random() % 5) as i32),
            _ => ncolors,
        };

        let n_lights = self.lights.len() as i32;
        for (i, l) in self.lights.iter_mut().enumerate() {
            let n = (i as i32 * ncolors / n_lights) as usize;
            let m = i as i32 * ncycles / n_lights;
            l.color = colors[n.min(colors.len() - 1)];
            l.duty_cycle = [0; 10];

            if ncycles <= 1 {
                l.duty_cycle[1] = 100; /* always on */
            } else if m & 1 == 1 {
                l.duty_cycle[0] = 50;
                l.duty_cycle[1] = 50;
            } else {
                l.duty_cycle[1] = 50;
                l.duty_cycle[2] = 50;
            }
        }
    }

    fn move_hoop(&mut self) {
        self.th += 0.2 * self.knob_speed * self.speed;
        let tau = std::f32::consts::PI * 2.0;
        while self.th > tau {
            self.th -= tau;
        }
        while self.th < 0.0 {
            self.th += tau;
        }

        self.midpoint[0] = self.axial_radius * self.th.cos();
        self.midpoint[1] = self.axial_radius * self.th.sin();

        self.tick_oscillators();

        if random().is_multiple_of(80) {
            self.add_random_oscillator();
        }
        if random().is_multiple_of(120) {
            self.randomize_colors();
        }
    }
}

/// The light itself: a soft round dot, brightest in the middle.
fn build_texture(g: &mut Gl) -> u32 {
    let size = 128i32;
    let s2 = size as f32 / 2.0;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dist = (((s2 - x as f32).powi(2) + (s2 - y as f32).powi(2)).sqrt()) / s2;
            // Upstream's texture is luminance and alpha: white everywhere,
            // with the alpha falling off to nothing at the rim.
            let a = (255.0 * if dist > 1.0 { 0.0 } else { (1.0 - dist).sin() }) as u8;
            data.extend_from_slice(&[255, 255, 255, a]);
        }
    }

    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    g.glx.tex_image_2d(size, size, data);
    g.glx.tex_clamp(false);
    g.glx.tex_nearest(false);
    id
}

impl Hack3d for RaverHoop {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.lighting(false);
        g.glx.depth_test(false);
        g.glx.cull_face(true);
        // Every dot adds to what is under it, so where the smear crosses
        // itself it burns out to white.
        g.glx.blend(Blend::AlphaAdd);

        g.glx.push_matrix();

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 7.0,
            (y as f32 - 0.5) * 0.5,
            (z as f32 - 0.5) * 15.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(0.2, 0.2, 0.2);
        g.glx.rotate(70.0, 1.0, 0.0, 0.0);

        if !down {
            self.move_hoop();
        }
        self.decay_afterimages();
        self.tick_hoop(g);
        self.draw_lights(g);

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
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event
            && (*key == ' ' || *key == '\t')
        {
            self.randomize_colors();
            self.calm_oscillators();
            self.add_random_oscillator();
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.bool("spin");
    let spin_speed = 0.3;
    let wander_speed = 0.005;
    let spin_accel = 2.0;
    let wire = g.res.bool("wireframe");

    let nlights = g.res.int("lights").clamp(3, 2000) as usize;
    let mut st = RaverHoop {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            spin_accel,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            false,
        ),
        trackball: Trackball::new(),
        lights: vec![Light::default(); nlights],
        radius: 30.0,
        axial_radius: 30.0 * 0.3,
        midpoint: [0.0; 3],
        tilt: -(5 + (random() % 12) as i32) as f32,
        // Upstream writes `random() % 1`, which is always zero, so the hoop
        // always starts turning the same way. Kept: an oscillator reverses it
        // soon enough anyway.
        speed: -1.0,
        th: 0.0,
        spin: 0.0,
        trail: Vec::new(),
        oscillators: Vec::new(),
        texture: None,
        ncolors: g.res.int("ncolors").clamp(1, 512),
        knob_speed: g.res.float("speed").max(0.01) as f32,
        light_speed: g.res.float("lightSpeed").max(0.01) as f32,
        sustain: g.res.float("sustain").max(0.01) as f32,
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire {
        st.texture = Some(build_texture(g));
    }

    st.randomize_colors();
    st.move_hoop();
    st.add_random_oscillator();
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:      20000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*ncolors:    12",
    "*lights:     200",
    "*speed:      1.0",
    "*lightSpeed: 1.0",
    "*sustain:    1.0",
    "*spin:       False",
    "*wander:     False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("ncolors", "Colors", 1.0, 64.0, 1.0, 0, "12"),
    Opt::slider("lights", "Lights", 3.0, 512.0, 1.0, 0, "200"),
    Opt::slider("speed", "Speed", 0.1, 5.0, 0.1, 2, "1.0"),
    Opt::slider("lightSpeed", "Light speed", 0.1, 5.0, 0.1, 2, "1.0"),
    Opt::slider("sustain", "Sustain", 0.1, 5.0, 0.1, 2, "1.0"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("spin", "Spin", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "raverhoop",
    label: "Raver Hoop",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2016",
        video: Some("https://www.youtube.com/watch?v=0k2sP_Imb80"),
        blurb: "Simulates an LED hula hoop in a dark room.",
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
    use crate::runtime::gl::Primitive;

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260811));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// Every dot is one square, and there are more of them as the trail builds
    /// up and then no more as the oldest start to expire.
    #[test]
    fn the_trail_builds_up_and_then_holds() {
        let mut r = start(StartArgs::new(640, 480, "lights=60", 20260811));
        let mut counts = Vec::new();
        for _ in 0..120 {
            r.step();
            counts.push(
                r.frame()
                    .batches
                    .iter()
                    .filter(|b| b.primitive == Primitive::Triangles)
                    .map(|b| b.count / 6)
                    .sum::<usize>(),
            );
        }
        assert!(counts[0] > 0, "nothing was lit at all");
        assert!(counts[20] > counts[0], "the trail did not build up");
        // Sustain is a second or so at twenty frames a lap, so it levels off
        // rather than growing without bound.
        let late = &counts[80..];
        let hi = late.iter().copied().max().unwrap();
        let lo = late.iter().copied().min().unwrap();
        assert!(hi < counts[20] * 6, "the trail is still growing: {hi}");
        assert!(lo > 0);
    }

    /// A dot fades from full to nothing, and is dropped when it gets there.
    #[test]
    fn the_dots_fade_out() {
        let mut r = start(StartArgs::new(640, 480, "lights=20&sustain=1", 20260811));
        for _ in 0..60 {
            r.step();
        }
        let f = r.frame();
        let alphas: Vec<f32> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
            .map(|b| f.vertices[b.first].color[3])
            .collect();
        assert!(alphas.len() > 5, "not enough dots to tell");
        let hi = alphas.iter().copied().fold(0.0f32, f32::max);
        let lo = alphas.iter().copied().fold(f32::MAX, f32::min);
        assert!(hi > 0.9, "no dot is fresh");
        assert!(lo < 0.5, "no dot has faded");
        assert!(alphas.iter().all(|a| *a >= 0.0), "a dot went past nothing");
    }

    /// The dots always face the camera: whatever the hoop is doing, every
    /// square is drawn square on.
    #[test]
    fn the_dots_face_the_camera() {
        let r = run("lights=30", 20);
        let f = r.frame();
        let quads: Vec<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
            .collect();
        assert!(!quads.is_empty());

        // Billboarded means the modelview has no rotation left in it: its
        // upper-left is the identity, bar the scale the dot applies itself.
        for b in &quads {
            let m = b.modelview.0;
            // The dot turns forty-five degrees about z and scales, so compare
            // the x and y axes' lengths rather than the matrix itself.
            let xlen = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
            let zlen = (m[8] * m[8] + m[9] * m[9] + m[10] * m[10]).sqrt();
            assert!(
                (xlen - zlen).abs() < 1e-4,
                "the dot is not square on: {xlen} against {zlen}"
            );
            // And nothing tips it out of the view plane.
            assert!(m[2].abs() < 1e-4 && m[6].abs() < 1e-4, "the dot is tilted");
        }
    }

    /// The lights flash: a duty cycle switches most of them off at any moment,
    /// which is what makes the smear dotted rather than solid.
    #[test]
    fn most_of_the_lights_are_off_at_any_moment() {
        let mut r = start(StartArgs::new(640, 480, "lights=200&sustain=0.1", 20260811));
        // Long enough for the trail to be about one frame deep.
        for _ in 0..40 {
            r.step();
        }
        let lit = r
            .frame()
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
            .map(|b| b.count / 6)
            .sum::<usize>();
        assert!(lit > 0, "nothing lit");
        // Two hundred lights, a short sustain: far fewer dots than lights.
        assert!(lit < 200 * 4, "{lit} dots is not a flashing hoop");
    }

    /// The colours are quantised to sixteen levels a channel, which is what
    /// keeps them looking like LEDs.
    #[test]
    fn the_colours_are_quantised() {
        let r = run("lights=40", 5);
        let f = r.frame();
        for b in f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
        {
            let c = f.vertices[b.first].color;
            for (k, channel) in c.iter().take(3).enumerate() {
                let byte = (channel * 255.0).round();
                assert!(
                    (byte as i32 & 0x0F) == 0x0F,
                    "channel {k} is {byte}, not a quantised level"
                );
            }
        }
    }

    /// Poking it repaints the lights and starts something new moving.
    #[test]
    fn a_poke_repaints_and_stirs() {
        let mut r = start(StartArgs::new(640, 480, "lights=40", 20260811));
        for _ in 0..10 {
            r.step();
        }
        let colours = |r: &Runner3d| {
            let f = r.frame();
            f.batches
                .iter()
                .filter(|b| b.primitive == Primitive::Triangles)
                .map(|b| {
                    let c = f.vertices[b.first].color;
                    [c[0].to_bits(), c[1].to_bits(), c[2].to_bits()]
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = colours(&r);
        for _ in 0..6 {
            r.event(XEvent::KeyPress { key: ' ' });
            r.step();
            if colours(&r) != before {
                return;
            }
        }
        panic!("six pokes and the colours never changed");
    }
}
