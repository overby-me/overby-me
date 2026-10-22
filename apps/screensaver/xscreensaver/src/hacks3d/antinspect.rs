//! Port of `hacks/glx/antinspect.c`.
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
//! tennessy@cs.ubc.ca
//! ```
//!
//! Ants move spheres around a circle.
//!
//! Three ants, each clinging to the underside of a translucent bubble, ride
//! round a triangular plate. An ant is a few wireframe spheres for its body
//! and a handful of coloured line strips for its legs and antennae, and its
//! legs walk because the six of them are sampled off one sine at a third of a
//! turn apart.
//!
//! The shadows are the interesting part and cost no depth buffer: the whole
//! scene is drawn a second time, flat grey, through a matrix that projects
//! every point onto the ground plane along the ray from the light. That is
//! four lines of arithmetic in [`shadow_matrix`] and it works for any geometry,
//! which is why the shadow of a wireframe ant is a wireframe ant.
//!
//! Because everything is translucent and nothing is depth-sorted by the
//! hardware, the three ants have to be drawn back to front, and which order
//! that is comes off a table of the six permutations indexed by where the
//! second ant has got to.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Mat4, Shape};
use crate::runtime::shapes::unit_sphere;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
};

/// Only works with 3 right now.
const ANTCOUNT: usize = 3;

const MATERIAL_RED: [f32; 4] = [0.6, 0.0, 0.0, 1.0];
const MATERIAL_BEN: [f32; 4] = [0.25, 0.30, 0.46, 1.0];
const MATERIAL_ORANGE: [f32; 4] = [1.0, 0.69, 0.00, 1.0];
const MATERIAL_GRAY: [f32; 4] = [0.2, 0.2, 0.2, 1.0];
const MATERIAL_BLACK: [f32; 4] = [0.1, 0.1, 0.1, 0.4];
const MATERIAL_SHADOW: [f32; 4] = [0.3, 0.3, 0.3, 0.3];

const ANT_MATERIAL: [[f32; 4]; ANTCOUNT] = [MATERIAL_RED, MATERIAL_BEN, MATERIAL_ORANGE];
const ANT_VELOCITY: [f64; ANTCOUNT] = [0.3, 0.3, 0.3];
const ANT_SPHERE: [f32; ANTCOUNT] = [1.2, 1.2, 1.2];

/// The six orders three ants can be drawn in. Which one is used depends on
/// where the second ant has got to, so that whichever is furthest away is
/// drawn first and the translucency stacks up the right way.
const ANT_ORDER: [[usize; ANTCOUNT]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [2, 0, 1],
    [2, 1, 0],
    [1, 2, 0],
    [1, 0, 2],
];

/// The plane the shadows fall on: the ground, a hair below zero.
const GROUND: [f32; 4] = [0.0, 1.0, 0.0, -0.00001];

/// The matrix that flattens everything onto `plane` along the rays from
/// `light`. Multiply it into the modelview and draw the scene again, and what
/// comes out is its shadow.
fn shadow_matrix(plane: [f32; 4], light: [f32; 4]) -> Mat4 {
    let dot = plane[0] * light[0] + plane[1] * light[1] + plane[2] * light[2] + plane[3] * light[3];
    let mut m = [0.0f32; 16];
    for i in 0..4 {
        for j in 0..4 {
            m[i * 4 + j] = if i == j { dot } else { 0.0 } - light[j] * plane[i];
        }
    }
    Mat4(m)
}

/// `mySphere`: a filled sphere of this radius.
fn my_sphere(g: &mut Gl, radius: f32) {
    g.glx.push_matrix();
    g.glx.scale(radius, radius, radius);
    g.glx.rotate(90.0, 1.0, 0.0, 0.0);
    unit_sphere(&mut g.glx, 16, 16, false);
    g.glx.pop_matrix();
}

/// `mySphere2`: a caged one, which is what every part of an ant is made of.
fn my_sphere2(g: &mut Gl, radius: f32) {
    g.glx.push_matrix();
    g.glx.scale(radius, radius, radius);
    g.glx.rotate(90.0, 1.0, 0.0, 0.0);
    unit_sphere(&mut g.glx, 16, 8, true);
    g.glx.pop_matrix();
}

struct AntInspect {
    trackball: Trackball,
    shadows: bool,
    /// The phase the legs are walking at, and what tilts the camera.
    ant_step: f32,
    /// How far round the plate each ant has got, in degrees.
    ant_position: [f64; ANTCOUNT],
}

impl AntInspect {
    /// One ant: three wireframe spheres for the body, then the antennae and the
    /// six legs as flat-coloured lines.
    ///
    /// Upstream passes in which sphere to use and which cone; the cone draws
    /// nothing at all, and the sphere is the caged one both times it is called,
    /// so both are written out here. The transforms around the two absent cones
    /// are not: they carry to the sphere after them.
    fn draw_ant(&self, g: &mut Gl, material: [f32; 4]) {
        let step = self.ant_step;
        let tau = std::f32::consts::PI * 2.0;
        let cos1 = step.cos();
        let cos2 = (step + tau / 3.0).cos();
        let cos3 = (step + 2.0 * tau / 3.0).cos();
        let sin1 = step.sin();
        let sin2 = (step + tau / 3.0).sin();
        let sin3 = (step + 2.0 * tau / 3.0).sin();

        g.glx.material_ambient_diffuse(material);
        g.glx.cull_face(true);
        g.glx.push_matrix();
        g.glx.scale(1.0, 1.3, 1.0);
        my_sphere2(g, 0.18);
        g.glx.scale(1.0, 1.0 / 1.3, 1.0);
        g.glx.translate(0.00, 0.30, 0.00);
        my_sphere2(g, 0.2);

        // Where the two absent cones would have been.
        g.glx.translate(-0.05, 0.17, 0.05);
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(-25.0, 0.0, 1.0, 0.0);
        g.glx.translate(0.00, 0.10, 0.00);
        g.glx.rotate(25.0, 0.0, 1.0, 0.0);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);

        g.glx.scale(1.0, 1.3, 1.0);
        g.glx.translate(0.15, -0.65, 0.05);
        my_sphere2(g, 0.25);
        g.glx.scale(1.0, 1.0 / 1.3, 1.0);
        g.glx.pop_matrix();
        g.glx.cull_face(false);

        g.glx.lighting(false);

        let c = |g: &mut Gl, v: [f32; 4]| g.glx.color4f(v[0], v[1], v[2], v[3]);

        // Antennae: from the head out, fading to grey at the tip.
        g.glx.begin(Shape::Lines);
        for z in [0.40, -0.40] {
            c(g, material);
            g.glx.vertex3f(0.00, 0.30, 0.00);
            c(g, MATERIAL_GRAY);
            g.glx.vertex3f(0.40, 0.70, z);
        }
        g.glx.end();
        g.glx.begin(Shape::Points);
        c(g, material);
        g.glx.vertex3f(0.40, 0.70, 0.40);
        g.glx.vertex3f(0.40, 0.70, -0.40);
        g.glx.end();

        // Six legs, three a side, each sampled off the same walk cycle a third
        // of a turn apart. The right-hand three read the sine where the
        // left-hand three read the cosine, so the ant walks rather than hops.
        for (y0, y1, z, knee, foot) in [
            (0.05, 0.15, 0.18, 0.05 * cos1, 0.25 + 0.1 * sin1),
            (0.00, 0.00, 0.18, 0.05 * cos2, 0.00 + 0.1 * sin2),
            (-0.05, -0.15, 0.18, 0.05 * cos3, -0.25 + 0.1 * sin3),
            (0.05, 0.15, -0.18, -0.05 * sin1, 0.25 + 0.1 * cos1),
            (0.00, 0.00, -0.18, -0.05 * sin2, 0.00 + 0.1 * cos2),
            (-0.05, -0.15, -0.18, -0.05 * sin3, -0.25 + 0.1 * cos3),
        ] {
            let far = if z > 0.0 { 0.45 } else { -0.45 };
            g.glx.begin(Shape::LineStrip);
            c(g, material);
            g.glx.vertex3f(0.00, y0, z);
            g.glx.vertex3f(0.35 + knee, y1, z + 0.07 * z.signum());
            c(g, MATERIAL_GRAY);
            g.glx.vertex3f(-0.20 + knee, foot, far);
            g.glx.end();
        }

        g.glx.lighting(true);
    }

    /// The plate, the shadows and then the ants.
    fn draw_strip(&mut self, g: &mut Gl) {
        // Which way round to draw them, from where the second one has got to.
        let ro = (self.ant_position[1] as i32 / (360 / (2 * ANTCOUNT as i32)))
            .rem_euclid(2 * ANTCOUNT as i32) as usize;

        // The light is put up high before anything is drawn, and the shadows
        // are cast from where it ends up.
        let position0 = [0.0, 9.6, 0.0, 1.0];
        g.glx
            .light_position(0, position0[0], position0[1], position0[2], position0[3]);

        g.glx.rotate(-30.0, 0.0, 1.0, 0.0);
        g.glx.blend(Blend::Off);

        // The plate: a triangle in the middle and three more outside it, each
        // laid down by turning a third of the way round and drawing the same
        // one further out.
        let r3 = 3.0f32.sqrt() / 2.0;
        g.glx.color4f(
            MATERIAL_SHADOW[0],
            MATERIAL_SHADOW[1],
            MATERIAL_SHADOW[2],
            MATERIAL_SHADOW[3],
        );
        g.glx.material_ambient_diffuse(MATERIAL_BLACK);
        g.glx.begin(Shape::Triangles);
        g.glx.normal3f(0.0, 1.0, 0.0);
        g.glx.vertex3f(0.0, 0.0, -1.0);
        g.glx.vertex3f(-r3, 0.0, 0.5);
        g.glx.vertex3f(r3, 0.0, 0.5);
        g.glx.end();

        for _ in 0..3 {
            g.glx.rotate(120.0, 0.0, 1.0, 0.0);
            g.glx.begin(Shape::Triangles);
            g.glx.vertex3f(0.0, 0.0, 1.0 + 3.0);
            g.glx.vertex3f(r3, 0.0, -0.5 + 3.0);
            g.glx.vertex3f(-r3, 0.0, -0.5 + 3.0);
            g.glx.end();
        }

        // The shadows first, since they need no depth test of their own: they
        // are the same scene flattened onto the ground.
        if self.shadows {
            let m = shadow_matrix(GROUND, position0);
            g.glx.color4f(
                MATERIAL_SHADOW[0],
                MATERIAL_SHADOW[1],
                MATERIAL_SHADOW[2],
                MATERIAL_SHADOW[3],
            );
            g.glx.blend(Blend::Off);
            g.glx.lighting(false);

            g.glx.push_matrix();
            g.glx.translate(0.0, 0.001, 0.0);
            g.glx.mult_matrix(m);

            for i in 0..ANTCOUNT {
                g.glx.push_matrix();
                self.place_ant(g, i);

                if self.ant_position[i] > 360.0 {
                    self.ant_position[i] = 0.0;
                }
                self.draw_ant(g, MATERIAL_SHADOW);

                g.glx.blend(Blend::Off);
                g.glx.lighting(false);

                g.glx.rotate(-20.0, 1.0, 0.0, 0.0);
                g.glx.rotate(-self.ant_step * 2.0, 0.0, 0.0, 1.0);
                g.glx.material_ambient_diffuse(MATERIAL_SHADOW);
                my_sphere2(g, 1.2);
                g.glx.pop_matrix();
            }
            g.glx.pop_matrix();
        }

        g.glx.lighting(true);

        for &i in &ANT_ORDER[ro] {
            g.glx.push_matrix();

            // Round the plate, then out to the rim and up onto the bubble.
            g.glx.rotate(self.ant_position[i] as f32, 0.0, 1.0, 0.0);
            g.glx.translate(2.4, 0.0, 0.0);
            g.glx.translate(0.0, ANT_SPHERE[i], 0.0);
            g.glx.rotate(90.0, 0.0, 1.0, 0.0);

            g.glx.push_matrix();
            self.hang_ant(g);
            if self.ant_position[i] > 360.0 {
                self.ant_position[i] = 0.0;
            }
            g.glx.blend(Blend::Alpha);
            self.draw_ant(g, ANT_MATERIAL[i]);
            g.glx.blend(Blend::Off);
            g.glx.pop_matrix();

            // The bubble: a cage in the ant's colour with a dark translucent
            // ball just inside it.
            g.glx.rotate(-20.0, 1.0, 0.0, 0.0);
            g.glx.rotate(-self.ant_step * 2.0, 0.0, 0.0, 1.0);
            g.glx.material_ambient_diffuse(ANT_MATERIAL[i]);
            my_sphere2(g, 1.2);
            g.glx.blend(Blend::Alpha);
            g.glx.material_ambient_diffuse(MATERIAL_BLACK);
            my_sphere(g, 1.16);
            g.glx.blend(Blend::Off);

            g.glx.pop_matrix();

            self.ant_position[i] += ANT_VELOCITY[i];
        }

        // But the step size is the same!
        self.ant_step += 0.2;
    }

    /// Where ant `i` sits: round the plate, out to the rim, up onto its bubble
    /// and then hanging off the underside of it.
    fn place_ant(&self, g: &mut Gl, i: usize) {
        g.glx.rotate(self.ant_position[i] as f32, 0.0, 1.0, 0.0);
        g.glx.translate(2.4, 0.0, 0.0);
        g.glx.translate(0.0, ANT_SPHERE[i], 0.0);
        g.glx.rotate(90.0, 0.0, 1.0, 0.0);
        self.hang_ant(g);
    }

    /// The turn that puts an ant on the outside of its bubble, upside down.
    fn hang_ant(&self, g: &mut Gl) {
        g.glx.rotate(10.0, 0.0, 1.0, 0.0);
        g.glx.rotate(40.0, 0.0, 0.0, 1.0);
        g.glx.translate(0.0, -0.8, 0.0);
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        g.glx.rotate(90.0, 0.0, 0.0, 1.0);
    }
}

impl Hack3d for AntInspect {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.push_matrix();

        // Close enough in to see inside a bubble.
        g.glx.translate(0.0, 0.0, -10.0);
        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.rotate(
            15.0 / 2.0 + 15.0 * (self.ant_step / 100.0).sin(),
            1.0,
            0.0,
            0.0,
        );
        g.glx.rotate(30.0, 1.0, 0.0, 0.0);
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);

        self.draw_strip(g);
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = f64::from(height) / f64::from(width.max(1));
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = f64::from(height) / f64::from(width);
        }

        // One pixel per five hundred of width, which is what makes the legs
        // thicker on a big window. Only the points get it: WebGL draws every
        // line one pixel wide whatever it is told.
        let linewidth = (width / 512 + 1) as f32;

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, (1.0 / h) as f32, 7.0, 20.0);
        g.glx.matrix_mode_modelview();
        g.glx.line_width(linewidth);
        g.glx.point_size(linewidth);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut st = AntInspect {
        trackball: Trackball::new(),
        shadows: g.res.bool("shadows"),
        ant_step: 0.0,
        ant_position: [0.0, 120.0, 240.0],
    };
    // Upstream seeds two counters here that nothing ever reads back.
    let _ = random() % 90;

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    // Two lights, one overhead and one from below and behind, and half the
    // scene lit by neither of them.
    g.glx.lighting(true);
    for (i, pos) in [[0.0, 3.0, 0.0, 1.0], [-1.0, -3.0, 1.0, 0.0]]
        .into_iter()
        .enumerate()
    {
        g.glx.light_enable(i, true);
        g.glx.light_position(i, pos[0], pos[1], pos[2], pos[3]);
        g.glx.light_ambient(i, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(i, [1.0, 1.0, 1.0, 1.0]);
    }
    g.glx.light_model_ambient([0.5, 0.5, 0.5, 1.0]);
    // Upstream sets only GL_DIFFUSE on each object and leaves GL_AMBIENT at
    // OpenGL's grey default; the runtime carries one colour for both, so an
    // object's ambient here is its own colour rather than that grey. The
    // difference is a slightly more saturated dark side.
    g.glx.material_specular([0.7, 0.7, 0.7, 1.0]);
    g.glx.material_shininess(60.0);
    g.glx.depth_test(true);
    g.glx.front_face_cw(false);

    Box::new(st)
}

const DEFAULTS: &[&str] = &["*delay:   20000", "*showFPS: False", "*shadows: True"];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::boolean("shadows", "Draw shadows", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "antinspect",
    label: "Ant Inspect",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Blair Tennessy",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=Ecw9dDc0db0"),
        blurb: "Ants move spheres around a circle.",
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

    /// The shadow matrix flattens everything onto the ground plane: whatever
    /// point goes in, what comes out is on `y = 0`, on the line from the light
    /// through the point.
    #[test]
    fn the_shadow_matrix_lands_on_the_ground() {
        let light = [0.0, 9.6, 0.0, 1.0];
        let m = shadow_matrix(GROUND, light);

        for p in [
            [1.0f32, 2.0, 3.0],
            [-2.5, 5.0, 0.5],
            [0.0, 1.0, 0.0],
            [3.0, 0.5, -4.0],
        ] {
            let s = m.transform(p);
            assert!(s[1].abs() < 1e-3, "{p:?} cast a shadow at height {}", s[1]);

            // And it is where the ray from the light through the point meets
            // the ground: at t = ly / (ly - py) along it.
            let t = light[1] / (light[1] - p[1]);
            for k in [0, 2] {
                let want = light[k] + t * (p[k] - light[k]);
                assert!(
                    (s[k] - want).abs() < 1e-3,
                    "{p:?} cast at {s:?}, not through the light"
                );
            }
        }
    }

    /// Three ants, each drawn once, and each of them made of the same parts.
    #[test]
    fn three_ants_ride_three_bubbles() {
        let r = run("shadows=false", 5);
        let f = r.frame();

        // A leg is a three-point strip and there are six of them an ant.
        let legs = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::LineStrip && b.count == 3)
            .count();
        assert_eq!(legs, 6 * ANTCOUNT, "{legs} legs is not three ants");

        // Two antennae a head, drawn as one two-line block.
        let antennae = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Lines && b.count == 4)
            .count();
        assert_eq!(antennae, ANTCOUNT);
    }

    /// Turning the shadows on draws the whole scene a second time, and every
    /// bit of the second copy is flat on the ground.
    #[test]
    fn shadows_double_the_scene_and_lie_flat() {
        let plain = run("shadows=false", 5).frame().batches.len();
        let r = run("shadows=true", 5);
        let f = r.frame();
        assert!(
            f.batches.len() > plain,
            "shadows drew nothing extra: {} vs {plain}",
            f.batches.len()
        );

        // The shadow copies are the unlit grey ones. Flattened means they all
        // lie in one plane, which is what to check: the camera has the ground
        // at an angle, so "on the ground" is not "at y = 0" by the time these
        // are in eye space.
        let mut points = Vec::new();
        for b in f
            .batches
            .iter()
            .filter(|b| !b.lighting && b.material.ambient_diffuse == MATERIAL_SHADOW)
        {
            for v in &f.vertices[b.first..b.first + b.count] {
                points.push(b.modelview.transform(v.pos));
            }
        }
        assert!(points.len() > 100, "only {} shadow vertices", points.len());

        let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let len = |a: [f32; 3]| dot(a, a).sqrt();

        // Three points as far from collinear as the set offers give the plane.
        let p0 = points[0];
        let a = points
            .iter()
            .copied()
            .max_by(|x, y| len(sub(*x, p0)).total_cmp(&len(sub(*y, p0))))
            .unwrap();
        let b = points
            .iter()
            .copied()
            .max_by(|x, y| {
                len(cross(sub(a, p0), sub(*x, p0))).total_cmp(&len(cross(sub(a, p0), sub(*y, p0))))
            })
            .unwrap();
        let n = cross(sub(a, p0), sub(b, p0));
        let nl = len(n);
        assert!(nl > 1e-3, "the shadows are a line, not a plane");
        let n = [n[0] / nl, n[1] / nl, n[2] / nl];

        for p in &points {
            assert!(
                dot(sub(*p, p0), n).abs() < 0.02,
                "a shadow is {} off the plane of the others",
                dot(sub(*p, p0), n)
            );
        }
    }

    /// The legs walk: the six of them are one sine sampled a third of a turn
    /// apart, so they move and they do not all move together.
    #[test]
    fn the_legs_walk() {
        let mut r = start(StartArgs::new(640, 480, "shadows=false", 20260811));
        let mut feet: Vec<Vec<[f32; 3]>> = Vec::new();
        for _ in 0..8 {
            r.step();
            let f = r.frame();
            feet.push(
                f.batches
                    .iter()
                    .filter(|b| b.primitive == Primitive::LineStrip && b.count == 3)
                    .take(6)
                    .map(|b| f.vertices[b.first + 2].pos)
                    .collect(),
            );
        }

        // Every foot moves between frames.
        for (k, (was, now)) in feet[0].iter().zip(&feet[1]).enumerate() {
            assert_ne!(was, now, "leg {k} is not walking");
        }
        // And they are not all in the same place as each other.
        let first = feet[0][0];
        assert!(
            feet[0].iter().any(|p| *p != first),
            "all six legs are in step"
        );
    }

    /// The draw order comes off the permutation table, so which ant is drawn
    /// first changes as they go round; that is what keeps the translucent
    /// bubbles stacking up correctly.
    #[test]
    fn the_ants_are_reordered_as_they_go_round() {
        let mut r = start(StartArgs::new(640, 480, "shadows=false", 20260811));
        let mut orders = std::collections::BTreeSet::new();
        for _ in 0..900 {
            r.step();
            let f = r.frame();
            // The colour of the first ant drawn says which one it is.
            if let Some(b) = f
                .batches
                .iter()
                .find(|b| b.lighting && ANT_MATERIAL.contains(&b.material.ambient_diffuse))
            {
                orders.insert(b.material.ambient_diffuse.map(f32::to_bits));
            }
        }
        assert_eq!(orders.len(), ANTCOUNT, "the order never changed");
    }
}
