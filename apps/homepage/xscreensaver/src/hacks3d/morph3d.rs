//! Port of `hacks/glx/morph3d.c`.
//!
//! ```text
//! morph3d --- Shows 3D morphing objects
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
//! The original code for this mode was written by Marcelo Fernandes Vianna
//! (me...) and was inspired on a WindowsNT(R)'s screen saver (Flower Box).
//! It was written from scratch and it was not based on any other source code.
//!
//! Marcelo F. Vianna (Feb-13-1997)
//! ```
//!
//! One of the five Platonic solids, breathing.
//!
//! The trick is that no face is ever a flat polygon. A face is a tessellated
//! sheet of triangles, and every vertex of it is pushed along its own direction
//! from the middle of the face by `1 - amp * r^2 / vr^2`, where `r` is how far
//! out the vertex started and `vr` is the distance to a corner. That factor is
//! one at the centre and falls off as the square of the radius, so the middle
//! of the face stays put and the corners travel. Drive `amp` with a sine and
//! the solid inflates into a sphere-ish blob, flattens back, and then keeps
//! going: past the point where the factor turns negative the corners have been
//! pushed through the centre and out the other side, and the solid is a set of
//! interpenetrating spikes.
//!
//! That sign change is worth watching for, because the program does. Whenever
//! the last factor it computed came out negative it knows the faces are
//! inside out and turns culling off for the frame, which is the only reason
//! the spiky phase is not full of holes.
//!
//! The normals are not derived; they are measured. For each vertex the code
//! also evaluates the surface a thousandth of a unit along in each of two
//! directions and takes the cross product of the two differences, which is a
//! finite-difference normal and costs no algebra at all.
//!
//! The object rolls on all three axes at slightly different rates and wanders
//! on a Lissajous figure, so it never quite repeats.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, random, screenhack_event_helper,
};

const SCALE4WINDOW: f32 = 0.3;

/// Increasing these produces better image quality; the price is speed.
const TETRA_DIVISIONS: i32 = 23;
const CUBE_DIVISIONS: i32 = 20;
const OCTA_DIVISIONS: i32 = 21;
const DODECA_DIVISIONS: i32 = 10;
const ICO_DIVISIONS: i32 = 15;

const TETRA_ANGLE: f32 = 109.471_22;
const CUBE_ANGLE: f32 = 90.0;
const OCTA_ANGLE: f32 = 109.471_22;
const DODECA_ANGLE: f32 = 63.434_95;
const ICO_ANGLE: f32 = 41.810_314;

const SQRT2: f32 = std::f32::consts::SQRT_2;
const SQRT3: f32 = 1.732_050_8;
const SQRT5: f32 = 2.236_068;
const SQRT6: f32 = 2.449_489_8;
const SQRT15: f32 = 3.872_983_4;
const COSSEC36_2: f32 = 0.850_650_8;
const COS72: f32 = 0.309_017;
const SIN72: f32 = 0.951_056_5;
const COS36: f32 = 0.809_017;
const SIN36: f32 = 0.587_785_25;

const MATERIAL_RED: [f32; 4] = [0.7, 0.0, 0.0, 1.0];
const MATERIAL_GREEN: [f32; 4] = [0.1, 0.5, 0.2, 1.0];
const MATERIAL_BLUE: [f32; 4] = [0.0, 0.0, 0.7, 1.0];
const MATERIAL_CYAN: [f32; 4] = [0.2, 0.5, 0.7, 1.0];
const MATERIAL_YELLOW: [f32; 4] = [0.7, 0.7, 0.0, 1.0];
const MATERIAL_MAGENTA: [f32; 4] = [0.6, 0.2, 0.5, 1.0];
const MATERIAL_WHITE: [f32; 4] = [0.7, 0.7, 0.7, 1.0];
const MATERIAL_GRAY: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

/// Which solid this run drew.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Object {
    Tetra,
    Cube,
    Octa,
    Dodeca,
    Icosa,
}

struct Morph3d {
    step: f32,
    /// The amplitude the faces are pushed out by this frame.
    seno: f32,
    object: Object,
    edgedivisions: i32,
    /// Whether the last face drawn had turned itself inside out, which is what
    /// decides whether the next frame culls.
    visible_spikes: bool,
    magnitude: f32,
    colors: Vec<[f32; 4]>,
    width: i32,
    height: i32,
}

fn sqr(x: f32) -> f32 {
    x * x
}

/// The cross product, which upstream writes out longhand in a macro.
fn vect_mul(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl Morph3d {
    /// One vertex of a face: where it lands, and the normal measured by
    /// evaluating the same surface a thousandth of a unit along in two
    /// directions.
    fn emit(&self, g: &mut Gl, xf: f32, yf: f32, xa: f32, yb: f32, zf: f32, amp_vr2: f32) -> f32 {
        let factor = 1.0 - ((sqr(xf) + sqr(yf)) * amp_vr2);
        let factor1 = 1.0 - ((sqr(xa) + sqr(yf)) * amp_vr2);
        let factor2 = 1.0 - ((sqr(xf) + sqr(yb)) * amp_vr2);
        let vert = [factor * xf, factor * yf, factor * zf];
        let nei_a = [
            factor1 * xa - vert[0],
            factor1 * yf - vert[1],
            factor1 * zf - vert[2],
        ];
        let nei_b = [
            factor2 * xf - vert[0],
            factor2 * yb - vert[1],
            factor2 * zf - vert[2],
        ];
        let n = vect_mul(nei_a, nei_b);
        g.glx.normal3f(n[0], n[1], n[2]);
        g.glx.vertex3f(vert[0], vert[1], vert[2]);
        factor
    }

    /// `TRIANGLE`: an equilateral face, drawn as concentric rings of strips.
    fn triangle(&mut self, g: &mut Gl, edge: f32, divisions: i32, z: f32) {
        let amp = self.seno;
        let vr = edge * SQRT3 / 3.0;
        let amp_vr2 = amp / sqr(vr);
        let zf = edge * z;
        let ax = edge * (0.5 / divisions as f32);
        let ay = edge * (-SQRT3 / (2.0 * divisions as f32));

        let mut factor = 0.0;
        let mut yf = vr + ay;
        let mut yb = yf + 0.001;

        for ri in 1..=divisions {
            g.glx.begin(Shape::TriangleStrip);
            let mut xf = ri as f32 * ax;
            let mut xa = xf + 0.001;

            for _ in 0..ri {
                // Only the closing vertex of the last ring decides whether the
                // face has turned itself inside out, so the rest go unread.
                self.emit(g, xf, yf, xa, yb, zf, amp_vr2);
                xf -= ax;
                yf -= ay;
                xa -= ax;
                yb -= ay;

                self.emit(g, xf, yf, xa, yb, zf, amp_vr2);
                xf -= ax;
                yf += ay;
                xa -= ax;
                yb += ay;
            }
            factor = self.emit(g, xf, yf, xa, yb, zf, amp_vr2);
            yf += ay;
            yb += ay;
            g.glx.end();
        }
        self.visible_spikes = factor < 0.0;
    }

    /// `SQUARE`: a square face, drawn as rows of quad strips.
    fn square(&mut self, g: &mut Gl, edge: f32, divisions: i32, z: f32) {
        let amp = self.seno;
        let zf = edge * z;
        let amp_vr2 = amp / sqr(edge * SQRT2 / 2.0);
        let mut factor = 0.0;

        for yi in 0..divisions {
            let yf = -(edge / 2.0) + (yi as f32) / divisions as f32 * edge;
            let y = yf + 1.0 / divisions as f32 * edge;
            g.glx.begin(Shape::QuadStrip);
            for xi in 0..=divisions {
                let xf = -(edge / 2.0) + (xi as f32) / divisions as f32 * edge;
                let xa = xf + 0.001;
                self.emit(g, xf, y, xa, y + 0.001, zf, amp_vr2);
                factor = self.emit(g, xf, yf, xa, yf + 0.001, zf, amp_vr2);
            }
            g.glx.end();
        }
        self.visible_spikes = factor < 0.0;
    }

    /// `PENTAGON`: five triangular sectors, each drawn as concentric strips.
    fn pentagon(&mut self, g: &mut Gl, edge: f32, divisions: i32, z: f32) {
        let amp = self.seno;
        let zf = edge * z;
        let amp_vr2 = amp / sqr(edge * COSSEC36_2);
        let mut factor = 0.0;

        let mut x = [0.0f32; 6];
        let mut y = [0.0f32; 6];
        for fi in 0..6 {
            let th = fi as f32 * 2.0 * std::f32::consts::PI / 5.0 + std::f32::consts::PI / 10.0;
            x[fi] = -th.cos() / divisions as f32 * COSSEC36_2 * edge;
            y[fi] = th.sin() / divisions as f32 * COSSEC36_2 * edge;
        }

        for ri in 1..=divisions {
            for fi in 0..5 {
                g.glx.begin(Shape::TriangleStrip);
                for ti in 0..ri {
                    let mut xf = (ri - ti) as f32 * x[fi] + ti as f32 * x[fi + 1];
                    let mut yf = (ri - ti) as f32 * y[fi] + ti as f32 * y[fi + 1];
                    self.emit(g, xf, yf, xf + 0.001, yf + 0.001, zf, amp_vr2);

                    xf -= x[fi];
                    yf -= y[fi];
                    self.emit(g, xf, yf, xf + 0.001, yf + 0.001, zf, amp_vr2);
                }
                let xf = ri as f32 * x[fi + 1];
                let yf = ri as f32 * y[fi + 1];
                factor = self.emit(g, xf, yf, xf + 0.001, yf + 0.001, zf, amp_vr2);
                g.glx.end();
            }
        }
        self.visible_spikes = factor < 0.0;
    }

    fn face_colour(&self, g: &mut Gl, i: usize) {
        // GL_DIFFUSE only: the ambient stays at GL's grey, which is what keeps
        // a scene ambient of a half from washing every face towards its own
        // colour.
        g.glx.material_diffuse(self.colors[i]);
    }

    fn draw_tetra(&mut self, g: &mut Gl) {
        let (d, z) = (self.edgedivisions, 0.5 / SQRT6);
        self.face_colour(g, 0);
        self.triangle(g, 2.0, d, z);

        g.glx.push_matrix();
        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        g.glx.rotate(-TETRA_ANGLE, 1.0, 0.0, 0.0);
        self.face_colour(g, 1);
        self.triangle(g, 2.0, d, z);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        g.glx.rotate(-180.0 + TETRA_ANGLE, 0.5, SQRT3 / 2.0, 0.0);
        self.face_colour(g, 2);
        self.triangle(g, 2.0, d, z);
        g.glx.pop_matrix();

        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        g.glx.rotate(-180.0 + TETRA_ANGLE, 0.5, -SQRT3 / 2.0, 0.0);
        self.face_colour(g, 3);
        self.triangle(g, 2.0, d, z);
    }

    fn draw_cube(&mut self, g: &mut Gl) {
        // Six faces reached by turning a quarter at a time, with no pushing:
        // three quarters about x brings you back, then two about y for the
        // remaining pair.
        let d = self.edgedivisions;
        self.face_colour(g, 0);
        self.square(g, 2.0, d, 0.5);

        for i in 1..=3 {
            g.glx.rotate(CUBE_ANGLE, 1.0, 0.0, 0.0);
            self.face_colour(g, i);
            self.square(g, 2.0, d, 0.5);
        }
        g.glx.rotate(CUBE_ANGLE, 0.0, 1.0, 0.0);
        self.face_colour(g, 4);
        self.square(g, 2.0, d, 0.5);

        g.glx.rotate(2.0 * CUBE_ANGLE, 0.0, 1.0, 0.0);
        self.face_colour(g, 5);
        self.square(g, 2.0, d, 0.5);
    }

    fn draw_octa(&mut self, g: &mut Gl) {
        let (d, z) = (self.edgedivisions, 1.0 / SQRT6);
        // Two caps of four: a face on the axis and three leaning off it. The
        // second cap is the first turned over. Upstream leaves the very last
        // turn un-pushed, which makes no difference with nothing after it.
        for half in 0..2 {
            let base = half * 4;
            if half == 1 {
                g.glx.rotate(180.0, 1.0, 0.0, 0.0);
            }
            self.face_colour(g, base);
            self.triangle(g, 2.0, d, z);

            g.glx.push_matrix();
            g.glx.rotate(180.0, 0.0, 0.0, 1.0);
            g.glx.rotate(-180.0 + OCTA_ANGLE, 1.0, 0.0, 0.0);
            self.face_colour(g, base + 1);
            self.triangle(g, 2.0, d, z);
            g.glx.pop_matrix();

            g.glx.push_matrix();
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            g.glx.rotate(-OCTA_ANGLE, 0.5, SQRT3 / 2.0, 0.0);
            self.face_colour(g, base + 2);
            self.triangle(g, 2.0, d, z);
            g.glx.pop_matrix();

            g.glx.push_matrix();
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            g.glx.rotate(-OCTA_ANGLE, 0.5, -SQRT3 / 2.0, 0.0);
            self.face_colour(g, base + 3);
            self.triangle(g, 2.0, d, z);
            g.glx.pop_matrix();
        }
    }

    fn draw_dodeca(&mut self, g: &mut Gl) {
        let tau = (SQRT5 + 1.0) / 2.0;
        let d = self.edgedivisions;
        let z = sqr(tau) * ((tau + 2.0) / 5.0).sqrt() / 2.0;

        // The twelve faces are two caps of six: a face on the axis and the
        // five leaning away from it. The second cap is the first turned over.
        let ring = [
            (-DODECA_ANGLE, 1.0, 0.0),
            (-DODECA_ANGLE, COS72, SIN72),
            (-DODECA_ANGLE, COS72, -SIN72),
            (DODECA_ANGLE, COS36, -SIN36),
        ];

        self.face_colour(g, 0);
        self.pentagon(g, 1.0, d, z);

        g.glx.push_matrix();
        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        for (k, (angle, ax, ay)) in ring.into_iter().enumerate() {
            g.glx.push_matrix();
            g.glx.rotate(angle, ax, ay, 0.0);
            self.face_colour(g, 1 + k);
            self.pentagon(g, 1.0, d, z);
            g.glx.pop_matrix();
        }
        g.glx.rotate(DODECA_ANGLE, COS36, SIN36, 0.0);
        self.face_colour(g, 5);
        self.pentagon(g, 1.0, d, z);
        g.glx.pop_matrix();

        g.glx.rotate(180.0, 1.0, 0.0, 0.0);
        self.face_colour(g, 6);
        self.pentagon(g, 1.0, d, z);

        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        for (k, (angle, ax, ay)) in ring.into_iter().enumerate() {
            g.glx.push_matrix();
            g.glx.rotate(angle, ax, ay, 0.0);
            self.face_colour(g, 7 + k);
            self.pentagon(g, 1.0, d, z);
            g.glx.pop_matrix();
        }
        g.glx.rotate(DODECA_ANGLE, COS36, SIN36, 0.0);
        self.face_colour(g, 11);
        self.pentagon(g, 1.0, d, z);
    }

    fn draw_icosa(&mut self, g: &mut Gl) {
        let d = self.edgedivisions;
        let z = (3.0 * SQRT3 + SQRT15) / 12.0;

        // Upstream writes this one out as twenty calls with a hand-built stack
        // of pushes and pops, and only three distinct turns between them. Named
        // and nested, the shape is two caps of ten and three branches a cap;
        // the faces come out in the same places in the same order.
        let face = |g: &mut Gl, this: &mut Self, i: usize| {
            this.face_colour(g, i);
            this.triangle(g, 1.5, d, z);
        };
        let turn_a = |g: &mut Gl| {
            g.glx.rotate(180.0, 0.0, 0.0, 1.0);
            g.glx.rotate(-ICO_ANGLE, 1.0, 0.0, 0.0);
        };
        let turn_b = |g: &mut Gl| {
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            g.glx.rotate(-180.0 + ICO_ANGLE, 0.5, SQRT3 / 2.0, 0.0);
        };
        let turn_c = |g: &mut Gl| {
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            g.glx.rotate(-180.0 + ICO_ANGLE, 0.5, -SQRT3 / 2.0, 0.0);
        };

        face(g, self, 0);

        // The twenty faces are two caps of ten, the second the first turned
        // over, and each cap is a face plus three branches off it.
        for half in 0..2 {
            let base = half * 10;
            if half == 0 {
                g.glx.push_matrix();
            } else {
                g.glx.rotate(180.0, 1.0, 0.0, 0.0);
                face(g, self, base);
            }

            g.glx.push_matrix();
            turn_a(g);
            face(g, self, base + 1);
            g.glx.push_matrix();
            turn_b(g);
            face(g, self, base + 2);
            g.glx.pop_matrix();
            turn_c(g);
            face(g, self, base + 3);
            g.glx.pop_matrix();

            g.glx.push_matrix();
            turn_b(g);
            face(g, self, base + 4);
            g.glx.push_matrix();
            turn_b(g);
            face(g, self, base + 5);
            g.glx.pop_matrix();
            turn_a(g);
            face(g, self, base + 6);
            g.glx.pop_matrix();

            turn_c(g);
            face(g, self, base + 7);
            g.glx.push_matrix();
            turn_c(g);
            face(g, self, base + 8);
            g.glx.pop_matrix();
            turn_a(g);
            face(g, self, base + 9);
            if half == 0 {
                g.glx.pop_matrix();
            }
        }
    }
}

/// `pinit`: which solid, how finely tessellated, and the colour of each face.
fn palette(object: Object) -> (Vec<[f32; 4]>, i32, f32) {
    match object {
        Object::Cube => (
            vec![
                MATERIAL_RED,
                MATERIAL_GREEN,
                MATERIAL_CYAN,
                MATERIAL_MAGENTA,
                MATERIAL_YELLOW,
                MATERIAL_BLUE,
            ],
            CUBE_DIVISIONS,
            2.0,
        ),
        Object::Octa => (
            vec![
                MATERIAL_RED,
                MATERIAL_GREEN,
                MATERIAL_BLUE,
                MATERIAL_WHITE,
                MATERIAL_CYAN,
                MATERIAL_MAGENTA,
                MATERIAL_GRAY,
                MATERIAL_YELLOW,
            ],
            OCTA_DIVISIONS,
            2.5,
        ),
        Object::Dodeca => (
            vec![
                MATERIAL_RED,
                MATERIAL_GREEN,
                MATERIAL_CYAN,
                MATERIAL_BLUE,
                MATERIAL_MAGENTA,
                MATERIAL_YELLOW,
                MATERIAL_GREEN,
                MATERIAL_CYAN,
                MATERIAL_RED,
                MATERIAL_MAGENTA,
                MATERIAL_BLUE,
                MATERIAL_YELLOW,
            ],
            DODECA_DIVISIONS,
            2.0,
        ),
        Object::Icosa => (
            vec![
                MATERIAL_RED,
                MATERIAL_GREEN,
                MATERIAL_BLUE,
                MATERIAL_CYAN,
                MATERIAL_YELLOW,
                MATERIAL_MAGENTA,
                MATERIAL_RED,
                MATERIAL_GREEN,
                MATERIAL_BLUE,
                MATERIAL_WHITE,
                MATERIAL_CYAN,
                MATERIAL_YELLOW,
                MATERIAL_MAGENTA,
                MATERIAL_RED,
                MATERIAL_GREEN,
                MATERIAL_BLUE,
                MATERIAL_CYAN,
                MATERIAL_YELLOW,
                MATERIAL_MAGENTA,
                MATERIAL_GRAY,
            ],
            ICO_DIVISIONS,
            2.5,
        ),
        Object::Tetra => (
            vec![MATERIAL_RED, MATERIAL_GREEN, MATERIAL_BLUE, MATERIAL_WHITE],
            TETRA_DIVISIONS,
            2.5,
        ),
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let count = g.res.int("count");
    let object = match count {
        2 => Object::Cube,
        3 => Object::Octa,
        4 => Object::Dodeca,
        5 => Object::Icosa,
        1 => Object::Tetra,
        _ => match random() % 5 {
            0 => Object::Tetra,
            1 => Object::Cube,
            2 => Object::Octa,
            3 => Object::Dodeca,
            _ => Object::Icosa,
        },
    };
    let (colors, edgedivisions, magnitude) = palette(object);

    let mut this = Morph3d {
        step: (random() % 90) as f32,
        seno: 0.0,
        object,
        edgedivisions,
        visible_spikes: true,
        magnitude,
        colors,
        width: 1,
        height: 1,
    };

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Morph3d {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let (mut height, mut y) = (height, 0);
        if width > height * 5 {
            // Tiny window: show the middle.
            height = width;
            y = -height / 2;
        }
        self.width = width;
        self.height = height;
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -1.0, 1.0, 5.0, 15.0);
        g.glx.matrix_mode_modelview();
    }

    fn event(&mut self, _g: &mut Gl, event: &XEvent) -> bool {
        // Upstream's xlockmore build cycles the solid on `change_morph3d`,
        // which the standalone saver never calls. Poking it here is the
        // nearest thing a browser has to that.
        if screenhack_event_helper(event) {
            self.object = match self.object {
                Object::Tetra => Object::Cube,
                Object::Cube => Object::Octa,
                Object::Octa => Object::Dodeca,
                Object::Dodeca => Object::Icosa,
                Object::Icosa => Object::Tetra,
            };
            let (colors, edgedivisions, magnitude) = palette(self.object);
            self.colors = colors;
            self.edgedivisions = edgedivisions;
            self.magnitude = magnitude;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.lighting(true);

        for i in 0..2 {
            g.glx.light_enable(i, true);
            g.glx.light_ambient(i, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(i, [1.0, 1.0, 1.0, 1.0]);
        }
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_position(1, -1.0, -1.0, 1.0, 0.0);
        g.glx.light_model_ambient([0.5, 0.5, 0.5, 1.0]);
        g.glx.material_specular([0.7, 0.7, 0.7, 1.0]);
        g.glx.material_shininess(60.0);

        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -10.0);

        let (w, h) = (self.width as f32, self.height as f32);
        g.glx
            .scale(SCALE4WINDOW * h / w, SCALE4WINDOW, SCALE4WINDOW);
        g.glx.translate(
            2.5 * w / h * (self.step * 1.11).sin(),
            2.5 * (self.step * 1.25 * 1.11).cos(),
            0.0,
        );

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        g.glx.rotate(self.step * 100.0, 1.0, 0.0, 0.0);
        g.glx.rotate(self.step * 95.0, 0.0, 1.0, 0.0);
        g.glx.rotate(self.step * 90.0, 0.0, 0.0, 1.0);

        self.seno = (self.step.sin() + 1.0 / 3.0) * (4.0 / 5.0) * self.magnitude;

        // Once the faces have turned themselves inside out, culling would eat
        // the spikes, so it goes off until they come back.
        g.glx.cull_face(!self.visible_spikes);

        match self.object {
            Object::Tetra => self.draw_tetra(g),
            Object::Cube => self.draw_cube(g),
            Object::Octa => self.draw_octa(g),
            Object::Dodeca => self.draw_dodeca(g),
            Object::Icosa => self.draw_icosa(g),
        }

        g.glx.pop_matrix();
        self.step += 0.05;

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        40000",
    "*count:        0",
    "*showFPS:      False",
    "*suppressRotationAnimation: True",
];

const OBJECTS: &[SelectItem] = &[
    SelectItem {
        value: "0",
        label: "Random object",
    },
    SelectItem {
        value: "1",
        label: "Tetrahedron",
    },
    SelectItem {
        value: "2",
        label: "Cube",
    },
    SelectItem {
        value: "3",
        label: "Octahedron",
    },
    SelectItem {
        value: "4",
        label: "Dodecahedron",
    },
    SelectItem {
        value: "5",
        label: "Icosahedron",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "40000").inverted(),
    Opt::select("count", "Object", OBJECTS, "0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "morph3d",
    label: "Morph3D",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Marcelo Vianna",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=lNtDppjOli4"),
        blurb: "Platonic solids that turn inside out and get spikey.",
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

    /// Every solid has to have as many faces as it has colours, and each face
    /// is one material change, so the batches count them.
    #[test]
    fn each_solid_draws_all_of_its_faces() {
        for (count, faces) in [(1, 4), (2, 6), (3, 8), (4, 12), (5, 20)] {
            let mut r = start(StartArgs::new(
                640,
                480,
                &format!("count={count}"),
                20260811,
            ));
            r.step();
            let f = r.frame();
            // A face is drawn as several strips, all under the same material,
            // so count the distinct materials in order rather than the batches.
            let mut changes = 1;
            for w in f.batches.windows(2) {
                if w[0].material.ambient_diffuse != w[1].material.ambient_diffuse {
                    changes += 1;
                }
            }
            assert_eq!(changes, faces, "count={count} drew {changes} faces");
        }
    }

    #[test]
    fn the_middle_of_a_face_stays_put_and_the_corners_travel() {
        // The displacement falls off as the square of the radius from the
        // centre of the face, which is the whole of the effect.
        let mut r = start(StartArgs::new(640, 480, "count=2", 20260811));
        r.step();
        let f = r.frame();
        let radius =
            |v: &crate::runtime::gl::Vertex| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1]).sqrt();
        // For a cube face at z = 0.5 * edge, the vertex nearest the middle of
        // the face keeps its z and the ones at the corners do not.
        let mid = f
            .vertices
            .iter()
            .min_by(|a, b| radius(a).total_cmp(&radius(b)))
            .expect("no vertices");
        let far = f
            .vertices
            .iter()
            .max_by(|a, b| radius(a).total_cmp(&radius(b)))
            .expect("no vertices");
        assert!(
            (mid.pos[2].abs() - 1.0).abs() < 0.05,
            "the middle moved to {}",
            mid.pos[2]
        );
        assert!(
            (far.pos[2].abs() - 1.0).abs() > 0.1,
            "the corner did not move, {}",
            far.pos[2]
        );
    }

    #[test]
    fn the_spikes_turn_culling_off() {
        // The faces invert once the displacement pushes their corners through
        // the middle, and the saver has to stop culling or the spikes vanish.
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        let mut culled = 0;
        let mut open = 0;
        for _ in 0..300 {
            r.step();
            if r.frame().batches[0].cull_face {
                culled += 1;
            } else {
                open += 1;
            }
        }
        assert!(culled > 20, "never culled, {culled} frames");
        assert!(open > 20, "never stopped culling, {open} frames");
    }

    #[test]
    fn the_face_colour_lifts_towards_grey_rather_than_towards_itself() {
        // Upstream sets only GL_DIFFUSE, so the ambient stays at GL's grey
        // under a scene ambient of a half. Setting both would make every face
        // half its own colour before a light touched it.
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        r.step();
        let m = r.frame().batches[0].material;
        assert_eq!(m.ambient, [0.2, 0.2, 0.2, 1.0]);
        assert_ne!(m.ambient_diffuse, m.ambient);
    }

    #[test]
    fn poking_it_moves_on_to_the_next_solid() {
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        r.step();
        let before = r.frame().batches.len();
        r.event(XEvent::KeyPress { key: ' ' });
        r.step();
        assert_ne!(before, r.frame().batches.len(), "still the tetrahedron");
    }
}
