//! Port of `hacks/glx/energystream.c`.
//!
//! ```text
//! energystream, Copyright (c) 2016 Eugene Sandulenko <sev@scummvm.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Based on Public Domain code by konrad "yoghurt" zagorowicz
//! for Tesla demo by Sunflower (http://www.pouet.net/prod.php?which=33)
//! ```
//!
//! Sixteen streams of glowing particles rushing past the camera, from a demo
//! for the Amiga scene in 2000.
//!
//! There is no particle system here in the usual sense. Each flare's position
//! is a closed-form function of the clock: its x is its starting offset plus
//! the elapsed time times the stream's speed, taken modulo eight hundred, so
//! the stream wraps rather than being respawned; and its y and z wobble on a
//! sine and a cosine of the same clock. Nothing is integrated and nothing
//! accumulates, which is why it can restart the clock every sixty seconds and
//! the picture simply continues.
//!
//! A flare is one quad facing the camera. The two axes it is built on come
//! from inverting the modelview: the first two columns of the inverse are the
//! camera's right and up in world space, so adding and subtracting them makes
//! a quad square on the screen however the scene is turned. That is the whole
//! billboard trick, and it costs one matrix inverse a frame rather than one
//! per particle.
//!
//! Two texture details carry the look. The texture environment is `GL_ADD`
//! rather than the usual multiply, so overlapping flares pile up into white
//! instead of darkening each other. And the blend is additive on top of that,
//! which is what makes a dense stream glow.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Fog, Mat4, Shape, TexEnv};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
};

const TEX_WIDTH: i32 = 256;
const TEX_HEIGHT: i32 = 256;
/// How sharply a flare's glow falls off from its middle.
const COEFF: f64 = 0.2;

/// When the camera changes its mind, in seconds. The last one restarts the
/// clock, which is the whole of the loop.
const CHANGE_TIME1: f32 = 25.0;
const CHANGE_TIME2: f32 = 40.0;
const CHANGE_TIME3: f32 = 60.0;

/// How many flares in a stream.
const FLARES: usize = 150;

/// Upstream spreads the flares of a stream by this much per flare, and wrote
/// it as a rounded pi rather than the real one. Using the real one would shift
/// every flare in every stream, so it stays as written.
#[allow(clippy::approx_constant)]
const ROUGH_PI: f32 = 3.14;

/// The sixteen streams: where each one runs and how fast. Upstream writes
/// these out one call at a time and would walk off the end of its array if the
/// stream count were anything but sixteen, so the count is not a knob here.
const STREAMS: [(f32, f32, f32); 16] = [
    (0.0, 50.0, 300.0),
    (0.0, 0.0, 150.0),
    (90.0, 60.0, 250.0),
    (-100.0, 30.0, 160.0),
    (50.0, -100.0, 340.0),
    (-50.0, 50.0, 270.0),
    (100.0, 50.0, 180.0),
    (-30.0, 90.0, 130.0),
    (150.0, 10.0, 200.0),
    (100.0, -100.0, 210.0),
    (190.0, 160.0, 220.0),
    (-200.0, 130.0, 230.0),
    (150.0, -200.0, 240.0),
    (-150.0, 250.0, 160.0),
    (200.0, 150.0, 230.0),
    (-130.0, 190.0, 250.0),
];

struct FlareStream {
    flares: Vec<[f32; 3]>,
    flare_tex: u32,
    speed: f32,
}

struct EnergyStream {
    rot: Rotator,
    trackball: Trackball,
    /// When the clock was last restarted, in saver seconds.
    start: f64,
    streams: Vec<FlareStream>,
    speed: f32,
}

/// A round glow, tinted a random colour. Each stream gets its own, which is
/// why the streams are different colours.
fn gen_texture(g: &mut Gl) -> u32 {
    let tint: [f64; 3] = [frand(1.0), frand(1.0), frand(1.0)];
    let mut texture = Vec::with_capacity((TEX_WIDTH * TEX_HEIGHT * 4) as usize);
    for y in 0..TEX_HEIGHT {
        for x in 0..TEX_WIDTH {
            let dx = f64::from(x - TEX_WIDTH / 2);
            let dy = f64::from(y - TEX_HEIGHT / 2);
            let color = 255.0 - (dx * dx / COEFF + dy * dy / COEFF).sqrt();
            let color = color.max(0.0);
            for t in tint {
                texture.push((color * t) as u8);
            }
            texture.push(if color != 0.0 { 255 } else { 0 });
        }
    }

    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    g.glx.tex_image_2d(TEX_WIDTH, TEX_HEIGHT, texture);
    id
}

/// The camera's right and up axes in world space, ten units long: the first
/// two columns of the inverse of the modelview's rotation.
fn billboard_axes(m: &Mat4) -> ([f32; 3], [f32; 3]) {
    // Row `i`, column `j` of the upper 3x3, out of GL's column-major layout.
    let e = |i: usize, j: usize| m.0[j * 4 + i];
    let c00 = e(1, 1) * e(2, 2) - e(1, 2) * e(2, 1);
    let c01 = -(e(1, 0) * e(2, 2) - e(1, 2) * e(2, 0));
    let c02 = e(1, 0) * e(2, 1) - e(1, 1) * e(2, 0);
    let c10 = -(e(0, 1) * e(2, 2) - e(0, 2) * e(2, 1));
    let c11 = e(0, 0) * e(2, 2) - e(0, 2) * e(2, 0);
    let c12 = -(e(0, 0) * e(2, 1) - e(0, 1) * e(2, 0));

    let det = e(0, 0) * c00 + e(0, 1) * c01 + e(0, 2) * c02;
    if det == 0.0 {
        return ([10.0, 0.0, 0.0], [0.0, 10.0, 0.0]);
    }
    let s = 10.0 / det;
    ([c00 * s, c01 * s, c02 * s], [c10 * s, c11 * s, c12 * s])
}

impl EnergyStream {
    fn render_flare_stream(&self, g: &mut Gl, i: usize, cur_time: f32, vx: [f32; 3], vy: [f32; 3]) {
        let s = &self.streams[i];
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        g.glx.bind_texture(s.flare_tex);

        // The streams speed up twice as the clock runs on, which is most of
        // what makes the piece build.
        let mult = if cur_time > CHANGE_TIME2 {
            2.5
        } else if cur_time > CHANGE_TIME1 {
            2.0
        } else {
            1.0
        };

        g.glx.begin(Shape::Quads);
        for (j, f) in s.flares.iter().enumerate() {
            let x = (f[0] + cur_time * s.speed * mult) % 800.0 - 400.0;
            let y = f[1] + 2.0 * (cur_time * 7.0 + f[0]).sin();
            let z = f[2] + 2.0 * (cur_time * 7.0 + j as f32 * ROUGH_PI).cos();

            for (u, v, sx, sy) in [
                (0.0, 0.0, -1.0, 1.0),
                (1.0, 0.0, 1.0, 1.0),
                (1.0, 1.0, 1.0, -1.0),
                (0.0, 1.0, -1.0, -1.0),
            ] {
                g.glx.tex_coord2f(u, v);
                g.glx.vertex3f(
                    x + sx * vx[0] + sy * vy[0],
                    y + sx * vx[1] + sy * vy[1],
                    z + sx * vx[2] + sy * vy[2],
                );
            }
        }
        g.glx.end();
    }
}

impl Hack3d for EnergyStream {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let cur_time = ((g.time - self.start) as f32) * self.speed;

        g.glx.clear();

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-0.6, 0.6, -0.45, 0.45, 1.0, 1000.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        // Lighting on with no lights: the flares come out at the scene
        // ambient, and the texture is added on top of that. Upstream's, and it
        // is why they are as bright as they are.
        g.glx.lighting(true);
        g.glx.texturing(true);
        g.glx.tex_env(TexEnv::Add);
        g.glx.blend(Blend::AlphaAdd);
        g.glx.cull_face(false);
        g.glx.depth_test(false);

        g.glx.translate(0.0, 0.0, -300.0);
        g.glx.rotate(cur_time * 30.0, 1.0, 0.0, 0.0);
        g.glx
            .rotate(30.0 * (cur_time / 3.0).sin() + 10.0, 0.0, 0.0, 1.0);

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

        if cur_time > CHANGE_TIME2 {
            g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            if cur_time > CHANGE_TIME3 {
                // Back to five seconds in rather than to zero, so the loop
                // does not stall on the sparse opening.
                self.start = g.time - 5.0;
            }
        } else if cur_time > CHANGE_TIME1 {
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        }

        g.glx.fog(Some(Fog::Linear {
            start: 200.0,
            end: 500.0,
            color: [0.0, 0.0, 0.0, 0.0],
        }));

        let (vx, vy) = billboard_axes(&g.glx.modelview());

        for i in 0..self.streams.len() {
            self.render_flare_stream(g, i, cur_time, vx, vy);
        }

        g.glx.texturing(false);
        g.glx.lighting(false);
        g.glx.fog(None);
        g.glx.tex_env(TexEnv::Modulate);

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width.max(1) as f32;

        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let speed = g.res.float("speed") as f32;
    let spin = g.res.bool("spin");
    let wander = g.res.bool("wander");
    let spin_speed = 0.5 * f64::from(speed);
    let wander_speed = 0.02 * f64::from(speed);
    let spin_accel = 1.1;

    let mut st = EnergyStream {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            spin_accel,
            if wander { wander_speed } else { 0.0 },
            true,
        ),
        trackball: Trackball::new(),
        start: 0.0,
        streams: Vec::with_capacity(STREAMS.len()),
        speed,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    for (by, bz, stream_speed) in STREAMS {
        let flare_tex = gen_texture(g);
        let flares = (0..FLARES)
            .map(|_| {
                [
                    (-800.0 * frand(1.0) - 1150.0) as f32,
                    (10.0 * frand(1.0) - 20.0) as f32 + by,
                    (10.0 * frand(1.0) - 20.0) as f32 + bz,
                ]
            })
            .collect();
        st.streams.push(FlareStream {
            flares,
            flare_tex,
            speed: stream_speed,
        });
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        30",
    "*showFPS:      False",
    "*wireframe:    False",
    "*spin:         False",
    "*wander:       False",
    "*speed:        1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 5.0, 0.01, 2, "1.0"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("spin", "Spin", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "energystream",
    label: "Energy Stream",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Eugene Sandulenko and Konrad \"Yoghurt\" Zagorowicz",
        year: "2016",
        video: Some("https://www.youtube.com/watch?v=TbWZ6v5Zzk8"),
        blurb: "A flow of particles which form an energy stream.",
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

    /// Inverting a rotation gives its transpose, so the billboard axes come
    /// out as the rows of the rotation rather than its columns. Getting that
    /// the wrong way round gives quads that turn with the scene instead of
    /// facing the camera.
    #[test]
    fn the_billboard_axes_face_the_camera() {
        // A quarter turn about y: the camera's right in world space is -z.
        let m = Mat4([
            0.0, 0.0, -1.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ]);
        let (vx, vy) = billboard_axes(&m);
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-4);
        assert!(close(vx, [0.0, 0.0, 10.0]), "{vx:?}");
        assert!(close(vy, [0.0, 10.0, 0.0]), "{vy:?}");
        // And the identity gives the plain screen axes.
        let (vx, vy) = billboard_axes(&Mat4::IDENTITY);
        assert!(close(vx, [10.0, 0.0, 0.0]) && close(vy, [0.0, 10.0, 0.0]));
    }

    /// Every quad is square on the screen and the same size, whatever the
    /// scene is doing, since all four corners are the same two axes added and
    /// subtracted.
    #[test]
    fn every_flare_is_the_same_square() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        assert!(!f.batches.is_empty());
        let b = &f.batches[0];
        // Quads become pairs of triangles; the first three vertices are three
        // corners of the first flare.
        let vs = &f.vertices[b.first..b.first + 3];
        let d = |a: [f32; 3], c: [f32; 3]| {
            ((a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2) + (a[2] - c[2]).powi(2)).sqrt()
        };
        let side = d(vs[0].pos, vs[1].pos);
        assert!((side - 20.0).abs() < 1e-3, "a flare is {side} across");
        assert!((d(vs[1].pos, vs[2].pos) - 20.0).abs() < 1e-3);
    }

    /// Each stream has its own texture, which is why they are different
    /// colours, and every one of them is a round glow that fades to nothing
    /// before its edge.
    #[test]
    fn each_stream_glows_in_its_own_colour() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let ids: std::collections::BTreeSet<u32> =
            r.frame().batches.iter().filter_map(|b| b.texture).collect();
        assert_eq!(ids.len(), STREAMS.len(), "one texture a stream");

        let t = r.texture(*ids.iter().next().unwrap()).unwrap();
        assert_eq!((t.width, t.height), (TEX_WIDTH, TEX_HEIGHT));
        let at = |x: i32, y: i32| {
            let i = ((y * TEX_WIDTH + x) * 4) as usize;
            (t.data[i], t.data[i + 3])
        };
        // Brightest in the middle, gone in the corner.
        assert!(at(TEX_WIDTH / 2, TEX_HEIGHT / 2).1 == 255);
        assert_eq!(at(0, 0), (0, 0));
        assert!(at(TEX_WIDTH / 2, TEX_HEIGHT / 2).0 > at(TEX_WIDTH / 2 + 40, TEX_HEIGHT / 2).0);
    }

    /// The flares wrap round rather than being respawned, so however long it
    /// runs they stay in the same eight hundred units of space.
    #[test]
    fn the_stream_wraps_rather_than_running_out() {
        let mut r = start(StartArgs::new(640, 480, "speed=5", 20260811));
        // They start well behind the camera and stream in, so the range is
        // the eight hundred units of the wrap plus the four hundred they
        // begin behind it, and a flare's own twenty of width.
        let mut wrapped = false;
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            let lo = f.vertices.iter().map(|v| v.pos[0]).fold(f32::MAX, f32::min);
            let hi = f.vertices.iter().map(|v| v.pos[0]).fold(f32::MIN, f32::max);
            assert!(lo > -1250.0 && hi < 450.0, "a flare escaped to {lo}..{hi}");
            if lo > -450.0 {
                wrapped = true;
            }
        }
        assert!(wrapped, "the stream never wrapped round");
    }
}
