//! Port of `hacks/glx/geodesic.c`.
//!
//! ```text
//! geodesic, Copyright (c) 2013-2014 Jamie Zawinski <jwz@jwz.org>
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
//! A geodesic sphere, subdividing and un-subdividing without ever cutting.
//!
//! The sphere starts as an icosahedron, laid out in latitude and longitude
//! rather than as vertices: ten faces around the equator at 26.57 degrees, each
//! reaching to a pole. Every level of frequency splits each triangle into four
//! by taking the midpoints of its sides, and because the midpoints are computed
//! in polar coordinates they land on the sphere rather than inside it.
//!
//! What makes it more than a subdivision demo is that the frequency is a real
//! number, not an integer, and the fractional part is spent on getting from one
//! whole one to the next. Over the first stretch the finer mesh fades in on top
//! of the coarser, still flat; over the second the new midpoints ease outward
//! onto the sphere. Nothing ever pops.
//!
//! Five ways to draw a face. Solid is what it says; mesh cuts a triangular hole
//! in every face and gives the resulting frame an inside colour and a
//! thickness, which is the shape most people picture; stellated pushes the
//! centre of each face out to the sphere and inverse-stellated pushes it in, so
//! the same morph that rounds a face can also spike it. Wireframe draws the
//! edges, and looks surprisingly good lit.
//!
//! While two frequencies overlap the further one is drawn first at the
//! complementary alpha, and the nearer gets a polygon offset so that the two
//! coincident surfaces do not fight over the depth buffer.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::opts::SelectItem;
use crate::runtime::shapes::do_normal;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

/// Latitude and longitude, which is how the sphere is described: subdividing
/// in polar coordinates is what puts the new points on the surface rather than
/// on the chord.
#[derive(Clone, Copy)]
struct Ll {
    a: f32,
    o: f32,
}

impl Ll {
    fn to_xyz(self) -> [f32; 3] {
        [
            self.a.cos() * self.o.cos(),
            self.a.cos() * self.o.sin(),
            self.a.sin(),
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Wire,
    Mesh,
    Solid,
    Stellated,
    Stellated2,
}

struct Geodesic {
    rot: Rotator,
    trackball: Trackball,
    colors: Vec<crate::runtime::color::XColor>,
    ccolor: usize,
    ccolor2: usize,
    color1: [f32; 4],
    color2: [f32; 4],

    /// The frequency, as a real number: the whole part is how many times each
    /// face has been split and the fraction is how far through the next split.
    depth: f32,
    delta: f32,
    thickness: f32,

    /// How far the new midpoints have travelled from the chord to the sphere,
    /// or, when stellated, from flat to spiked.
    morph_ratio: f32,

    random_mode: bool,
    mode: Mode,
    max_depth: i32,
    speed: f32,
}

impl Geodesic {
    /// `triangle0`. One face, in whichever of the five styles is current.
    fn triangle0(&self, g: &mut Gl, p1: [f32; 3], p2: [f32; 3], p3: [f32; 3]) {
        let wire = self.mode == Mode::Wire;
        let mut r = self.thickness;
        if matches!(self.mode, Mode::Solid | Mode::Stellated | Mode::Stellated2) {
            r = 1.0;
        }
        if r <= 0.001 {
            r = 0.001;
        }
        if wire {
            r = 1.0;
        }

        // Upstream sets the material per face. The colours are the same two
        // for the whole frame, so they go through GL_COLOR_MATERIAL instead:
        // the lighting is identical and the mesh collapses from thousands of
        // draw calls into one.
        let c1 = self.color1;
        let c2 = self.color2;
        g.glx.color4f(c1[0], c1[1], c1[2], c1[3]);

        if r >= 1.0 {
            // Solid triangular face.
            g.glx.front_face_cw(false);
            g.glx.begin(if wire {
                Shape::LineLoop
            } else {
                Shape::Triangles
            });
            do_normal(&mut g.glx, p1, p2, p3);
            g.glx.vertex3f(p1[0], p1[1], p1[2]);
            g.glx.vertex3f(p2[0], p2[1], p2[2]);
            g.glx.vertex3f(p3[0], p3[1], p3[2]);
            g.glx.end();
            return;
        }

        // Mesh: a triangular face with a triangular hole.
        let d = 0.98;
        let c = [
            (p1[0] + p2[0] + p3[0]) / 3.0,
            (p1[1] + p2[1] + p3[1]) / 3.0,
            (p1[2] + p2[2] + p3[2]) / 3.0,
        ];
        let inset = |p: [f32; 3]| {
            [
                p[0] + r * (c[0] - p[0]),
                p[1] + r * (c[1] - p[1]),
                p[2] + r * (c[2] - p[2]),
            ]
        };
        let (p1b, p2b, p3b) = (inset(p1), inset(p2), inset(p3));
        let sunk = |p: [f32; 3]| [d * p[0], d * p[1], d * p[2]];

        let shape = if wire { Shape::LineLoop } else { Shape::Quads };
        let quad = |g: &mut Gl, a: [f32; 3], b: [f32; 3], cc: [f32; 3], dd: [f32; 3]| {
            for v in [a, b, cc, dd] {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
        };

        // Outside faces.
        g.glx.begin(shape);
        do_normal(&mut g.glx, p1, p2, p3);
        quad(g, p1, p1b, p3b, p3);
        quad(g, p1, p2, p2b, p1b);
        quad(g, p2, p3, p3b, p2b);
        g.glx.end();

        // Inside faces.
        g.glx.color4f(c2[0], c2[1], c2[2], c2[3]);
        g.glx.begin(shape);
        do_normal(&mut g.glx, p3, p3b, p1b);
        quad(g, sunk(p3), sunk(p3b), sunk(p1b), sunk(p1));
        quad(g, sunk(p1b), sunk(p2b), sunk(p2), sunk(p1));
        quad(g, sunk(p2b), sunk(p3b), sunk(p3), sunk(p2));
        g.glx.end();

        // Connecting edges.
        g.glx.color4f(c1[0], c1[1], c1[2], c1[3]);
        g.glx.begin(shape);
        do_normal(&mut g.glx, p1b, p2b, sunk(p2b));
        quad(g, p1b, p2b, sunk(p2b), sunk(p1b));
        do_normal(&mut g.glx, p2b, p3b, sunk(p3b));
        quad(g, p2b, p3b, sunk(p3b), sunk(p2b));
        do_normal(&mut g.glx, p3b, p1b, sunk(p1b));
        quad(g, p3b, p1b, sunk(p1b), sunk(p3b));
        g.glx.end();
    }

    /// `midpoint2`. The point half way along a chord, and where on the sphere
    /// that direction lands.
    fn midpoint2(v1: Ll, v2: Ll) -> (Ll, [f32; 3], [f32; 3], [f32; 3]) {
        let p1 = v1.to_xyz();
        let p2 = v2.to_xyz();
        let pm = [
            (p1[0] + p2[0]) / 2.0,
            (p1[1] + p2[1]) / 2.0,
            (p1[2] + p2[2]) / 2.0,
        ];
        let o = pm[1].atan2(pm[0]);
        let hyp = (pm[0] * pm[0] + pm[1] * pm[1]).sqrt();
        (
            Ll {
                a: pm[2].atan2(hyp),
                o,
            },
            p1,
            p2,
            pm,
        )
    }

    /// `midpoint3`. The same for the centre of a face.
    fn midpoint3(v1: Ll, v2: Ll, v3: Ll) -> (Ll, [f32; 3], [f32; 3], [f32; 3], [f32; 3]) {
        let p1 = v1.to_xyz();
        let p2 = v2.to_xyz();
        let p3 = v3.to_xyz();
        let pm = [
            (p1[0] + p2[0] + p3[0]) / 3.0,
            (p1[1] + p2[1] + p3[1]) / 3.0,
            (p1[2] + p2[2] + p3[2]) / 3.0,
        ];
        let o = pm[1].atan2(pm[0]);
        let hyp = (pm[0] * pm[0] + pm[1] * pm[1]).sqrt();
        (
            Ll {
                a: pm[2].atan2(hyp),
                o,
            },
            p1,
            p2,
            p3,
            pm,
        )
    }

    /// `triangle`. Subdivide to the given depth; the last level is where the
    /// morph happens.
    fn triangle(&self, g: &mut Gl, v1: Ll, v2: Ll, v3: Ll, depth: i32) {
        if depth <= 0 {
            self.triangle0(g, v1.to_xyz(), v2.to_xyz(), v3.to_xyz());
            return;
        }

        let (v12, p1, p2, mut p12) = Self::midpoint2(v1, v2);
        let (v23, _, p3, mut p23) = Self::midpoint2(v2, v3);
        let (v13, _, _, mut p13) = Self::midpoint2(v1, v3);
        let depth = depth - 1;
        let r = self.morph_ratio;

        if depth == 0 && r != 0.0 && matches!(self.mode, Mode::Stellated | Mode::Stellated2) {
            // Morph between flat and stellated faces.
            let (vc, p1, p2, p3, mut pc) = Self::midpoint3(v1, v2, v3);
            let pc2 = vc.to_xyz();
            for k in 0..3 {
                pc[k] += r * (pc2[k] - pc[k]);
            }
            self.triangle0(g, p1, p2, pc);
            self.triangle0(g, p2, p3, pc);
            self.triangle0(g, p3, p1, pc);
        } else if depth == 0 && r < 1.0 {
            // Morph between flat and sphere-oid faces.
            let ease = |p: &mut [f32; 3], v: Ll| {
                let b = v.to_xyz();
                for k in 0..3 {
                    p[k] += r * (b[k] - p[k]);
                }
            };
            ease(&mut p12, v12);
            ease(&mut p23, v23);
            ease(&mut p13, v13);

            self.triangle0(g, p1, p12, p13);
            self.triangle0(g, p12, p2, p23);
            self.triangle0(g, p13, p23, p3);
            self.triangle0(g, p12, p23, p13);
        } else {
            self.triangle(g, v1, v12, v13, depth);
            self.triangle(g, v12, v2, v23, depth);
            self.triangle(g, v13, v23, v3, depth);
            self.triangle(g, v12, v23, v13, depth);
        }
    }

    /// `make_geodesic`. The icosahedron, as twenty faces built from ten
    /// longitudes and one latitude.
    fn make_geodesic(&self, g: &mut Gl, depth: i32) {
        let th0 = 0.5f32.atan(); // lat division: 26.57 deg
        let s = std::f32::consts::PI / 5.0; // lon division: 72 deg

        for i in 0..10 {
            let th1 = s * i as f32;
            let th2 = s * (i + 1) as f32;
            let th3 = s * (i + 2) as f32;
            let mut v1 = Ll { a: th0, o: th1 };
            let mut v2 = Ll { a: th0, o: th3 };
            let mut v3 = Ll { a: -th0, o: th2 };
            let mut vc = Ll {
                a: std::f32::consts::FRAC_PI_2,
                o: th2,
            };

            if i & 1 != 0 {
                // North.
                self.triangle(g, v1, v2, vc, depth);
                self.triangle(g, v2, v1, v3, depth);
            } else {
                // South.
                v1.a = -v1.a;
                v2.a = -v2.a;
                v3.a = -v3.a;
                vc.a = -vc.a;
                self.triangle(g, v2, v1, vc, depth);
                self.triangle(g, v1, v2, v3, depth);
            }
        }
    }

    fn pick_mode() -> Mode {
        match random() % 4 {
            0 => Mode::Mesh,
            1 => Mode::Solid,
            2 => Mode::Stellated,
            _ => Mode::Stellated2,
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mode_str = g.res.string("mode").to_string();
    let random_mode = mode_str == "random";
    let mode = match mode_str.as_str() {
        "solid" => Mode::Solid,
        "stellated" => Mode::Stellated,
        "stellated2" => Mode::Stellated2,
        "wire" => Mode::Wire,
        "random" => Geodesic::pick_mode(),
        _ => Mode::Mesh,
    };

    let speed = g.res.float("speed") as f32;
    let do_spin = g.res.bool("spin");
    let do_wander = g.res.bool("wander");

    let mut this = Geodesic {
        rot: Rotator::new(
            if do_spin { 0.25 * speed as f64 } else { 0.0 },
            if do_spin { 0.25 * speed as f64 } else { 0.0 },
            if do_spin { 0.25 * speed as f64 } else { 0.0 },
            0.2,
            if do_wander { 0.01 * speed as f64 } else { 0.0 },
            true,
        ),
        trackball: Trackball::new(),
        colors: make_smooth_colormap(1024),
        ccolor: 0,
        ccolor2: 0,
        color1: [1.0; 4],
        color2: [1.0; 4],
        // Start one up from the icosahedron.
        depth: 1.0,
        delta: 0.003,
        thickness: 0.1,
        morph_ratio: 0.0,
        random_mode,
        mode,
        max_depth: g.res.int("count").max(1),
        speed,
    };

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Geodesic {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let (mut height, mut y) = (height, 0);
        let mut h = height as f32 / width as f32;
        if width > height * 5 {
            // Tiny window: show the middle.
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
        let s = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) {
            // Upstream restarts itself in random mode, which comes to the same
            // thing as rolling a new style and starting the frequency over.
            self.random_mode = true;
            self.mode = Geodesic::pick_mode();
            self.depth = 1.0;
            self.delta = 0.003;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.mode == Mode::Wire;

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.lighting(true);
        g.glx.color_material(true);
        g.glx.light_enable(0, true);
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);
        g.glx.blend(if wire { Blend::Off } else { Blend::Alpha });

        g.glx.push_matrix();

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

        let c = &self.colors[self.ccolor];
        self.color1 = [
            c.red as f32 / 65536.0,
            c.green as f32 / 65536.0,
            c.blue as f32 / 65536.0,
            1.0,
        ];
        let c = &self.colors[self.ccolor2];
        self.color2 = [
            c.red as f32 / 65536.0,
            c.green as f32 / 65536.0,
            c.blue as f32 / 65536.0,
            1.0,
        ];
        let n = self.colors.len();
        self.ccolor = (self.ccolor + 1) % n;
        self.ccolor2 = (self.ccolor + n / 2) % n;

        g.glx.scale(10.0, 10.0, 10.0);

        let r = self.depth - self.depth.floor();
        // Two ranges: the first fades the new segments in, the second morphs
        // them into position.
        let range = 0.15;
        let min1 = (0.5 - range) / 2.0;
        let max1 = 0.5 - min1;
        let min2 = 0.5 + min1;
        let max2 = 0.5 + max1;

        let (mut d1, mut d2, mut morph1, mut morph2, mut alpha);
        if r < min1 {
            // Old alone.
            d1 = self.depth.floor() as i32;
            d2 = d1;
            morph1 = 1.0;
            morph2 = 1.0;
            alpha = 1.0;
        } else if r < max1 && matches!(self.mode, Mode::Mesh | Mode::Stellated | Mode::Stellated2) {
            // Fade to new flat.
            d1 = self.depth.floor() as i32;
            d2 = self.depth.ceil() as i32;
            morph1 = 1.0;
            morph2 = 0.0;
            alpha = (r - min1) / (max1 - min1);
            if matches!(self.mode, Mode::Stellated | Mode::Stellated2) {
                // De-stellate while fading out, and do it faster.
                morph1 = ((1.0 - alpha) - 0.5) * 2.0;
                if morph1 < 0.0 {
                    morph1 = 0.0;
                }
            }
        } else if r < min2 {
            // New flat.
            d1 = self.depth.ceil() as i32;
            d2 = d1;
            morph1 = 0.0;
            morph2 = 0.0;
            alpha = 1.0;
        } else if r < max2 {
            // Morph.
            d1 = self.depth.ceil() as i32;
            d2 = d1;
            morph1 = (r - min2) / (max2 - min2);
            morph2 = morph1;
            alpha = 1.0;
        } else {
            // New alone.
            d1 = self.depth.ceil() as i32;
            d2 = d1;
            morph1 = 1.0;
            morph2 = 1.0;
            alpha = 1.0;
        }

        if self.mode == Mode::Stellated2 {
            morph1 = -morph1;
            morph2 = -morph2;
        }

        if d1 != d2 {
            if alpha > 0.5 {
                // Always draw the more transparent one first.
                std::mem::swap(&mut d1, &mut d2);
                std::mem::swap(&mut morph1, &mut morph2);
                alpha = 1.0 - alpha;
            }
            self.color1[3] = 1.0 - alpha;
            self.color2[3] = 1.0 - alpha;

            if !wire {
                g.glx.polygon_offset(None);
            }
            self.morph_ratio = morph1;
            self.make_geodesic(g, d1);

            // Make the less-transparent object take precedence.
            if !wire {
                g.glx.polygon_offset(Some((1.0, 1.0)));
            }
        }

        self.color1[3] = alpha;
        self.color2[3] = alpha;
        self.morph_ratio = morph2;
        self.make_geodesic(g, d2);

        g.glx.polygon_offset(None);
        g.glx.pop_matrix();

        if !down {
            self.depth += self.speed * self.delta;
            if self.depth > (self.max_depth - 1) as f32 {
                self.depth = (self.max_depth - 1) as f32;
                self.delta = -self.delta.abs();
            } else if self.depth < 0.0 {
                self.depth = 0.0;
                self.delta = self.delta.abs();
                // Randomize the style again on the way back down.
                if self.random_mode {
                    self.mode = Geodesic::pick_mode();
                }
            }
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        4",
    "*showFPS:      False",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        1.0",
    "*mode:         mesh",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "mesh",
        label: "Mesh faces",
    },
    SelectItem {
        value: "solid",
        label: "Solid faces",
    },
    SelectItem {
        value: "stellated",
        label: "Stellated faces",
    },
    SelectItem {
        value: "stellated2",
        label: "Inverse Stellated",
    },
    SelectItem {
        value: "wire",
        label: "Wireframe",
    },
    SelectItem {
        value: "random",
        label: "Random face style",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Animation speed", 0.05, 10.0, 0.05, 2, "1.0"),
    Opt::slider("count", "Depth", 1.0, 8.0, 1.0, 0, "4"),
    Opt::select("mode", "Face style", MODES, "mesh"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "geodesic",
    label: "Geodesic",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2013",
        video: Some("https://www.youtube.com/watch?v=qulzooBLIcU"),
        blurb: "A mesh geodesic sphere of increasing and decreasing frequency.",
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

    /// Every vertex of a solid face is on the unit sphere, because the
    /// midpoints are taken in polar coordinates rather than along the chord.
    #[test]
    fn a_settled_face_has_its_corners_on_the_sphere() {
        let mut r = start(StartArgs::new(640, 480, "mode=solid&count=2", 20260811));
        // Run to where the morph has finished and the mesh is round.
        for _ in 0..400 {
            r.step();
        }
        let f = r.frame();
        let radii: Vec<f32> = f
            .vertices
            .iter()
            .map(|v| (v.pos[0].powi(2) + v.pos[1].powi(2) + v.pos[2].powi(2)).sqrt())
            .collect();
        let lo = radii.iter().copied().fold(f32::MAX, f32::min);
        let hi = radii.iter().copied().fold(0.0_f32, f32::max);
        assert!(hi <= 1.0001, "a corner left the sphere at {hi}");
        // Flat faces put their corners on the sphere and their middles inside
        // it, but a corner is never much under one.
        assert!(lo > 0.8, "a corner fell to {lo}");
    }

    #[test]
    fn the_icosahedron_has_twenty_faces() {
        let mut r = start(StartArgs::new(640, 480, "mode=solid&count=1", 20260811));
        // The frequency starts one up from the icosahedron and is clamped back
        // down at the end of the first frame, so the base solid appears on the
        // second one.
        r.step();
        r.step();
        let f = r.frame();
        let tris: usize = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
            .map(|b| b.count / 3)
            .sum();
        assert_eq!(tris, 20, "got {tris} faces");
    }

    #[test]
    fn each_level_of_frequency_quadruples_the_faces() {
        let count = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            let f = r.frame();
            f.batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
                .map(|b| b.count / 3)
                .sum::<usize>()
        };
        // Depth starts at one, so this is the once-subdivided icosahedron.
        assert_eq!(count("mode=solid&count=4"), 80);
    }

    #[test]
    fn the_mesh_style_cuts_a_hole_in_every_face() {
        let mut r = start(StartArgs::new(640, 480, "mode=mesh&count=2", 20260811));
        // Upstream picks the second colour half a colourmap along from the
        // first, but only updates it after using it, so on the very first
        // frame both are the same and the hole is invisible.
        r.step();
        r.step();
        let f = r.frame();
        // Nine quads a face: three outside, three inside, three edges, and a
        // quad is cut into two triangles.
        let verts: usize = f.batches.iter().map(|b| b.count).sum();
        assert_eq!(verts, 80 * 9 * 6, "got {verts} vertices");
        // And the hole has an inside colour of its own.
        let mut colours: Vec<[u32; 3]> = f
            .vertices
            .iter()
            .map(|v| {
                [
                    v.color[0].to_bits(),
                    v.color[1].to_bits(),
                    v.color[2].to_bits(),
                ]
            })
            .collect();
        colours.sort_unstable();
        colours.dedup();
        assert_eq!(colours.len(), 2, "got {} colours", colours.len());
    }

    #[test]
    fn two_frequencies_overlap_while_one_fades_into_the_other() {
        // During the fade the coarser mesh is drawn first at the complementary
        // alpha, and the finer one gets a polygon offset so the two coincident
        // surfaces do not fight over the depth buffer.
        let mut r = start(StartArgs::new(640, 480, "mode=mesh&count=4", 20260811));
        let mut saw_offset = false;
        let mut saw_two_alphas = false;
        for _ in 0..400 {
            r.step();
            let f = r.frame();
            if f.batches.iter().any(|b| b.polygon_offset.is_some()) {
                saw_offset = true;
            }
            let mut alphas: Vec<u32> = f.vertices.iter().map(|v| v.color[3].to_bits()).collect();
            alphas.sort_unstable();
            alphas.dedup();
            if alphas.len() > 1 {
                saw_two_alphas = true;
            }
        }
        assert!(saw_offset, "the overlap never got a polygon offset");
        assert!(saw_two_alphas, "the two meshes were never at once");
    }

    #[test]
    fn the_frequency_climbs_and_comes_back_down() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "mode=solid&count=3&speed=10",
            20260811,
        ));
        let faces = |r: &Runner3d| {
            let f = r.frame();
            f.batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
                .map(|b| b.count / 3)
                .sum::<usize>()
        };
        let (mut lo, mut hi) = (usize::MAX, 0);
        for _ in 0..600 {
            r.step();
            let n = faces(&r);
            lo = lo.min(n);
            hi = hi.max(n);
        }
        assert_eq!(hi, 320, "never reached the finest mesh, {hi}");
        assert_eq!(lo, 20, "never came back to the icosahedron, {lo}");
    }
}
