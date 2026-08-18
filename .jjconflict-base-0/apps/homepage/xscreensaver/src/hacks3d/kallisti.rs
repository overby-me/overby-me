//! Port of `hacks/glx/kallisti.c`.
//!
//! ```text
//! kallisti, Copyright © 2023-2024 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! The golden apple of discord: https://www.jwz.org/blog/2023/09/ti-kallisti/
//! ```
//!
//! The apple Eris rolled into the wedding, inscribed *kallisti*, to the
//! fairest. It is one model of twenty-eight thousand vertices, and the whole
//! saver is that model turning under three lights: a white key light, a dull
//! red one from below, and a yellow one swinging round it, which is what makes
//! the gold move rather than just sit there.
//!
//! It fades up from black at the start rather than appearing, which is done by
//! scaling every light and the material with the same rising number.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gllist::GlList;
use crate::runtime::rotator::Rotator;
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent};

struct Kallisti {
    trackball: Trackball,
    rot: Rotator,
    list: u32,
    /// How far up the opening fade we are, from nought to one.
    tick: f32,
    /// Where the swinging light has got to.
    th: f32,
    aspect: f32,
    scale: f32,
    speed: f32,
    wire: bool,
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let speed = g.res.float("speed") as f32;
    let spin = g.res.string("spin").to_string();
    let (mut x, mut y, mut z) = (false, false, false);
    for c in spin.chars() {
        match c {
            'x' | 'X' => x = true,
            'y' | 'Y' => y = true,
            'z' | 'Z' => z = true,
            _ => {}
        }
    }
    let spin_speed = 0.6 * speed as f64;
    let wander_speed = 0.01 * speed as f64;

    let wire = g.res.bool("wireframe");
    let model = GlList::parse(crate::models::KALLISTI_MODEL);
    let list = g.glx.gen_lists(1);
    g.glx.new_list(list);
    model.render(&mut g.glx, wire);
    g.glx.end_list();

    let mut this = Kallisti {
        trackball: Trackball::new(),
        rot: Rotator::new(
            if x { spin_speed } else { 0.0 },
            if y { spin_speed } else { 0.0 },
            if z { spin_speed } else { 0.0 },
            0.3,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            false,
        ),
        list,
        tick: 0.0,
        th: 0.0,
        aspect: 1.0,
        scale: 1.0,
        speed,
        wire,
    };
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Kallisti {
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
        self.scale = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, self.aspect, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.lighting(!self.wire);
        g.glx.color_material(self.wire);

        if self.tick < 1.0 {
            self.tick = (self.tick + 0.01 * self.speed).min(1.0);
        }

        if !self.wire {
            // Everything is scaled by the fade, so the apple rises out of
            // black rather than appearing.
            let t = self.tick;
            let dim = |c: [f32; 4]| [c[0] * t, c[1] * t, c[2] * t, c[3]];

            g.glx.light_enable(0, true);
            g.glx.light_position(0, -30.0, 15.0, 3.0, 1.0);
            g.glx.light_ambient(0, dim([0.4, 0.4, 0.4, 1.0]));
            g.glx.light_diffuse(0, dim([1.0, 1.0, 1.0, 1.0]));

            g.glx.light_enable(1, true);
            g.glx.light_position(1, 24.0, -12.0, -12.0, 1.0);
            g.glx.light_ambient(1, dim([0.0, 0.0, 0.0, 1.0]));
            g.glx.light_diffuse(1, dim([0.7, 0.1, 0.1, 1.0]));

            // The third light swings round the apple, which is what makes the
            // gold move.
            let s = 10.0;
            let th2 = std::f32::consts::PI * 0.65;
            self.th += 0.02 * self.speed;
            while self.th > std::f32::consts::PI * 2.0 {
                self.th -= std::f32::consts::PI * 2.0;
            }
            g.glx.light_enable(2, true);
            g.glx.light_position(
                2,
                s * th2.cos() * (-self.th).cos(),
                s * (-self.th).sin(),
                s * th2.sin() * (-self.th).cos(),
                1.0,
            );
            g.glx.light_ambient(2, dim([0.0, 0.0, 0.0, 1.0]));
            g.glx.light_diffuse(2, dim([0.6, 0.6, 0.0, 1.0]));
        }

        g.glx.push_matrix();
        g.glx.scale(10.0, 10.0, 10.0);
        g.glx.rotate(15.0, 1.0, 0.0, 0.0);
        g.glx.translate(0.0, -0.3, 0.0);

        let turning = !self.trackball.button_down();
        let (x, y, z) = self.rot.position(turning);
        g.glx.translate(
            (x as f32 - 0.5) * 1.0,
            (y as f32 - 0.5) * 0.8,
            (z as f32 - 1.0) * 2.0,
        );
        g.glx.mult_matrix(self.trackball.matrix());
        let (x, y, z) = self.rot.rotation(turning);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);

        // Gold, and upstream's own mixing: what it calls the diffuse goes
        // into the specular slot and the other way round.
        let t = self.tick;
        let dim = |c: [f32; 4]| [c[0] * t, c[1] * t, c[2] * t, c[3]];
        g.glx.material_ambient(dim([0.33, 0.22, 0.03, 1.0]));
        g.glx.material_diffuse(dim([0.78, 0.57, 0.11, 1.0]));
        g.glx.material_specular(dim([0.99, 0.91, 0.81, 1.0]));
        g.glx.material_shininess(27.80);

        // The model's bounding box is not centred, so line the axis up with
        // the core of the apple.
        g.glx.translate(0.0534, 0.0394, -0.03);
        g.glx.call_list(self.list);
        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:     30000",
    "*showFPS:   False",
    "*wireframe: False",
    "*spin:      Y",
    "*wander:    False",
    "*speed:     1.0",
];

const SPINS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "Y",
        label: "Spin around Y",
    },
    crate::runtime::opts::SelectItem {
        value: "X",
        label: "Spin around X",
    },
    crate::runtime::opts::SelectItem {
        value: "Z",
        label: "Spin around Z",
    },
    crate::runtime::opts::SelectItem {
        value: "XYZ",
        label: "Spin around all three",
    },
    crate::runtime::opts::SelectItem {
        value: "0",
        label: "Do not spin",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::select("spin", "Spin", SPINS, "Y"),
    Opt::slider("speed", "Speed", 0.1, 5.0, 0.1, 1, "1.0"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "kallisti",
    label: "Kallisti",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2023",
        video: Some("https://www.youtube.com/watch?v=RL-DlEe0hkk"),
        blurb: "The golden apple of discord, inscribed to the fairest.",
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

    /// One model, one draw, and a great many triangles.
    #[test]
    fn the_apple_is_one_model() {
        let m = GlList::parse(crate::models::KALLISTI_MODEL);
        assert_eq!(m.primitive, crate::runtime::gl::Shape::Triangles);
        assert!(m.points > 20000, "the apple is only {} vertices", m.points);
        assert!(m.points.is_multiple_of(3), "a triangle is missing a corner");

        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let solids = r
            .frame()
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
            .count();
        assert_eq!(solids, 1, "{solids} draws is not one apple");
    }

    /// It fades up from black: every light and the material start at nothing
    /// and rise together.
    #[test]
    fn it_fades_up_from_black() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let first = r.frame().batches[0].material.ambient_diffuse;
        assert!(
            first.iter().take(3).all(|c| *c < 0.02),
            "it started lit: {first:?}"
        );
        for _ in 0..200 {
            r.step();
        }
        let later = r.frame().batches[0].material.ambient_diffuse;
        assert!(
            later[0] > first[0] && later[0] > 0.3,
            "it never came up: {later:?}"
        );
    }

    /// The third light swings all the way round, which is what moves the
    /// gold.
    #[test]
    fn a_light_swings_round_it() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut xs = Vec::new();
        for _ in 0..400 {
            r.step();
            let b = &r.frame().batches[0];
            xs.push(b.lights[2].position[0]);
        }
        let lo = xs.iter().copied().fold(f32::MAX, f32::min);
        let hi = xs.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 5.0, "the light only moved from {lo} to {hi}");
    }
}
