//! Port of `hacks/glx/superquadrics.c`.
//!
//! ```text
//! superquadrics --- 3D mathematical shapes
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
//! Superquadrics were invented by Dr. Alan Barr of Caltech University.
//! They were first published in "Computer Graphics and Applications",
//! volume 1, number 1, 1981, in the article "Superquadrics and Angle-
//! Preserving Transformations."
//!
//! Ed Mackey
//! ```
//!
//! A surface of revolution with an exponent in it, turning slowly and morphing
//! into the next one.
//!
//! A superquadric is an ellipsoid whose sine and cosine terms are each raised
//! to a power. At an exponent of one it is a sphere; below one the surface
//! draws in towards the axes and becomes an octahedron, a starfish, a pinched
//! spindle; above one it swells out towards a box. Two exponents, one for each
//! angle, so the two directions can be doing different things at once.
//!
//! Raising a negative number to a fractional power is undefined, and the
//! surface needs it constantly, so upstream's `XtoY` takes the magnitude,
//! exponentiates that, and puts the sign back. That is not the real function,
//! and the shape is what it is because of the substitute.
//!
//! Three modes, and they are not separate shapes but one number: the surface is
//! generated from a `Mode` that runs continuously from 1 to 3, and the whole
//! parameterisation slides with it from a closed quadric to a torus. Everything
//! else slides too, because a new shape is not switched to but eased into over
//! `cycles` frames, exponents, colours, pitch and bank all interpolating at
//! once.
//!
//! The colours are two shades chosen at random and laid over the mesh in one of
//! four patterns: plain, striped along either direction, or a checkerboard,
//! which is what makes the surface read as a solid rather than a shape.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand};

const MAX_RES: usize = 50;
const MIN_RES: usize = 5;

/// The exponent function clamps here, so a spike does not run off to infinity
/// and take the normals with it.
const CLIP_NORMALS: f64 = 10000.0;

/// The four colour patterns, indexed by a two-bit toggle built from the parity
/// of the two mesh directions: plain, striped one way, striped the other, and
/// a checkerboard.
const PATS: [[bool; 4]; 4] = [
    [false, false, false, false],
    [false, true, false, true],
    [false, false, true, true],
    [false, true, true, false],
];

/// A shape: the two exponents, the four corner colours, the mode, and how it
/// is turned.
#[derive(Clone, Copy)]
struct State {
    x_exponent: f64,
    y_exponent: f64,
    rgb: [[f32; 4]; 4],
    mode: i64,
    rotx: i32,
    rotz: i32,
}

impl Default for State {
    fn default() -> Self {
        State {
            x_exponent: 1.0,
            y_exponent: 1.0,
            rgb: [[0.0, 0.0, 0.0, 1.0]; 4],
            mode: 1,
            rotx: 0,
            rotz: 0,
        }
    }
}

struct Superquadrics {
    dist: i32,
    wireframe: bool,
    /// How many frames a morph takes, and half that again waiting at the end.
    maxcount: i32,
    maxwait: i32,
    counter: i32,
    curmat: [[f32; 4]; 4],
    rotx: f64,
    roty: f64,
    rotz: f64,
    spinspeed: f64,

    // The sampled surface, one entry per mesh line. Indexed from one, as
    // upstream's Pascal-descended code does, so the slot at zero is unused.
    cs: Vec<f64>,
    se: Vec<f64>,
    sw: Vec<f64>,
    sn: Vec<f64>,
    ss: Vec<f64>,
    ce: Vec<f64>,
    cw: Vec<f64>,
    cn: Vec<f64>,

    x_exponent: f64,
    y_exponent: f64,
    mode: f64,
    resolution: usize,
    now: State,
    later: State,
    /// Which face is being culled: none, the back, or the front.
    cullmode: i32,
}

/// `myrand`: an integer in `0..range`, taken from a uniform real as upstream
/// does rather than by a remainder.
fn myrand(range: i32) -> i32 {
    (range as f64 * frand(1.0)) as i32
}

fn myrandreal() -> f64 {
    frand(1.0)
}

/// `XtoY`: not the usual power function. A negative base is exponentiated by
/// magnitude and given its sign back, because the surface asks for fractional
/// powers of negative numbers on every quadrant but the first.
fn xtoy(x: f64, y: f64) -> f64 {
    let z = x.abs();
    if z < 1e-20 {
        return 0.0;
    }
    let mut a = (y * z.ln()).exp();
    if a > CLIP_NORMALS {
        a = CLIP_NORMALS;
    }
    if x < 0.0 { -a } else { a }
}

fn sine(x: f64, e: f64) -> f64 {
    xtoy(x.sin(), e)
}

fn cosine(x: f64, e: f64) -> f64 {
    xtoy(x.cos(), e)
}

impl Superquadrics {
    /// `MakeUpStuff`: decide what to morph into next. Which of the exponents,
    /// mode, colours and attitude get new values is itself random, so a change
    /// is usually partial.
    fn make_up_stuff(&mut self, allstuff: bool) {
        let allstuff = allstuff || self.maxcount < 2;
        let mut dostuff = if allstuff { 15 } else { 0 };
        if dostuff == 0 {
            dostuff = myrand(3) + 1;
            if myrand(2) != 0 || (dostuff & 1) != 0 {
                dostuff |= 4;
            }
            if myrand(2) != 0 {
                dostuff |= 8;
            }
        }

        if dostuff & 1 != 0 {
            self.later.x_exponent =
                ((myrandreal() * 250.0 + 0.5).floor() as i64) as f64 / 100.0 + 0.1;
            self.later.y_exponent =
                ((myrandreal() * 250.0 + 0.5).floor() as i64) as f64 / 100.0 + 0.1;
            // Increase the 2.0 .. 2.5 range to 2.0 .. 3.0
            if self.later.x_exponent > 2.0 {
                self.later.x_exponent = self.later.x_exponent * 2.0 - 2.0;
            }
            if self.later.y_exponent > 2.0 {
                self.later.y_exponent = self.later.y_exponent * 2.0 - 2.0;
            }
        }

        if dostuff & 2 != 0 {
            loop {
                self.later.mode = (myrand(3) + 1) as i64;
                // On init, let it stay in mode 1 if it feels like it.
                if allstuff || self.later.mode != self.now.mode {
                    break;
                }
            }
        }

        if dostuff & 4 != 0 {
            let r = (40 + myrand(200)) as f32 / 255.0;
            let g = (40 + myrand(200)) as f32 / 255.0;
            let b = (40 + myrand(200)) as f32 / 255.0;
            let flip = |c: f32| {
                if myrand(4) != 0 && !(0.31..=0.69).contains(&c) {
                    1.0 - c
                } else {
                    c
                }
            };
            let (r2, g2, b2) = (flip(r), flip(g), flip(b));

            let pat = PATS[myrand(4) as usize];
            for (slot, on) in self.later.rgb.iter_mut().zip(pat) {
                *slot = if on {
                    [r, g, b, 1.0]
                } else {
                    [r2, g2, b2, 1.0]
                };
            }
        }

        if dostuff & 8 != 0 {
            self.later.rotx = myrand(360) - 180;
            self.later.rotz = myrand(160) - 80;
        }
    }

    /// `inputs`: sample the surface for the exponents and mode it currently
    /// has. Everything the mesh needs is in these eight tables.
    fn inputs(&mut self) {
        let (mode3, cn3, inverter2) = if self.mode < 1.000001 {
            (1.0, 0.0, 1.0)
        } else if self.mode < 2.000001 {
            (1.0, (self.mode - 1.0) * 1.5, (self.mode - 1.0) * -2.0 + 1.0)
        } else {
            (self.mode - 1.0, (self.mode - 2.0) / 2.0 + 1.5, -1.0)
        };

        let n = self.resolution;
        let denom = (n - 1) as f64;
        for iv in 1..=n {
            // u runs from PI down to -PI, v from PI/2 down to -PI/2.
            let u =
                (1 - iv as i64) as f64 * 2.0 * std::f64::consts::PI / denom + std::f64::consts::PI;
            let v = (1 - iv as i64) as f64 * mode3 * std::f64::consts::PI / denom
                + std::f64::consts::PI * (mode3 / 2.0);

            self.se[iv] = sine(u, self.x_exponent);
            self.ce[iv] = cosine(u, self.x_exponent);
            self.sn[iv] = sine(v, self.y_exponent);
            self.cn[iv] = cosine(v, self.y_exponent) * inverter2 + cn3;

            // Normal vector computations only.
            // Upstream offsets these by a step when flat shading, which is
            // a mode the standalone saver never selects.
            self.sw[iv] = sine(u, 2.0 - self.x_exponent);
            self.cw[iv] = cosine(u, 2.0 - self.x_exponent);
            self.ss[iv] = sine(v, 2.0 - self.y_exponent) * inverter2;
            self.cs[iv] = cosine(v, 2.0 - self.y_exponent);
        }

        // Now fix up the endpoints.
        self.se[n] = self.se[1];
        self.ce[n] = self.ce[1];
        if self.mode > 2.999999 {
            self.sn[n] = self.sn[1];
            self.cn[n] = self.cn[1];
        }
    }

    /// `DoneScale`: walk the mesh and emit it, one quad at a time, keeping the
    /// previous row so each quad can close against it.
    fn done_scale(&self, g: &mut Gl) {
        let n = self.resolution;
        let mut prev_xx = vec![0.0f64; n + 1];
        let mut prev_yy = vec![0.0f64; n + 1];
        let mut prev_zz = vec![0.0f64; n + 1];
        let mut prev_xn = vec![0.0f64; n + 1];
        let mut prev_yn = vec![0.0f64; n + 1];
        let mut prev_zn = vec![0.0f64; n + 1];

        let (mut xp, mut yp, mut zp) = (0.0f64, 0.0f64, 0.0f64);
        let (mut xnp, mut ynp, mut znp) = (0.0f64, 0.0f64, 0.0f64);
        let mut toggle = 0usize;

        for ih in 1..=n {
            toggle ^= 2;
            for iv in 1..=n {
                toggle ^= 1;
                let c = self.curmat[toggle];
                if self.wireframe {
                    g.glx.color3f(c[0], c[1], c[2]);
                } else {
                    g.glx.material_ambient_diffuse(c);
                }

                let xx = self.cn[iv] * self.ce[ih];
                let zz = self.cn[iv] * self.se[ih];
                let yy = self.sn[iv];

                if self.wireframe {
                    if ih > 1 || iv > 1 {
                        g.glx.begin(Shape::Lines);
                        if ih > 1 {
                            g.glx.vertex3f(xx as f32, yy as f32, zz as f32);
                            g.glx.vertex3f(
                                prev_xx[iv] as f32,
                                prev_yy[iv] as f32,
                                prev_zz[iv] as f32,
                            );
                        }
                        if iv > 1 {
                            g.glx.vertex3f(xx as f32, yy as f32, zz as f32);
                            g.glx.vertex3f(
                                prev_xx[iv - 1] as f32,
                                prev_yy[iv - 1] as f32,
                                prev_zz[iv - 1] as f32,
                            );
                        }
                        g.glx.end();
                    }
                } else {
                    // A spike whose exponent has run away takes its normal
                    // with it, and upstream points that one straight out.
                    let (xn, yn, zn) = if self.cs[iv] > 1e+10 || self.cs[iv] < -1e+10 {
                        (self.cs[iv], self.ss[iv], self.cs[iv])
                    } else {
                        (
                            self.cs[iv] * self.cw[ih],
                            self.ss[iv],
                            self.cs[iv] * self.sw[ih],
                        )
                    };

                    if ih > 1 && iv > 1 {
                        g.glx.normal3f(xn as f32, yn as f32, zn as f32);
                        g.glx.begin(Shape::Polygon);
                        g.glx.vertex3f(xx as f32, yy as f32, zz as f32);
                        g.glx
                            .normal3f(prev_xn[iv] as f32, prev_yn[iv] as f32, prev_zn[iv] as f32);
                        g.glx
                            .vertex3f(prev_xx[iv] as f32, prev_yy[iv] as f32, prev_zz[iv] as f32);
                        g.glx.normal3f(xnp as f32, ynp as f32, znp as f32);
                        g.glx.vertex3f(xp as f32, yp as f32, zp as f32);
                        g.glx.normal3f(
                            prev_xn[iv - 1] as f32,
                            prev_yn[iv - 1] as f32,
                            prev_zn[iv - 1] as f32,
                        );
                        g.glx.vertex3f(
                            prev_xx[iv - 1] as f32,
                            prev_yy[iv - 1] as f32,
                            prev_zz[iv - 1] as f32,
                        );
                        g.glx.end();
                    }

                    xnp = prev_xn[iv];
                    ynp = prev_yn[iv];
                    znp = prev_zn[iv];
                    prev_xn[iv] = xn;
                    prev_yn[iv] = yn;
                    prev_zn[iv] = zn;
                }

                xp = prev_xx[iv];
                yp = prev_yy[iv];
                zp = prev_zz[iv];
                prev_xx[iv] = xx;
                prev_yy[iv] = yy;
                prev_zz[iv] = zz;
            }
        }
    }

    /// `SetCull`. A closed quadric shows only its outside and a torus only its
    /// inside; in between, when the surface is open, neither can be thrown
    /// away.
    ///
    /// There is no separate `glCullFace` here, so culling the front is culling
    /// the back of the opposite winding, which comes to the same thing.
    fn set_cull(&mut self, g: &mut Gl) {
        self.cullmode = if self.mode < 1.0001 {
            1
        } else if self.mode > 2.9999 {
            2
        } else {
            0
        };

        g.glx.cull_face(self.cullmode != 0);
        // Upstream sets GL_CW once and then names the face to drop.
        g.glx.front_face_cw(self.cullmode != 2);
    }

    /// `SetCurrentShape`: the morph has arrived, so what was coming is now
    /// what is.
    fn set_current_shape(&mut self) {
        self.x_exponent = self.later.x_exponent;
        self.y_exponent = self.later.y_exponent;
        self.now = self.later;
        self.curmat = self.later.rgb;
        self.mode = self.later.mode as f64;
        self.rotx = self.later.rotx as f64;
        self.rotz = self.later.rotz as f64;
        self.counter = -self.maxwait;
        self.inputs();
    }

    /// `NextSuperquadric`: spin a little, and either wait, morph a step, or
    /// pick the next shape.
    fn next(&mut self) {
        self.roty -= self.spinspeed;
        while self.roty >= 360.0 {
            self.roty -= 360.0;
        }
        while self.roty < 0.0 {
            self.roty += 360.0;
        }

        if self.counter > 0 {
            self.counter -= 1;
            if self.counter == 0 {
                self.set_current_shape();
                if self.counter == 0 {
                    // Happens when maxwait is zero.
                    self.make_up_stuff(false);
                    self.counter = self.maxcount;
                }
            } else {
                let fnow = self.counter as f64 / self.maxcount as f64;
                let flater = (self.maxcount - self.counter) as f64 / self.maxcount as f64;
                self.x_exponent = self.now.x_exponent * fnow + self.later.x_exponent * flater;
                self.y_exponent = self.now.y_exponent * fnow + self.later.y_exponent * flater;
                for t in 0..4 {
                    for k in 0..3 {
                        self.curmat[t][k] =
                            self.now.rgb[t][k] * fnow as f32 + self.later.rgb[t][k] * flater as f32;
                    }
                }
                self.mode = self.now.mode as f64 * fnow + self.later.mode as f64 * flater;
                self.rotx = self.now.rotx as f64 * fnow + self.later.rotx as f64 * flater;
                self.rotz = self.now.rotz as f64 * fnow + self.later.rotz as f64 * flater;
                self.inputs();
            }
        } else {
            self.counter += 1;
            if self.counter >= 0 {
                self.make_up_stuff(false);
                self.counter = self.maxcount;
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wireframe = g.res.bool("wireframe");
    let resolution = (g.res.int("count") as usize).clamp(MIN_RES, MAX_RES);
    let maxcount = g.res.int("cycles").max(1);
    let n = MAX_RES + 1;

    let mut this = Superquadrics {
        dist: 16 << 3,
        wireframe,
        maxcount,
        maxwait: maxcount >> 1,
        counter: 0,
        curmat: [[0.0, 0.0, 0.0, 1.0]; 4],
        rotx: 35.0,
        roty: 0.0,
        rotz: 0.0,
        spinspeed: g.res.float("spinspeed"),
        cs: vec![0.0; n],
        se: vec![0.0; n],
        sw: vec![0.0; n],
        sn: vec![0.0; n],
        ss: vec![0.0; n],
        ce: vec![0.0; n],
        cw: vec![0.0; n],
        cn: vec![0.0; n],
        x_exponent: 1.0,
        y_exponent: 1.0,
        mode: 1.0,
        resolution,
        now: State::default(),
        later: State::default(),
        cullmode: 0,
    };

    this.make_up_stuff(true);
    this.set_current_shape();
    this.make_up_stuff(true);
    this.counter = maxcount;

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Superquadrics {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let (mut height, mut y) = (height, 0);
        if width > height * 5 {
            // Tiny window: show the middle.
            height = width;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx
            .perspective(15.0, width as f32 / height as f32, 0.1, 200.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.next();

        g.glx.clear();
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.depth_test(true);
            g.glx.light_enable(0, true);
            g.glx.light_ambient(0, [0.4, 0.4, 0.4, 1.0]);
            g.glx.light_position(0, 10.0, 1.0, 1.0, 10.0);
            g.glx.material_specular([0.8, 0.8, 0.8, 1.0]);
            g.glx.material_shininess(50.0);
        } else {
            g.glx.lighting(false);
            g.glx.depth_test(false);
        }

        g.glx.push_matrix();
        // Viewing transform. The distance backs off as the mode opens the
        // surface out, because a torus is wider than a sphere.
        g.glx.translate(
            0.0,
            0.0,
            -(self.dist as f32 / 16.0) - (self.mode as f32 * 3.0 - 1.0),
        );
        g.glx.rotate(self.rotx as f32, 1.0, 0.0, 0.0); // pitch
        g.glx.rotate(self.rotz as f32, 0.0, 0.0, 1.0); // bank
        g.glx.rotate(self.roty as f32, 0.0, 1.0, 0.0); // spin

        self.set_cull(g);

        g.glx.scale(0.7, 0.7, 0.7);
        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        self.done_scale(g);
        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        40000",
    "*count:        25",
    "*cycles:       40",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spinspeed:    5.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "40000").inverted(),
    Opt::slider("spinspeed", "Spin speed", 0.1, 15.0, 0.1, 1, "5.0"),
    // The XML offers 0 to 100 for both of these; the C clamps the first to
    // 5..50 and the second to at least 1, so the panel offers what has an
    // effect rather than a slider whose top half does nothing.
    Opt::slider("count", "Density", 5.0, 50.0, 1.0, 0, "25"),
    Opt::slider("cycles", "Duration", 1.0, 100.0, 1.0, 0, "40"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "superquadrics",
    label: "Superquadrics",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ed Mackey",
        year: "1987",
        video: Some("https://www.youtube.com/watch?v=Mjlc7iPA1N4"),
        blurb: "Morphing 3D mathematical shapes, invented by Alan Barr in 1981.",
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

    #[test]
    fn a_negative_base_keeps_its_sign_through_a_fractional_power() {
        // The real function is undefined here, and the shape depends on the
        // substitute: the magnitude is raised and the sign put back.
        assert!((xtoy(-0.5, 0.5) + 0.5f64.sqrt()).abs() < 1e-12);
        assert!((xtoy(0.5, 0.5) - 0.5f64.sqrt()).abs() < 1e-12);
        // And a spike is clipped rather than allowed to run away.
        assert_eq!(xtoy(1e6, 4.0), CLIP_NORMALS);
        assert_eq!(xtoy(-1e6, 4.0), -CLIP_NORMALS);
        assert_eq!(xtoy(0.0, 0.5), 0.0);
    }

    #[test]
    fn the_mesh_closes_on_itself() {
        // The last mesh line has to land on the first or the surface has a
        // seam down it.
        let mut r = start(StartArgs::new(640, 480, "count=12", 20260811));
        r.step();
        let f = r.frame();
        assert!(!f.vertices.is_empty());
    }

    #[test]
    fn every_quad_is_a_quad() {
        let mut r = start(StartArgs::new(640, 480, "count=10", 20260811));
        r.step();
        let f = r.frame();
        // A four-cornered polygon stays a fan of four rather than being cut
        // into triangles, so a batch is a whole number of quads.
        for b in &f.batches {
            assert_eq!(b.primitive, crate::runtime::gl::Primitive::TriangleFan);
            assert_eq!(b.count, 4, "a partial quad was drawn");
        }
    }

    #[test]
    fn the_surface_is_two_colours_in_one_of_four_patterns() {
        // Plain, striped either way, or a checkerboard. Whichever it is, the
        // mesh only ever carries two distinct colours. A duration of one is
        // what makes that visible: with any longer one the frame is caught
        // part way through a morph and the four pattern slots are four
        // different blends of the old pair and the new.
        for seed in [1u32, 7, 20260811, 99] {
            let mut r = start(StartArgs::new(640, 480, "count=10&cycles=1", seed));
            r.step();
            let f = r.frame();
            let mut seen: Vec<[u32; 3]> = f
                .batches
                .iter()
                .map(|b| {
                    let m = b.material.ambient_diffuse;
                    [m[0].to_bits(), m[1].to_bits(), m[2].to_bits()]
                })
                .collect();
            seen.sort_unstable();
            seen.dedup();
            assert!(seen.len() <= 2, "seed {seed} used {} colours", seen.len());
        }
    }

    #[test]
    fn a_closed_shape_culls_and_an_open_one_cannot() {
        // Mode 1 is a closed quadric and shows only its outside; mode 3 is a
        // torus and shows only its inside; between them the surface is open at
        // both ends and neither side can be thrown away.
        let mut r = start(StartArgs::new(640, 480, "count=8&cycles=100", 20260811));
        let mut culled_back = 0;
        let mut culled_front = 0;
        let mut open = 0;
        for _ in 0..800 {
            r.step();
            let f = r.frame();
            let b = &f.batches[0];
            if !b.cull_face {
                open += 1;
            } else if b.front_face_cw {
                culled_back += 1;
            } else {
                culled_front += 1;
            }
        }
        assert!(open > 0, "the surface was never open");
        assert!(
            culled_back + culled_front > 0,
            "the surface never closed up"
        );
    }

    #[test]
    fn a_new_shape_is_eased_into_rather_than_switched_to() {
        // The exponents interpolate over `cycles` frames, so consecutive
        // frames differ by a little and never by a lot.
        let mut r = start(StartArgs::new(640, 480, "count=8&cycles=60", 20260811));
        r.step();
        let mut last: Vec<[f32; 3]> = r.frame().vertices.iter().map(|v| v.pos).collect();
        let mut moved = 0;
        for _ in 0..120 {
            r.step();
            let now: Vec<[f32; 3]> = r.frame().vertices.iter().map(|v| v.pos).collect();
            if now.len() == last.len() {
                let d: f32 = now
                    .iter()
                    .zip(&last)
                    .map(|(a, b)| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs())
                    .sum::<f32>()
                    / now.len() as f32;
                assert!(d < 0.5, "the shape jumped by {d}");
                if d > 1e-6 {
                    moved += 1;
                }
            }
            last = now;
        }
        assert!(moved > 60, "it only moved on {moved} of 120 frames");
    }
}
