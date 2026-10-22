//! Port of `hacks/glx/moebius.c`.
//!
//! ```text
//! moebius --- Moebius Strip II, an Escher-like GL scene with ants.
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
//! The RotateAroundU() routine was adapted from the book
//!    "Computer Graphics Principles and Practice
//!     Foley - vanDam - Feiner - Hughes
//!     Second Edition" Pag. 227, exercise 5.15.
//!
//! Copyright (c) 1998 by Marcelo F. Vianna.
//! ```
//!
//! Escher's *Moebius Strip II*, the woodcut with the ants: four of them walk
//! the strip, and because it has only one side they all end up on the same
//! surface however far round they go. Two of the four are on the far side at
//! any moment and turn up again on this one half a lap later.
//!
//! The strip is a circle of radius three with a cross-section that turns
//! through half a revolution over one full turn, built by rotating a vector
//! about the tangent as it goes. In net mode the same strip is drawn eight
//! times over at different distances from the centre line, which upstream
//! outlines with a polygon mode; there is no polygon mode here, so the
//! outlines go out as lines.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::rotator::Rotator;
use crate::runtime::shapes::unit_sphere;
use crate::runtime::tube::cone;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
};

/// How many steps round the strip, and how many bands across it in net mode.
const DIVISIONS: usize = 40;
const TRANSVERSALS: i32 = 4;

const SCALE_4_WINDOW: f32 = 0.3;

const RED: [f32; 4] = [0.7, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.1, 0.5, 0.2, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 0.7, 1.0];
const CYAN: [f32; 4] = [0.2, 0.5, 0.7, 1.0];
const YELLOW: [f32; 4] = [0.7, 0.7, 0.0, 1.0];
const MAGENTA: [f32; 4] = [0.6, 0.2, 0.5, 1.0];
const WHITE: [f32; 4] = [0.7, 0.7, 0.7, 1.0];
const GRAY: [f32; 4] = [0.2, 0.2, 0.2, 1.0];

struct Moebius {
    trackball: Trackball,
    rot: Rotator,
    /// How far round the strip the ants have walked, and where their legs are
    /// in their stride.
    ant_position: f32,
    ant_step: f32,
    width: i32,
    height: i32,
    solid: bool,
    ants: bool,
}

/// `RotateAaroundU`: turn the vector A about the unit axis U by Theta. This is
/// what sweeps the strip's cross-section round as it goes, and what gives the
/// thing its half twist.
fn rotate_around_u(a: [f32; 3], u: [f32; 3], theta: f32) -> [f32; 3] {
    let (c, s) = (theta.cos(), theta.sin());
    let k = 1.0 - c;
    let (ux, uy, uz) = (u[0], u[1], u[2]);
    let (ux2, uy2, uz2) = (ux * ux, uy * uy, uz * uz);
    let (uxuy, uxuz, uyuz) = (ux * uy, ux * uz, uy * uz);
    [
        (ux2 + c * (1.0 - ux2)) * a[0] + (uxuy * k - uz * s) * a[1] + (uxuz * k + uy * s) * a[2],
        (uxuy * k + uz * s) * a[0] + (uy2 + c * (1.0 - uy2)) * a[1] + (uyuz * k - ux * s) * a[2],
        (uxuz * k - uy * s) * a[0] + (uyuz * k + ux * s) * a[1] + (uz2 + c * (1.0 - uz2)) * a[2],
    ]
}

/// One corner of a quad of the net: where it is and what colour it is.
type Corner = ([f32; 3], [f32; 4]);

/// The point and normal of the strip at one place along it, `phi` round and
/// `t` across (with `t` in -1..1).
fn strip_point(phi: f32, t: f32) -> ([f32; 3], [f32; 3]) {
    let theta = phi / 2.0;
    let (c, s) = (phi.cos(), phi.sin());
    let axis = [-s, c, 0.0];
    let normal = rotate_around_u([c, s, 0.0], axis, theta);
    let across = rotate_around_u([0.0, 0.0, 1.0], axis, theta);
    (
        [
            c * 3.0 + across[0] * t,
            s * 3.0 + across[1] * t,
            across[2] * t,
        ],
        normal,
    )
}

impl Moebius {
    fn sphere(&self, g: &mut Gl, radius: f32) {
        g.glx.push_matrix();
        g.glx.scale(radius, radius, radius);
        unit_sphere(&mut g.glx, 16, 16, false);
        g.glx.pop_matrix();
    }

    fn cone(&self, g: &mut Gl, radius: f32) {
        cone(
            &mut g.glx,
            [0.0, 0.0, 0.0],
            [0.0, 0.0, radius * 3.0],
            radius,
            0.0,
            8,
            true,
            true,
            false,
        );
    }

    /// `draw_moebius_ant`: a body of three spheres and two horns, with legs
    /// and antennae drawn as bare lines because they are only a few pixels
    /// wide anyway.
    fn draw_ant(&mut self, g: &mut Gl, material: [f32; 4]) {
        let phase = self.ant_step;
        let third = 2.0 * std::f32::consts::PI / 3.0;
        let (cos1, cos2, cos3) = (
            phase.cos(),
            (phase + third).cos(),
            (phase + 2.0 * third).cos(),
        );
        let (sin1, sin2, sin3) = (
            phase.sin(),
            (phase + third).sin(),
            (phase + 2.0 * third).sin(),
        );

        g.glx.lighting(true);
        g.glx.color_material(true);
        g.glx
            .color4f(material[0], material[1], material[2], material[3]);
        g.glx.cull_face(true);

        g.glx.push_matrix();
        g.glx.scale(1.0, 1.3, 1.0);
        self.sphere(g, 0.18);
        g.glx.scale(1.0, 1.0 / 1.3, 1.0);
        g.glx.translate(0.0, 0.30, 0.0);
        self.sphere(g, 0.2);
        g.glx.translate(-0.05, 0.17, 0.05);
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(-25.0, 0.0, 1.0, 0.0);
        self.cone(g, 0.05);
        g.glx.translate(0.0, 0.10, 0.0);
        self.cone(g, 0.05);
        g.glx.rotate(25.0, 0.0, 1.0, 0.0);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.scale(1.0, 1.3, 1.0);
        g.glx.translate(0.15, -0.65, 0.05);
        self.sphere(g, 0.25);
        g.glx.scale(1.0, 1.0 / 1.3, 1.0);
        g.glx.pop_matrix();

        g.glx.cull_face(false);
        g.glx.lighting(false);

        // Antennae, dark at the tips.
        g.glx.begin(Shape::Lines);
        for (x, y, z) in [(0.40, 0.70, 0.40), (0.40, 0.70, -0.40)] {
            g.glx
                .color4f(material[0], material[1], material[2], material[3]);
            g.glx.vertex3f(0.0, 0.30, 0.0);
            g.glx.color4f(GRAY[0], GRAY[1], GRAY[2], GRAY[3]);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.end();
        g.glx.begin(Shape::Points);
        g.glx.color4f(RED[0], RED[1], RED[2], RED[3]);
        g.glx.vertex3f(0.40, 0.70, 0.40);
        g.glx.vertex3f(0.40, 0.70, -0.40);
        g.glx.end();

        // Six legs, three a side, a third of a stride apart from each other.
        let legs = [
            (
                0.05,
                0.35 + 0.05 * cos1,
                0.15,
                -0.20 + 0.05 * cos1,
                0.25 + 0.1 * sin1,
                0.18,
            ),
            (
                0.00,
                0.35 + 0.05 * cos2,
                0.00,
                -0.20 + 0.05 * cos2,
                0.00 + 0.1 * sin2,
                0.18,
            ),
            (
                -0.05,
                0.35 + 0.05 * cos3,
                -0.15,
                -0.20 + 0.05 * cos3,
                -0.25 + 0.1 * sin3,
                0.18,
            ),
            (
                0.05,
                0.35 - 0.05 * sin1,
                0.15,
                -0.20 - 0.05 * sin1,
                0.25 + 0.1 * cos1,
                -0.18,
            ),
            (
                0.00,
                0.35 - 0.05 * sin2,
                0.00,
                -0.20 - 0.05 * sin2,
                0.00 + 0.1 * cos2,
                -0.18,
            ),
            (
                -0.05,
                0.35 - 0.05 * sin3,
                -0.15,
                -0.20 - 0.05 * sin3,
                -0.25 + 0.1 * cos3,
                -0.18,
            ),
        ];
        for (y0, x1, y1, x2, y2, side) in legs {
            let z = if side > 0.0 { 0.25 } else { -0.25 };
            let foot = if side > 0.0 { 0.45 } else { -0.45 };
            g.glx.begin(Shape::LineStrip);
            g.glx
                .color4f(material[0], material[1], material[2], material[3]);
            g.glx.vertex3f(0.0, y0, side);
            g.glx.vertex3f(x1, y1, z);
            g.glx.color4f(GRAY[0], GRAY[1], GRAY[2], GRAY[3]);
            g.glx.vertex3f(x2, y2, foot);
            g.glx.end();
        }

        // The feet.
        g.glx.begin(Shape::Points);
        g.glx
            .color4f(MAGENTA[0], MAGENTA[1], MAGENTA[2], MAGENTA[3]);
        for (_, _, _, x2, y2, side) in legs {
            g.glx
                .vertex3f(x2, y2, if side > 0.0 { 0.45 } else { -0.45 });
        }
        g.glx.end();

        g.glx.lighting(true);
        self.ant_step += 0.3;
    }

    /// `draw_moebius_strip`.
    fn draw_strip(&mut self, g: &mut Gl) {
        g.glx.color_material(true);
        let step = std::f32::consts::PI / DIVISIONS as f32;

        if self.solid {
            g.glx.begin(Shape::QuadStrip);
            for i in 0..(DIVISIONS * 2 + 1) {
                let phi = i as f32 * step;
                let (p, n) = strip_point(phi, 1.0);
                // The bands alternate colour every step, which is what makes
                // the twist readable.
                let c = if (i + 1) % 2 == 1 { RED } else { GRAY };
                g.glx.color4f(c[0], c[1], c[2], c[3]);
                g.glx.normal3f(n[0], n[1], n[2]);
                g.glx.vertex3f(p[0], p[1], p[2]);
                let (p, _) = strip_point(phi, -1.0);
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
            return;
        }

        // The net: the same strip at eight distances across, outlined. Every
        // quad of the strip becomes its own outline, as a polygon mode of
        // GL_LINE would draw it.
        for j in -TRANSVERSALS..TRANSVERSALS {
            let mut prev: Option<(Corner, Corner)> = None;
            for i in 0..(DIVISIONS * 2 + 1) {
                let phi = i as f32 * step;
                let t_hi = (j + 1) as f32 / TRANSVERSALS as f32;
                let t_lo = j as f32 / TRANSVERSALS as f32;
                let (hi, _) = strip_point(phi, t_hi);
                let (lo, _) = strip_point(phi, t_lo);
                // The outermost band is white; the rest alternate.
                let col = |edge: i32| {
                    if edge == TRANSVERSALS || edge == -TRANSVERSALS {
                        WHITE
                    } else if i % 2 == 1 {
                        RED
                    } else {
                        GRAY
                    }
                };
                let (c_hi, c_lo) = (col(j + 1), col(j));
                if let Some((prev_hi, prev_lo)) = prev {
                    g.glx.begin(Shape::LineLoop);
                    for (v, c) in [prev_hi, (hi, c_hi), (lo, c_lo), prev_lo] {
                        g.glx.color4f(c[0], c[1], c[2], c[3]);
                        g.glx.vertex3f(v[0], v[1], v[2]);
                    }
                    g.glx.end();
                }
                prev = Some(((hi, c_hi), (lo, c_lo)));
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut this = Moebius {
        trackball: Trackball::new(),
        rot: Rotator::new(0.3, 0.3, 0.3, 1.0, 0.0, true),
        ant_position: (random() % 90) as f32,
        ant_step: 0.0,
        width: 1,
        height: 1,
        solid: g.res.bool("solidmoebius"),
        ants: g.res.bool("drawants"),
    };
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Moebius {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width;
            y = -height / 2;
        }
        self.width = width;
        self.height = height;
        g.glx.viewport(0, y, width, height);
        // The lines are the ants' legs, so they get thicker with the window.
        let w = if width >= 1024 {
            3.0
        } else if width >= 512 {
            2.0
        } else {
            1.0
        };
        g.glx.line_width(w);
        g.glx.point_size(w);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -1.0, 1.0, 5.0, 15.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.clear();
        g.glx.light_enable(0, true);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_enable(1, true);
        g.glx.light_ambient(1, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(1, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(1, -1.0, -1.0, 1.0, 0.0);
        g.glx.light_model_ambient([0.5, 0.5, 0.5, 1.0]);
        g.glx.material_specular([0.7, 0.7, 0.7, 1.0]);
        g.glx.material_shininess(60.0);
        g.glx.lighting(true);
        g.glx.front_face_cw(false);
        g.glx.depth_test(true);

        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -10.0);
        g.glx.mult_matrix(self.trackball.matrix());
        let (w, h) = (self.width as f32, self.height as f32);
        g.glx
            .scale(SCALE_4_WINDOW * h / w, SCALE_4_WINDOW, SCALE_4_WINDOW);
        let s = if w < h { w / h } else { 1.0 };
        g.glx.scale(s, s, s);

        let (x, y, z) = self.rot.rotation(!self.trackball.button_down());
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        self.draw_strip(g);

        if self.ants {
            // Four ants, a quarter lap apart, two of them on the far face.
            // Because the strip has one side, the far pair are the near pair
            // half a lap later.
            let p = self.ant_position;
            for (turn, half_turn, z, flip, colour) in [
                (p + 180.0, p / 2.0 + 90.0, -0.45, false, YELLOW),
                (p, p / 2.0, -0.45, false, BLUE),
                (-p, -p / 2.0, 0.45, true, GREEN),
                (-p + 180.0, -p / 2.0 + 90.0, 0.45, true, CYAN),
            ] {
                g.glx.push_matrix();
                g.glx.rotate(turn, 0.0, 0.0, 1.0);
                g.glx.translate(3.0, 0.0, 0.0);
                g.glx.rotate(half_turn, 0.0, 1.0, 0.0);
                g.glx.translate(0.28, 0.0, z);
                if flip {
                    g.glx.rotate(180.0, 1.0, 0.0, 0.0);
                }
                self.draw_ant(g, colour);
                g.glx.pop_matrix();
            }
        }
        self.ant_position += 1.0;

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:         1000",
    "*showFPS:       False",
    "*solidmoebius:  False",
    "*drawants:      True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "1000").inverted(),
    Opt::boolean("solidmoebius", "Solid strip", "false"),
    Opt::boolean("drawants", "Draw ants", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "moebius",
    label: "Moebius",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Marcelo Vianna",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=77Nib6jQrXc"),
        blurb: "Escher's Moebius Strip II, with the ants walking round a \
                surface that has only one side.",
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

    /// The strip has one side: go all the way round and the cross-section has
    /// swapped ends with itself.
    #[test]
    fn one_full_turn_comes_back_upside_down() {
        let two_pi = 2.0 * std::f32::consts::PI;
        let (start_hi, _) = strip_point(0.0, 1.0);
        let (start_lo, _) = strip_point(0.0, -1.0);
        let (end_hi, _) = strip_point(two_pi, 1.0);
        let (end_lo, _) = strip_point(two_pi, -1.0);
        let dist = |a: [f32; 3], b: [f32; 3]| -> f32 {
            (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
        };
        assert!(
            dist(start_hi, end_lo) < 1e-4 && dist(start_lo, end_hi) < 1e-4,
            "the edges did not swap: {:?} {:?} vs {:?} {:?}",
            start_hi,
            start_lo,
            end_hi,
            end_lo
        );
        assert!(dist(start_hi, end_hi) > 1.0, "the strip has no twist");
    }

    /// Its centre line is a circle of radius three, whatever the twist is
    /// doing.
    #[test]
    fn the_centre_line_is_a_circle() {
        for i in 0..32 {
            let phi = 2.0 * std::f32::consts::PI * i as f32 / 32.0;
            let (p, _) = strip_point(phi, 0.0);
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((r - 3.0).abs() < 1e-4, "at {phi} the radius is {r}");
        }
    }

    /// The normal is a unit vector everywhere, or the lighting would band.
    #[test]
    fn the_normals_are_unit_length() {
        for i in 0..64 {
            let phi = 2.0 * std::f32::consts::PI * i as f32 / 64.0;
            let (_, n) = strip_point(phi, 0.3);
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "at {phi} the normal is {len}");
        }
    }

    #[test]
    fn there_are_four_ants_and_they_walk() {
        let mut r = start(StartArgs::new(640, 480, "solidmoebius=true", 20260811));
        r.step();
        let f = r.frame();
        // Every ant is three spheres, and a sphere is one strip.
        let strips = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::TriangleStrip)
            .count();
        assert_eq!(strips, 4 * 3, "{strips} sphere strips is not four ants");

        // Their legs move: the feet are somewhere else a few frames later.
        let feet = |r: &Runner3d| -> Vec<f32> {
            let f = r.frame();
            f.batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Points)
                .flat_map(|b| f.vertices[b.first..b.first + b.count].iter())
                .map(|v| v.pos[1])
                .collect()
        };
        let before = feet(&r);
        r.step();
        let after = feet(&r);
        assert_eq!(before.len(), after.len());
        assert!(
            before
                .iter()
                .zip(after.iter())
                .any(|(a, b)| (a - b).abs() > 0.001),
            "the ants are not moving their legs"
        );
    }

    #[test]
    fn the_ants_can_be_left_out() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "solidmoebius=true&drawants=false",
            20260811,
        ));
        r.step();
        let f = r.frame();
        assert!(
            f.batches
                .iter()
                .all(|b| b.primitive == crate::runtime::gl::Primitive::Triangles),
            "something other than the strip was drawn"
        );
    }

    /// The net draws the same strip eight times across, as outlines.
    #[test]
    fn the_net_is_eight_bands_of_outlines() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "solidmoebius=false&drawants=false",
            20260811,
        ));
        r.step();
        let loops = r
            .frame()
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::LineLoop)
            .count();
        assert_eq!(loops, 8 * DIVISIONS * 2, "the net is {loops} outlines");
    }
}
