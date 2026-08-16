//! Port of `hacks/glx/providence.c`.
//!
//! ```text
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
//! Copyright 2004 Blair Tennessy
//! ```
//!
//! "A pyramid unfinished. In the zenith an eye in a triangle, surrounded by a
//! glory, proper." The reverse of the Great Seal of the United States, as it
//! appears on the back of a dollar bill.
//!
//! The pyramid is four textured quads and a lid. Everything else is particles:
//! the glory is two thousand short line segments crawling outwards along the
//! edges of the triangle, and the eye is two thousand points that walk out
//! from the pupil along one of seven hundred and twenty rays and are reborn at
//! the middle when they reach the rim. Turn the seal far enough round and the
//! eye is replaced by a dollar sign made the same way.
//!
//! Upstream precomputes both particle paths into a pair of tables that come to
//! several megabytes. They are pure functions of the ray and the distance
//! along it, so this computes them where they are used instead.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape, TexEnv};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

/// How many rays the eye's particles crawl along, and how far along each of
/// them they can get.
const LOOKUPSIZE: usize = 3600 / 5;
const EYELENGTH: usize = 300;

const PARTICLE_COUNT: usize = 2000;
const EYE_PARTICLE_COUNT: usize = 2000;

/// The animation is written against a fixed clock rather than real time.
const FPS: f64 = 50.0;

const CHECK_DIMENSION: usize = 64;

const MATERIAL_GLORY: [f32; 4] = [0.04, 0.30, 0.22, 0.7];
const MATERIAL_GLORY_B: [f32; 4] = [0.07, 0.50, 0.36, 0.6];
const MATERIAL_GLORY_F: [f32; 4] = [0.07, 0.50, 0.36, 1.0];

/// One glory particle: where it is, when it was born and how long it lives.
#[derive(Clone, Copy, Default)]
struct Particle {
    pos: [f64; 3],
    born: f64,
    life: f64,
}

struct Providence {
    trackball: Trackball,
    aspect: f32,
    brick: u32,
    pyramid: u32,

    currenttime: f64,
    theta: f64,
    theta_scale: f64,
    camera_z: f64,
    camera_velocity: f64,

    particles: Vec<Particle>,
    /// Which ray each eye particle is on, and how far along it has got.
    eye_particles: Vec<(usize, usize)>,

    eye: bool,
    wire: bool,
}

/// A point on the eye: distance `j` out from the pupil along ray `i`.
fn eye_point(i: usize, j: usize) -> [f32; 2] {
    let r = i as f64 * 2.0 * std::f64::consts::PI / LOOKUPSIZE as f64;
    let x = 0.07 + j as f64 * (0.1 / EYELENGTH as f64);
    [(x * r.sin()) as f32, (x * r.cos()) as f32]
}

/// The same for the dollar sign, which is drawn as a sine wave with a stroke
/// through it rather than as rays out of a centre.
fn dollar_point(i: usize, j: usize) -> [f32; 2] {
    let pi = std::f64::consts::PI;
    let y = -1.2 * pi + j as f64 * (2.4 * pi / EYELENGTH as f64);
    if !i.is_multiple_of(2) {
        [
            (y.sin() / 6.0 + i as f64 / 36000.0 - 0.05) as f32,
            if !i.is_multiple_of(4) {
                (y / 12.0 - 0.05) as f32
            } else {
                (1.2 * pi - y / 12.0 + 0.05) as f32
            },
        ]
    } else {
        [(i as f64 / 36000.0 - 0.05) as f32, (y / 9.0 - 0.05) as f32]
    }
}

impl Providence {
    /// `init_particle`: a place somewhere along the three sides of the glory's
    /// triangle, chosen by arc length so the sides are evenly covered.
    fn init_particle(&self, p: &mut Particle) {
        let d = (random() % 485_410) as f64 / 100_000.0;
        let slope = 2.0f64.atan();
        p.pos[2] = 0.0;
        if d < 1.5 {
            p.pos[0] = d - 0.75;
            p.pos[1] = -0.75001;
        } else if d < 1.5 + 45.0f64.sqrt() / 4.0 {
            let d = d - 1.5;
            p.pos[0] = 0.75 - d * slope.cos();
            p.pos[1] = d * slope.sin() - 0.75;
        } else {
            let d = 4.8541 - d;
            p.pos[0] = -0.75 + d * slope.cos();
            p.pos[1] = d * slope.sin() - 0.75;
        }
        p.born = self.currenttime;
        p.life = 1.25 + (random() % 10) as f64 / 10.0;
    }

    fn update_particles(&mut self) {
        for i in 0..self.particles.len() {
            let mut p = self.particles[i];
            if self.currenttime > p.born + p.life {
                self.init_particle(&mut p);
                self.particles[i] = p;
            }
        }

        // Behind the seal the eye particles are slower and die sooner, which
        // is what makes the dollar sign a fainter thing than the eye.
        let behind = self.theta.cos() < 0.0;
        for e in &mut self.eye_particles {
            let x = e.1 + random() as usize % if behind { 8 } else { 16 };
            if x >= EYELENGTH || random().is_multiple_of(if behind { 40 } else { 10 }) {
                *e = (random() as usize % LOOKUPSIZE, random() as usize % 40);
            } else {
                e.1 = x;
            }
        }
    }

    /// `draw_seal`: the pyramid, which never changes and so is a display list.
    fn build_pyramid(&mut self, g: &mut Gl) {
        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        g.glx.push_matrix();
        if self.wire {
            g.glx.lighting(false);
            g.glx.texturing(false);
        } else {
            g.glx.texturing(true);
            g.glx.bind_texture(self.brick);
            g.glx.lighting(true);
            g.glx.color4f(
                MATERIAL_GLORY_F[0],
                MATERIAL_GLORY_F[1],
                MATERIAL_GLORY_F[2],
                MATERIAL_GLORY_F[3],
            );
            g.glx.material_diffuse(MATERIAL_GLORY_F);
        }

        let base = 2.0f32.sqrt();
        let top = 1.0 / 2.0f32.sqrt();
        let tmod = 7.0 / 6.0;

        g.glx.rotate(45.0, 0.0, 1.0, 0.0);
        g.glx.translate(0.0, -3.25, 0.0);

        // The four sloping faces. Each rotation is relative to the last, so
        // the quarter turns accumulate: 0, 90, 270, 630.
        for i in 0..4 {
            g.glx.rotate(i as f32 * 90.0, 0.0, 1.0, 0.0);
            g.glx.begin(if self.wire {
                Shape::LineLoop
            } else {
                Shape::Quads
            });
            let s6 = 6.0f32.sqrt();
            g.glx.normal3f(1.0 / s6, 2.0 / s6, 1.0 / s6);
            g.glx.tex_coord2f(-base, 0.0);
            g.glx.vertex3f(-base, 0.0, base);
            g.glx.tex_coord2f(base, 0.0);
            g.glx.vertex3f(base, 0.0, base);
            g.glx.tex_coord2f(top, 13.0 / 4.0);
            g.glx.vertex3f(top, 2.0, top);
            g.glx.tex_coord2f(-top, 13.0 / 4.0);
            g.glx.vertex3f(-top, 2.0, top);
            g.glx.end();
        }

        g.glx.begin(if self.wire {
            Shape::LineLoop
        } else {
            Shape::Quads
        });
        // The flat top, where the capstone is missing.
        g.glx.normal3f(0.0, 1.0, 0.0);
        g.glx.tex_coord2f(0.02, 0.0);
        g.glx.vertex3f(-top, 2.0, top);
        g.glx.tex_coord2f(2.0 * top, 0.0);
        g.glx.vertex3f(top, 2.0, top);
        g.glx.tex_coord2f(2.0 * top, tmod * 2.1 * top);
        g.glx.vertex3f(top, 2.0, -top);
        g.glx.tex_coord2f(0.02, tmod * 2.1 * top);
        g.glx.vertex3f(-top, 2.0, -top);
        // And the base.
        g.glx.normal3f(0.0, -1.0, 0.0);
        g.glx.tex_coord2f(-base, 0.0);
        g.glx.vertex3f(-base, 0.0, -base);
        g.glx.tex_coord2f(top, 0.0);
        g.glx.vertex3f(base, 0.0, -base);
        g.glx.tex_coord2f(top, top * 13.0 / 4.0);
        g.glx.vertex3f(base, 0.0, base);
        g.glx.tex_coord2f(-top, top * 13.0 / 4.0);
        g.glx.vertex3f(-base, 0.0, base);
        g.glx.end();

        g.glx.pop_matrix();
        g.glx.texturing(false);
        g.glx.end_list();
        self.pyramid = list;
    }

    /// `draw_glory`: every particle is a short line pointing away from the
    /// centre, and the longer it has lived the longer the line.
    fn draw_glory(&self, g: &mut Gl) {
        if self.wire {
            g.glx.begin(Shape::Triangles);
            for p in [
                [-0.75f32, -0.75, 0.0],
                [0.75, -0.75, 0.0],
                [0.0, 0.75, 0.0],
                [0.0, 0.75, 0.0],
                [0.75, -0.75, 0.0],
                [-0.75, -0.75, 0.0],
            ] {
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
            return;
        }

        g.glx.lighting(false);
        g.glx.blend(Blend::Alpha);
        for (half, col) in [MATERIAL_GLORY, MATERIAL_GLORY_B].into_iter().enumerate() {
            g.glx.color4f(col[0], col[1], col[2], col[3]);
            g.glx.material_diffuse(col);
            g.glx.begin(Shape::Lines);
            let range = if half == 0 {
                0..PARTICLE_COUNT / 2
            } else {
                PARTICLE_COUNT / 2..PARTICLE_COUNT
            };
            for i in range {
                let p = &self.particles[i];
                let t = self.currenttime - p.born;
                let mut th = (p.pos[1] / p.pos[0]).atan();
                if p.pos[0] < 0.0 {
                    th += std::f64::consts::PI;
                }
                g.glx
                    .vertex3f(p.pos[0] as f32, p.pos[1] as f32, p.pos[2] as f32);
                g.glx.vertex3f(
                    (p.pos[0] + 0.2 * th.cos() * t) as f32,
                    (p.pos[1] + 0.2 * th.sin() * t) as f32,
                    p.pos[2] as f32,
                );
            }
            g.glx.end();
        }
        g.glx.lighting(true);
    }

    /// `draw_eye`: the same particles twice, once small and once blown up over
    /// the whole triangle, with the two colours swapped between the copies.
    fn draw_eye(&self, g: &mut Gl) {
        if self.wire {
            g.glx.begin(Shape::Triangles);
            for p in [[-0.25f32, -0.25, 0.0], [0.25, -0.25, 0.0], [0.0, 0.25, 0.0]] {
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
            return;
        }

        g.glx.lighting(false);
        g.glx.blend(Blend::Alpha);
        self.eye_points(g, MATERIAL_GLORY, MATERIAL_GLORY_B, eye_point);

        g.glx.push_matrix();
        g.glx.scale(3.3, 2.2, 3.3);
        self.eye_points(g, MATERIAL_GLORY_B, MATERIAL_GLORY, eye_point);
        g.glx.pop_matrix();

        g.glx.lighting(true);
    }

    /// `draw_eye2`: the dollar sign, drawn once and not blown up.
    fn draw_eye2(&self, g: &mut Gl) {
        if self.wire {
            g.glx.begin(Shape::Triangles);
            for p in [[0.0f32, 0.25, 0.0], [0.25, -0.25, 0.0], [-0.25, -0.25, 0.0]] {
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
            return;
        }

        g.glx.lighting(false);
        g.glx.blend(Blend::Alpha);
        self.eye_points(g, MATERIAL_GLORY, MATERIAL_GLORY_B, dollar_point);
        g.glx.lighting(true);
    }

    /// Both halves of the eye particles, in the two given colours.
    fn eye_points(
        &self,
        g: &mut Gl,
        first: [f32; 4],
        second: [f32; 4],
        at: fn(usize, usize) -> [f32; 2],
    ) {
        for (half, col) in [first, second].into_iter().enumerate() {
            g.glx.color4f(col[0], col[1], col[2], col[3]);
            g.glx.material_diffuse(col);
            g.glx.begin(Shape::Points);
            let range = if half == 0 {
                0..EYE_PARTICLE_COUNT / 2
            } else {
                EYE_PARTICLE_COUNT / 2..EYE_PARTICLE_COUNT
            };
            for i in range {
                let (ray, along) = self.eye_particles[i];
                let p = at(ray, along);
                g.glx.vertex3f(p[0], p[1], 0.0);
            }
            g.glx.end();
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut this = Providence {
        trackball: Trackball::new(),
        aspect: 1.0,
        brick: 0,
        pyramid: 0,
        currenttime: 0.0,
        theta: 0.0,
        // Make multiple screens rotate at slightly different rates.
        theta_scale: 0.7 + frand(0.6),
        camera_z: 0.0,
        camera_velocity: -8.0,
        particles: vec![Particle::default(); PARTICLE_COUNT],
        eye_particles: vec![(0, 0); EYE_PARTICLE_COUNT],
        eye: g.res.bool("eye"),
        wire: g.res.bool("wireframe"),
    };

    for i in 0..PARTICLE_COUNT {
        let mut p = this.particles[i];
        this.init_particle(&mut p);
        // Stagger the birthdays so the glory does not pulse.
        p.born = this.currenttime - (random() % 1250) as f64 / 1000.0;
        this.particles[i] = p;
    }
    for e in &mut this.eye_particles {
        *e = (
            random() as usize % LOOKUPSIZE,
            random() as usize % EYELENGTH,
        );
    }

    // The brickwork is noise with a mortar line every sixteenth row and a
    // vertical joint offset by three rows, so the courses are staggered.
    let mut pixels = Vec::with_capacity(CHECK_DIMENSION * CHECK_DIMENSION * 4);
    for i in 0..CHECK_DIMENSION {
        for j in 0..CHECK_DIMENSION {
            // A full white line every sixteenth row is the mortar course,
            // and one vertical joint per course, offset three rows each time,
            // staggers the bricks.
            let c = if i % 16 == 15 || (j + 48 * (i / 16)).is_multiple_of(64) {
                255
            } else {
                102 + (random() % 102) as u8
            };
            pixels.extend_from_slice(&[c, c, c, 255]);
        }
    }
    this.brick = g.glx.gen_texture();
    g.glx.bind_texture(this.brick);
    g.glx
        .tex_image_2d(CHECK_DIMENSION as i32, CHECK_DIMENSION as i32, pixels);

    this.build_pyramid(g);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Providence {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 3;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = width as f32 / height as f32;
        g.glx.line_width(2.0);
        g.glx.point_size(2.0);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        // The wheel pushes the camera in and out.
        if let XEvent::ButtonPress { button, .. } = event {
            if *button == 4 {
                self.camera_velocity += 1.0;
                return true;
            }
            if *button == 5 {
                self.camera_velocity -= 1.0;
                return true;
            }
        }
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, self.aspect, 0.001, 25.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.light_enable(0, true);
        g.glx.light_ambient(0, [0.25, 0.25, 0.25, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_model_ambient([0.5, 0.5, 0.5, 1.0]);
        g.glx.lighting(!self.wire);
        g.glx.front_face_cw(false);
        g.glx.cull_face(true);
        g.glx.depth_test(true);
        g.glx.tex_env(TexEnv::Modulate);
        g.glx.blend(Blend::Off);
        g.glx.clear();

        g.glx.push_matrix();

        // The camera pulls back to arm's length and stays there.
        if self.camera_velocity.abs() > 0.0001 {
            self.camera_z = (self.camera_z + 0.1 * self.camera_velocity).clamp(-12.0, -4.0);
            self.camera_velocity *= 0.95;
        }

        g.glx
            .translate(0.0, 0.0, (self.camera_z + (self.theta / 4.0).sin()) as f32);
        g.glx.rotate(
            (10.0 + 20.0 * (self.theta / 2.0).sin()) as f32,
            1.0,
            0.0,
            0.0,
        );
        g.glx.mult_matrix(self.trackball.matrix());
        g.glx.rotate(
            (self.theta * 180.0 / std::f64::consts::PI) as f32,
            0.0,
            -1.0,
            0.0,
        );

        // The seal itself, standing on its base.
        g.glx.translate(0.0, 1.414, 0.0);
        g.glx.light_position(
            0,
            1.6 * self.theta.sin() as f32,
            1.2,
            1.6 * self.theta.cos() as f32,
            0.0,
        );
        g.glx.lighting(!self.wire);
        g.glx.blend(Blend::Off);
        g.glx.call_list(self.pyramid);
        self.draw_glory(g);
        if self.eye {
            if self.theta.cos() < 0.0 {
                self.draw_eye2(g);
            } else {
                self.draw_eye(g);
            }
        }

        g.glx.pop_matrix();

        self.currenttime += 1.0 / FPS;
        self.theta = self.currenttime / 2.0 * self.theta_scale;
        self.update_particles();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:     20000",
    "*showFPS:   False",
    "*wireframe: False",
    "*eye:       True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::boolean("eye", "Eye of providence", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "providence",
    label: "Providence",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Blair Tennessy",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=bnwRPPMopWc"),
        blurb: "A pyramid unfinished. In the zenith an eye in a triangle, \
                surrounded by a glory, proper.",
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

    /// Every glory particle starts on one of the three sides of the triangle
    /// the glory is drawn around.
    #[test]
    fn the_glory_starts_on_the_triangle() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let lines: Vec<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Lines)
            .collect();
        assert_eq!(lines.len(), 2, "the glory is drawn in two colours");
        let total: usize = lines.iter().map(|b| b.count).sum();
        assert_eq!(total, 2 * PARTICLE_COUNT, "one line per particle");

        // The first vertex of each pair is the particle itself, and it must
        // sit on an edge of the triangle with corners (+-0.75, -0.75) and
        // (0, 0.75).
        for b in &lines {
            for v in f.vertices[b.first..b.first + b.count].iter().step_by(2) {
                let (x, y) = (v.pos[0], v.pos[1]);
                let on_base = (y + 0.75).abs() < 0.001;
                // The two sloping sides have slope 2, through (0, 0.75).
                let on_side = ((y - 0.75).abs() - 2.0 * x.abs()).abs() < 0.01;
                assert!(on_base || on_side, "a particle is loose at ({x}, {y})");
            }
        }
    }

    /// The eye's particles crawl outwards from the pupil, and none of them
    /// ever gets past the rim.
    #[test]
    fn the_eye_particles_stay_inside_the_eye() {
        let inner = eye_point(0, 0);
        let outer = eye_point(0, EYELENGTH - 1);
        let r0 = (inner[0] * inner[0] + inner[1] * inner[1]).sqrt();
        let r1 = (outer[0] * outer[0] + outer[1] * outer[1]).sqrt();
        assert!((r0 - 0.07).abs() < 1e-6, "the pupil is {r0} across");
        assert!(r1 > r0 && r1 < 0.171, "the rim is at {r1}");

        // Every ray reaches the same distance for the same step, so the eye
        // is round rather than lopsided.
        let expect = 0.07 + 137.0 * 0.1 / EYELENGTH as f32;
        for i in [0usize, 1, 359, 719] {
            let p = eye_point(i, 137);
            let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
            assert!((r - expect).abs() < 1e-5, "ray {i} reaches {r}");
        }
    }

    /// Turning the seal past a quarter of the way round swaps the eye for the
    /// dollar sign, and the two are drawn from different numbers of batches.
    #[test]
    fn the_far_side_of_the_seal_shows_a_dollar_sign() {
        let points = |steps: usize| {
            let mut r = start(StartArgs::new(640, 480, "eye=true", 20260811));
            for _ in 0..steps {
                r.step();
            }
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Points)
                .count()
        };
        // The eye is drawn twice over, small and large, in two colours each.
        assert_eq!(points(1), 4);
        // A quarter turn takes theta past pi/2 at the default rate.
        let mut r = start(StartArgs::new(640, 480, "eye=true", 20260811));
        let mut swapped = 0;
        for _ in 0..600 {
            r.step();
            if r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Points)
                .count()
                == 2
            {
                swapped += 1;
            }
        }
        assert!(swapped > 0, "the dollar sign never came round");
    }

    #[test]
    fn the_camera_settles_at_arms_length() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..400 {
            r.step();
        }
        // It starts at -4 and drifts back to the far stop, and stays there.
        let z = r.frame().batches[0].modelview.0[14];
        assert!(z < -11.0 && z > -14.0, "the camera ended up at {z}");
    }

    #[test]
    fn the_brickwork_has_mortar_lines() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let tex = r.texture(1).expect("the brick texture");
        // Every sixteenth row is a full white mortar line.
        let row = |i: usize| {
            (0..CHECK_DIMENSION)
                .map(|j| tex.data[(i * CHECK_DIMENSION + j) * 4] as u32)
                .sum::<u32>()
        };
        assert_eq!(
            row(15),
            255 * CHECK_DIMENSION as u32,
            "row 15 is not mortar"
        );
        assert!(
            row(14) < 255 * CHECK_DIMENSION as u32,
            "row 14 is all white"
        );
    }
}
