//! Port of `hacks/glx/kaleidocycle.c`.
//!
//! ```text
//! kaleidocycle, Copyright (c) 2013-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! A loop of rotating tetrahedra.  Created by jwz, July 2013.
//! Inspired by, and some math borrowed from:
//! http://www.kaleidocycles.de/pdf/kaleidocycles_theory.pdf
//! http://intothecontinuum.tumblr.com/post/50873970770/an-even-number-of-at-least-8-regular-tetrahedra
//! ```
//!
//! A ring of tetrahedra joined at the edges, turning continuously through its
//! own middle without any of them deforming. It is a real object you can make
//! out of paper, and the surprising part is that it works at all: hinge an even
//! number of tetrahedra in a loop, eight or more, and the ring can be pushed
//! round through itself for ever.
//!
//! All of the motion is one parameter. `t` is how far round the toroidal turn
//! the ring has been pushed, `a` is the angle one tetrahedron occupies, and
//! `draw_tetra` solves the four corners from those two directly. Nothing is
//! simulated and nothing is hinged: the shape at any moment is a closed-form
//! function of the turn, which is why it never drifts apart or jams.
//!
//! Every second tetrahedron is a mirror image of the one before it, and the
//! reflection is a real reflection: a matrix that flips space through the plane
//! between them, which inverts the winding, which is why each face sets its own
//! front face before drawing.
//!
//! The count is a fraction rather than a whole number. The part after the point
//! is one tetrahedron growing into or out of the ring, scaled and faded, and
//! the ring keeps growing until the count is both even and past the count that
//! was asked for. That is the whole of the animation of the ring's size: there
//! is no separate insert or remove, just a number that keeps moving.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_uniform_colormap};
use crate::runtime::gl::{Blend, Mat4, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, SelectItem, StartArgs, Trackball, XEvent,
    random, screenhack_event_helper, shapes::calc_normal,
};

/// Which way the ring's size is going.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Static,
    In,
    Out,
}

struct Kaleidocycle {
    rot: Rotator,
    /// The second rotator drives only the twist, and only about one axis.
    rot2: Rotator,
    trackball: Trackball,

    min_count: f32,
    max_count: f32,
    /// Still filling in to the count that was asked for, which it does at
    /// double speed and without stopping at the first even number.
    startup_p: bool,

    colors: Vec<XColor>,
    ccolor: usize,

    /// How many tetrahedra, with the fraction being the one on its way in.
    count: f32,
    /// The toroidal turn, in degrees, and the amount it grows by each frame.
    th: f32,
    dth: f32,

    mode: Mode,
    prev_mode: Mode,

    wanted: f32,
    speed: f32,
    wireframe: bool,
}

/// Reflect through the plane normal to the given vector.
fn reflect(x: f32, y: f32, z: f32) -> Mat4 {
    // Symmetric, so which way round the rows and columns go does not matter.
    Mat4([
        1.0 - 2.0 * x * x,
        -2.0 * x * y,
        -2.0 * x * z,
        0.0,
        -2.0 * x * y,
        1.0 - 2.0 * y * y,
        -2.0 * y * z,
        0.0,
        -2.0 * x * z,
        -2.0 * y * z,
        1.0 - 2.0 * z * z,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ])
}

impl Kaleidocycle {
    /// `t` = toroidal rotation, `a` = radial position, `colors` = 4 colours,
    /// 4 channels each.
    fn draw_tetra(&self, g: &mut Gl, t: f64, a: f64, reflect_p: bool, colors: &[[f32; 4]; 4]) {
        let wire = self.wireframe;

        let sint = t.sin();
        let cost = t.cos();
        let tana = a.tan();
        let sint2 = sint * sint;
        let tana2 = tana * tana;

        let v1 = [cost, 0.0, sint];

        let scale = 1.0 / (1.0 + sint2 * tana2).sqrt();
        let v2 = [scale * -sint, scale * -sint * tana, scale * cost];
        let v3 = [scale * -sint2 * tana, scale, scale * cost * sint * tana];

        let p = [v3[1] / tana - v3[0], 0.0, -v3[2] / 2.0];
        let q = [v3[1] / tana, v3[1], v3[2] / 2.0];

        // The four corners are two opposite edges: one through P along v1, one
        // through Q along v2. That is what a tetrahedron is, seen the right
        // way, and it is why the hinge between two of them is an edge.
        let scale = std::f64::consts::SQRT_2 / 2.0;
        let mix = |o: [f64; 3], v: [f64; 3], s: f64| {
            [
                (o[0] + s * v[0]) as f32,
                (o[1] + s * v[1]) as f32,
                (o[2] + s * v[2]) as f32,
            ]
        };
        let verts = [
            mix(p, v1, -scale),
            mix(p, v1, scale),
            mix(q, v2, -scale),
            mix(q, v2, scale),
        ];

        for i in 0..4 {
            let reflect2_p = (i + usize::from(reflect_p)) & 1 != 0;
            let a = verts[(i + 1) % 4];
            let b = verts[(i + 2) % 4];
            let c = verts[(i + 3) % 4];
            let n = if i & 1 != 0 {
                calc_normal(b, a, c)
            } else {
                calc_normal(a, b, c)
            };
            let col = colors[i];
            if wire {
                g.glx.color4f(col[0], col[1], col[2], col[3]);
            } else {
                g.glx.material_ambient_diffuse(col);
            }

            g.glx.front_face_cw(reflect2_p);
            g.glx.begin(if wire {
                Shape::LineLoop
            } else {
                Shape::Triangles
            });
            g.glx.normal3f(n[0], n[1], n[2]);
            for v in [a, b, c] {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();
        }
    }

    /// Wind the ring's size on by one frame, for the piece that is currently
    /// growing in or out. Returns how far grown it is, 0 to 1.
    fn advance_count(&mut self) -> f32 {
        let scale = self.count - self.count.floor();
        let mut tick = 0.07 * self.speed;

        /* Fill in faster if we're starting up */
        if self.count < self.wanted {
            tick *= 2.0;
        }
        match self.mode {
            Mode::In => {}
            Mode::Out => tick = -tick,
            Mode::Static => tick = 0.0,
        }

        let ocount = self.count;
        self.count += tick;

        let crossed = if self.mode == Mode::In {
            ocount.floor() != self.count.floor()
        } else {
            ocount.ceil() != self.count.ceil()
        };
        if crossed {
            self.count = if self.mode == Mode::In {
                ocount.floor() + 1.0
            } else {
                ocount.ceil() - 1.0
            };

            if (self.count.floor() as i32) & 1 != 0
                || (self.mode == Mode::In && self.count < self.wanted && self.startup_p)
            {
                /* keep going if it's odd, or less than 8. */
                self.count = self.count.round();
            } else {
                self.mode = Mode::Static;
                self.startup_p = false;
            }
        }
        scale
    }
}

impl Hack3d for Kaleidocycle {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
        }

        g.glx.push_matrix();

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 5.0,
            (y as f32 - 0.5) * 5.0,
            (z as f32 - 0.5) * 10.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        let (x, _, _) = self.rot2.rotation(!down);
        self.th = x as f32 * 360.0 * 10.0 * self.speed;

        /* Make sure the twist is always in motion.  Without this, the rotator
          sometimes stops, and for too long, and it's boring looking.
        */
        self.th += self.speed * self.dth;
        self.dth += 1.0;
        while self.dth > 360.0 {
            self.dth -= 360.0;
        }
        while self.th > 360.0 {
            self.th -= 360.0;
        }

        if !self.wireframe {
            g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
            g.glx.material_shininess(128.0);
        }

        /* Evenly spread the colors of the faces, and cycle them together. */
        let n = self.colors.len();
        let mut colors = [[0.0f32; 4]; 4];
        for (i, col) in colors.iter_mut().enumerate() {
            let o = n / 4;
            let c = &self.colors[(self.ccolor + o * i) % n];
            *col = [
                f32::from(c.red) / 65536.0,
                f32::from(c.green) / 65536.0,
                f32::from(c.blue) / 65536.0,
                1.0,
            ];
        }
        self.ccolor = (self.ccolor + 1) % n;

        let a = 2.0 * std::f64::consts::PI / f64::from(self.count.max(8.0));
        let t = f64::from(self.th) / (180.0 / std::f64::consts::PI);

        g.glx.scale(3.0, 3.0, 3.0);
        g.glx.scale(a as f32, a as f32, a as f32);
        g.glx.rotate(90.0, 0.0, 0.0, 1.0);

        // The bound is re-read every time round, because the growing piece
        // moves the count while the loop is running.
        let mut i = 0;
        while i <= self.count.floor() as i32 {
            let flip_p = i & 1 != 0;
            g.glx.push_matrix();
            g.glx
                .rotate((i / 2) as f32 * 4.0 * 180.0 / self.count, 0.0, 0.0, 1.0);
            if flip_p {
                g.glx
                    .mult_matrix(reflect(-(a.sin() as f32), a.cos() as f32, 0.0));
            }

            let mut colors = colors;
            if self.mode != Mode::Static && i >= self.count.floor() as i32 {
                /* Fractional count means the last piece is in transition */
                let scale = self.advance_count();
                g.glx.scale(scale, scale, scale);
                // Fading it as it grows, but never below 0.4: a piece that
                // arrives from nothing pops in less than one that arrives from
                // half-there.
                let alpha = (scale * scale * scale * scale).max(0.4);
                for c in &mut colors {
                    c[3] = alpha;
                }
            }

            self.draw_tetra(g, t, a, flip_p, &colors);
            g.glx.pop_matrix();
            i += 1;
        }

        if self.mode == Mode::Static && random().is_multiple_of(200) {
            self.mode = if self.count <= self.min_count {
                Mode::In
            } else if self.count >= self.max_count {
                Mode::Out
            } else {
                self.prev_mode
            };
            self.prev_mode = self.mode;
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
        if let XEvent::KeyPress { key } = event {
            match key {
                '+' | '.' | '>' | '=' => {
                    self.mode = Mode::In;
                    return true;
                }
                '-' | ',' | '<' | '_' if self.count > self.min_count => {
                    self.mode = Mode::Out;
                    return true;
                }
                _ => {}
            }
        }
        if screenhack_event_helper(event) {
            // Below the minimum it can only grow; above it, a coin toss. The
            // short circuit is upstream's: the coin is not tossed at all when
            // the ring is at its smallest.
            self.mode = if self.count <= self.min_count || random() & 1 != 0 {
                Mode::In
            } else {
                Mode::Out
            };
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let spin = g.res.string("spin").to_string();
    let (mut spinx, mut spiny, mut spinz) = (false, false, false);
    for c in spin.chars() {
        match c {
            'x' | 'X' => spinx = true,
            'y' | 'Y' => spiny = true,
            'z' | 'Z' => spinz = true,
            // Upstream exits on anything else. The panel only offers the eight
            // it knows about, so here it just means no spin about that axis.
            _ => {}
        }
    }

    let spin_speed = 0.25;
    let wander_speed = 0.005;
    let spin_accel = 0.2;
    let twist_speed = 0.25;
    let twist_accel = 1.0;

    let mut wanted = g.res.int("count").clamp(8, 64);
    if wanted & 1 != 0 {
        wanted += 1;
    }
    // A whole number of tetrahedra, and an even one: an odd ring cannot close.
    let mut max_count = (12.0 + f64::from(wanted) * 1.3) as i32;
    if max_count & 1 != 0 {
        max_count += 1;
    }

    let mut colors = make_uniform_colormap(512);
    for c in &mut colors {
        /* make colors twice as bright */
        c.red = (c.red >> 2) + 0x7FFF;
        c.green = (c.green >> 2) + 0x7FFF;
        c.blue = (c.blue >> 2) + 0x7FFF;
    }

    let mut st = Kaleidocycle {
        rot: Rotator::new(
            if spinx { spin_speed } else { 0.0 },
            if spiny { spin_speed } else { 0.0 },
            if spinz { spin_speed } else { 0.0 },
            spin_accel,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            false,
        ),
        rot2: Rotator::new(twist_speed, 0.0, 0.0, twist_accel, 0.0, true),
        trackball: Trackball::new(),
        min_count: 8.0,
        max_count: max_count as f32,
        startup_p: true,
        colors,
        ccolor: 0,
        // It builds itself from nothing when it starts.
        count: 0.0,
        th: 0.0,
        dth: 0.0,
        mode: Mode::In,
        prev_mode: Mode::In,
        wanted: wanted as f32,
        speed: g.res.float("speed") as f32,
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    g.glx.line_width(4.0);

    if !wire {
        // After the reshape, so the light is fixed to the camera.
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
        // The piece that is growing in is translucent, so the ring behind it
        // shows through rather than being punched out.
        g.glx.blend(Blend::Alpha);
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        16",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         Z",
    "*wander:       False",
    "*speed:        1.0",
];

const SPINS: &[SelectItem] = &[
    SelectItem {
        value: "0",
        label: "Don't rotate",
    },
    SelectItem {
        value: "X",
        label: "Rotate around X axis",
    },
    SelectItem {
        value: "Y",
        label: "Rotate around Y axis",
    },
    SelectItem {
        value: "Z",
        label: "Rotate around Z axis",
    },
    SelectItem {
        value: "XY",
        label: "Rotate around X and Y axes",
    },
    SelectItem {
        value: "XZ",
        label: "Rotate around X and Z axes",
    },
    SelectItem {
        value: "YZ",
        label: "Rotate around Y and Z axes",
    },
    SelectItem {
        value: "XYZ",
        label: "Rotate around all three axes",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("count", "Count", 8.0, 64.0, 2.0, 0, "16"),
    Opt::slider("speed", "Speed", 0.1, 8.0, 0.1, 1, "1.0"),
    Opt::select("spin", "Rotation", SPINS, "Z"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "kaleidocycle",
    label: "Kaleidocycle",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2013",
        video: Some("https://www.youtube.com/watch?v=SJqRaCCy_vo"),
        blurb: "A ring of tetrahedra that twists and rotates toroidally.",
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

    /// Reflecting twice through the same plane is doing nothing, which is what
    /// makes it a reflection rather than some other matrix.
    #[test]
    fn a_reflection_is_its_own_inverse() {
        let m = reflect(0.6, 0.8, 0.0);
        let back = m.mul(&m);
        for (i, v) in back.0.iter().enumerate() {
            let want = if i % 5 == 0 { 1.0 } else { 0.0 };
            assert!((v - want).abs() < 1e-5, "{back:?}");
        }
        // And a point on the plane stays put while one off it crosses over.
        let on = m.transform([0.8, -0.6, 0.0]);
        assert!((on[0] - 0.8).abs() < 1e-5 && (on[1] + 0.6).abs() < 1e-5);
        let off = m.transform([0.6, 0.8, 0.0]);
        assert!((off[0] + 0.6).abs() < 1e-5 && (off[1] + 0.8).abs() < 1e-5);
    }

    /// Every tetrahedron is the same regular tetrahedron however far the ring
    /// has been turned. That is the whole claim the object makes: it rotates
    /// through itself without deforming.
    #[test]
    fn a_tetrahedron_never_deforms() {
        let mut r = start(StartArgs::new(640, 480, "count=8", 20260811));
        let mut edges: Vec<f32> = Vec::new();
        for _ in 0..80 {
            r.step();
            let f = r.frame();
            // The first four batches are the four faces of one tetrahedron,
            // whose twelve vertices are its four corners three times over.
            let Some(b) = f.batches.first() else { continue };
            if b.count < 3 {
                continue;
            }
            let vs = &f.vertices[b.first..b.first + 3];
            for (i, u) in vs.iter().enumerate() {
                let v = vs[(i + 1) % 3];
                let d = ((u.pos[0] - v.pos[0]).powi(2)
                    + (u.pos[1] - v.pos[1]).powi(2)
                    + (u.pos[2] - v.pos[2]).powi(2))
                .sqrt();
                edges.push(d);
            }
        }
        assert!(edges.len() > 100, "nothing was drawn");
        let lo = edges.iter().fold(f32::MAX, |a, &b| a.min(b));
        let hi = edges.iter().fold(0.0f32, |a, &b| a.max(b));
        assert!(
            (hi - lo).abs() < 1e-3,
            "edges ran from {lo} to {hi}, so it deformed"
        );
    }

    /// The ring builds itself up to the count it was asked for, one piece at a
    /// time, and stops there rather than growing for ever.
    #[test]
    fn the_ring_fills_in_to_the_count_it_was_given() {
        let mut r = start(StartArgs::new(640, 480, "count=12", 20260811));
        // One batch per face, four faces per tetrahedron.
        let pieces = |r: &Runner3d| r.frame().batches.len() / 4;
        r.step();
        assert!(pieces(&r) <= 2, "it starts from nothing");
        let mut most = 0;
        for _ in 0..600 {
            r.step();
            most = most.max(pieces(&r));
        }
        assert!(most >= 12, "only ever reached {most}");
        // Twelve asked for means it may keep going to 12 + 12*1.3, rounded
        // down and made even, and no further.
        assert!(most <= 30, "grew without bound to {most}");
    }
}
