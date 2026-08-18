//! Port of `hacks/glx/surfaces.c`.
//!
//! ```text
//! Surface --- Parametric 3d surfaces visualization
//!
//! Revision History:
//! 2000: written by Andrey Mirtchovski <mirtchov@cpsc.ucalgary.ca>
//!
//! 01-Mar-2003  mirtchov    Modified as a xscreensaver hack.
//! 01-jan-2009  steger      Renamed from klein.c to surfaces.c.
//!                          Removed the Klein bottle.
//!                          Added many new surfaces.
//!                          Added many command line options.
//! ```
//!
//! Parametric surfaces.
//!
//! Twelve named surfaces, each two nested loops over a pair of parameters and
//! three lines of arithmetic. Nothing is shaded and nothing is solid: a surface
//! is a mesh of points or lines, one block per step of the outer parameter, and
//! what you see of its shape is what the wireframe tells you.
//!
//! The colour is the position: a point at `(x, y, z)` is coloured
//! `(x + 0.7, y + 0.7, z + 0.7)`, clamped by the pipeline at each end. So a
//! surface that runs off in one direction saturates to a face of the colour
//! cube, and the pinch points, where all three coordinates are small, are the
//! only place the colour is dark. It costs nothing and it reads as depth.
//!
//! Two or three of the surfaces have a shape parameter driven by a sine, so
//! they breathe; the rest are fixed and only the camera moves. Every `speed`
//! frames, a random surface or a random way of drawing it is picked again.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, SelectItem, StartArgs, Trackball, XEvent,
    random,
};

/// The surfaces, in upstream's order: the index is what the random pick lands
/// on and what the line-loop rule below tests against.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Dini,
    Enneper,
    Kuen,
    Moebius,
    Seashell,
    Swallowtail,
    Bohemian,
    Whitney,
    Pluecker,
    Henneberg,
    Catalan,
    Corkscrew,
}

const SURFACES: [Surface; 12] = [
    Surface::Dini,
    Surface::Enneper,
    Surface::Kuen,
    Surface::Moebius,
    Surface::Seashell,
    Surface::Swallowtail,
    Surface::Bohemian,
    Surface::Whitney,
    Surface::Pluecker,
    Surface::Henneberg,
    Surface::Catalan,
    Surface::Corkscrew,
];

impl Surface {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "dini" => Surface::Dini,
            "enneper" => Surface::Enneper,
            "kuen" => Surface::Kuen,
            "moebius" => Surface::Moebius,
            "seashell" => Surface::Seashell,
            "swallowtail" => Surface::Swallowtail,
            "bohemian" => Surface::Bohemian,
            "whitney" => Surface::Whitney,
            "pluecker" => Surface::Pluecker,
            "henneberg" => Surface::Henneberg,
            "catalan" => Surface::Catalan,
            "corkscrew" => Surface::Corkscrew,
            _ => return None,
        })
    }

    /// Three of them close on themselves along the inner parameter, so their
    /// mesh lines are loops. The rest are open and get strips, or the last
    /// segment would be a chord back across the whole surface.
    fn closes(self) -> bool {
        matches!(
            self,
            Surface::Bohemian | Surface::Pluecker | Surface::Henneberg
        )
    }
}

/// The three ways of drawing one. Upstream skips polygons and triangle fans as
/// too slow, and strips and quads as not good enough to look at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Points,
    Lines,
    LineLoop,
}

const MODES: [Mode; 3] = [Mode::Points, Mode::Lines, Mode::LineLoop];

struct Surfaces {
    rot: Rotator,
    trackball: Trackball,

    surface: Surface,
    random_surface: bool,
    /// What each block of the mesh is drawn as, worked out from [`Mode`] and
    /// which surface is up.
    shape: Shape,
    random_render: bool,
    frame: i32,
    speed: i32,

    du: f64,
    dv: f64,
    /// The three shape parameters, walked round a sine so that the surfaces
    /// that use them breathe.
    a: f64,
    b: f64,
    c: f64,
    draw_step: f64,
}

/// `x` from `low` up to but not past `high`, the way a C `for` with a float
/// counter walks it. Written out because the accumulated rounding is part of
/// how many steps there are, and so of what the mesh looks like.
fn walk(low: f64, high: f64, step: f64) -> impl Iterator<Item = f64> {
    let mut x = low;
    std::iter::from_fn(move || {
        if x >= high {
            return None;
        }
        let now = x;
        x += step;
        Some(now)
    })
}

/// The same, but for the loops upstream writes with `<=`.
fn walk_inclusive(low: f64, high: f64, step: f64) -> impl Iterator<Item = f64> {
    let mut x = low;
    std::iter::from_fn(move || {
        if x > high {
            return None;
        }
        let now = x;
        x += step;
        Some(now)
    })
}

impl Surfaces {
    /// One block of the mesh, coloured by where each point is.
    fn strip(&self, g: &mut Gl, points: impl Iterator<Item = [f32; 3]>) {
        g.glx.begin(self.shape);
        for p in points {
            g.glx.color4f(p[0] + 0.7, p[1] + 0.7, p[2] + 0.7, 1.0);
            g.glx.vertex3f(p[0], p[1], p[2]);
        }
        g.glx.end();
    }

    fn draw_surface(&self, g: &mut Gl) {
        let pi = std::f64::consts::PI;
        let (du, dv) = (self.du, self.dv);
        let (a, b, c) = (self.a, self.b, self.c);
        let f = |x: f64, y: f64, z: f64| [x as f32, y as f32, z as f32];

        match self.surface {
            Surface::Dini => {
                for v in walk_inclusive(0.11, 2.0, dv) {
                    self.strip(
                        g,
                        walk_inclusive(0.0, 6.0 * pi, du).map(|u| {
                            f(
                                a * u.cos() * v.sin(),
                                a * u.sin() * v.sin(),
                                a * (v.cos() + (0.5 * v).tan().ln()) + 0.2 * b * u,
                            )
                        }),
                    );
                }
            }
            Surface::Enneper => {
                for u in walk_inclusive(-pi, pi, du) {
                    self.strip(
                        g,
                        walk(-pi, pi, dv).map(|v| {
                            f(
                                a * (u - (1.0 / 3.0 * u * u * u) + u * v * v),
                                b * (v - (1.0 / 3.0 * v * v * v) + u * u * v),
                                u * u - v * v,
                            )
                        }),
                    );
                }
            }
            Surface::Kuen => {
                for u in walk_inclusive(-4.48, 4.48, du) {
                    self.strip(
                        g,
                        walk(pi / 51.0, pi, dv).map(|v| {
                            let d = 1.0 + u * u * v.sin() * v.sin();
                            f(
                                2.0 * (u.cos() + u * u.sin()) * v.sin() / d,
                                2.0 * (u.sin() - u * u.cos()) * v.sin() / d,
                                (0.5 * v).tan().ln() + 2.0 * v.cos() / d,
                            )
                        }),
                    );
                }
            }
            Surface::Moebius => {
                for u in walk(-pi, pi, du) {
                    self.strip(
                        g,
                        walk(-0.735, 0.74, dv).map(|v| {
                            f(
                                u.cos() + v * (u / 2.0).cos() * u.cos(),
                                u.sin() + v * (u / 2.0).cos() * u.sin(),
                                v * (u / 2.0).sin(),
                            )
                        }),
                    );
                }
            }
            Surface::Seashell => {
                for u in walk(0.0, 2.0 * pi, du) {
                    self.strip(
                        g,
                        walk(0.0, 2.0 * pi, dv).map(|v| {
                            let taper = a * (1.0 - v / (2.0 * pi));
                            f(
                                taper * (2.0 * v).cos() * (1.0 + u.cos()) + c * (2.0 * v).cos(),
                                taper * (2.0 * v).sin() * (1.0 + u.cos()) + c * (2.0 * v).sin(),
                                2.0 * b * v / (2.0 * pi) + taper * u.sin(),
                            )
                        }),
                    );
                }
            }
            Surface::Swallowtail => {
                for u in walk(-2.5, 2.0, du) {
                    self.strip(
                        g,
                        walk(-1.085, 1.09, dv).map(|v| {
                            f(
                                u * v * v + 3.0 * v * v * v * v,
                                -2.0 * u * v - 4.0 * v * v * v,
                                u,
                            )
                        }),
                    );
                }
            }
            Surface::Bohemian => {
                for u in walk(-pi, pi, du) {
                    self.strip(
                        g,
                        walk(-pi, pi, dv)
                            .map(|v| f(a * u.cos(), b * v.cos() + a * u.sin(), v.sin())),
                    );
                }
            }
            Surface::Whitney => {
                for v in walk(-1.995, 2.0, dv) {
                    self.strip(g, walk(-1.995, 2.0, du).map(|u| f(u * v, u, v * v - 2.0)));
                }
            }
            // The two parameters are stepped by each other's increment here,
            // which is upstream's and makes no difference while they are equal.
            Surface::Pluecker => {
                for u in walk(0.0, 2.5, dv) {
                    self.strip(
                        g,
                        walk(-pi, pi, du)
                            .map(|v| f(u * v.cos(), u * v.sin(), 2.0 * v.cos() * v.sin())),
                    );
                }
            }
            Surface::Henneberg => {
                for u in walk(0.9, 2.55, dv) {
                    self.strip(
                        g,
                        walk(-pi, pi, du).map(|v| {
                            f(
                                (u / 3.0).sinh() * v.cos() - 1.0 / 3.0 * u.sinh() * (3.0 * v).cos(),
                                (u / 3.0).sinh() * v.sin() + 1.0 / 3.0 * u.sinh() * (3.0 * v).sin(),
                                (2.0 / 3.0 * u).cosh() * (2.0 * v).cos(),
                            )
                        }),
                    );
                }
            }
            Surface::Catalan => {
                for v in walk(-2.0, 2.0, du) {
                    self.strip(
                        g,
                        walk(-2.0 * pi, 2.0 * pi + 0.05, dv).map(|u| {
                            f(
                                0.33 * (u - u.sin() * v.cosh()),
                                0.33 * (1.0 - u.cos() * v.cosh()),
                                0.33 * 4.0 * (0.5 * u).sin() * (0.5 * v).sinh(),
                            )
                        }),
                    );
                }
            }
            Surface::Corkscrew => {
                for v in walk(-pi, pi, du) {
                    self.strip(
                        g,
                        walk(-pi, pi, dv).map(|u| {
                            let r = 0.5 * (a + 2.0);
                            f(
                                r * u.cos() * v.cos(),
                                r * u.sin() * v.cos(),
                                r * v.sin() + u,
                            )
                        }),
                    );
                }
            }
        }
    }

    /// Which primitive a mode means, which for line loops depends on whether
    /// the surface closes.
    fn shape_for(mode: Mode, surface: Surface) -> Shape {
        match mode {
            Mode::Points => Shape::Points,
            Mode::Lines => Shape::Lines,
            Mode::LineLoop => {
                if surface.closes() {
                    Shape::LineLoop
                } else {
                    Shape::LineStrip
                }
            }
        }
    }
}

impl Hack3d for Surfaces {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);

        g.glx.push_matrix();
        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 10.0,
            (y as f32 - 0.5) * 10.0,
            (z as f32 - 0.5) * 20.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        g.glx.scale(4.0, 4.0, 4.0);

        self.draw_surface(g);
        g.glx.pop_matrix();

        // Two steps a frame, and `b` is a quarter turn further round than `a`
        // because it is read after the second one. Upstream writes both
        // increments inside the calls.
        self.draw_step += 0.01;
        self.a = self.draw_step.sin();
        self.draw_step += 0.01;
        self.b = self.draw_step.cos();
        self.c = (self.draw_step + 0.25 * std::f64::consts::PI).sin();

        if self.random_surface || self.random_render {
            self.frame += 1;
            if self.frame >= self.speed {
                self.frame = 0;
                if self.random_surface {
                    self.surface = SURFACES[random() as usize % SURFACES.len()];
                }
                if self.random_render {
                    // After the surface, so a fresh line-loop pick knows
                    // whether the new surface closes.
                    let mode = MODES[random() as usize % MODES.len()];
                    self.shape = Self::shape_for(mode, self.surface);
                }
            }
        }

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
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.bool("spin");
    let spin_speed = 1.0;
    let wander_speed = 0.03;

    let name = g.res.get("surface").unwrap_or("random").to_string();
    let random_surface = Surface::from_name(&name).is_none();
    let surface =
        Surface::from_name(&name).unwrap_or_else(|| SURFACES[random() as usize % SURFACES.len()]);

    let mode_name = g.res.get("mode").unwrap_or("random").to_string();
    let (random_render, mode) = match mode_name.as_str() {
        "points" => (false, Mode::Points),
        "lines" => (false, Mode::Lines),
        "line-loops" => (false, Mode::LineLoop),
        _ => (true, MODES[random() as usize % MODES.len()]),
    };

    let mut st = Surfaces {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            1.0,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            true,
        ),
        trackball: Trackball::new(),
        surface,
        random_surface,
        shape: Surfaces::shape_for(mode, surface),
        random_render,
        frame: 0,
        speed: g.res.int("speed").max(1),
        du: 0.07,
        dv: 0.07,
        a: 1.0,
        b: 1.0,
        c: 0.1,
        draw_step: 0.0,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*suppressRotationAnimation: True",
    "*surface:      random",
    "*mode:         random",
    "*spin:         True",
    "*wander:       False",
    "*speed:        300",
];

const SURFACE_ITEMS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random Surface",
    },
    SelectItem {
        value: "dini",
        label: "Dini's Surface",
    },
    SelectItem {
        value: "enneper",
        label: "Enneper's Surface",
    },
    SelectItem {
        value: "kuen",
        label: "Kuen Surface",
    },
    SelectItem {
        value: "moebius",
        label: "Möbius Strip",
    },
    SelectItem {
        value: "seashell",
        label: "Seashell",
    },
    SelectItem {
        value: "swallowtail",
        label: "Swallowtail",
    },
    SelectItem {
        value: "bohemian",
        label: "Bohemian Dome",
    },
    SelectItem {
        value: "whitney",
        label: "Whitney Umbrella",
    },
    SelectItem {
        value: "pluecker",
        label: "Pluecker's Conoid",
    },
    SelectItem {
        value: "henneberg",
        label: "Henneberg's Surface",
    },
    SelectItem {
        value: "catalan",
        label: "Catalan's Surface",
    },
    SelectItem {
        value: "corkscrew",
        label: "Corkscrew Surface",
    },
];

const MODE_ITEMS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random Display Mode",
    },
    SelectItem {
        value: "points",
        label: "Points",
    },
    SelectItem {
        value: "lines",
        label: "Lines",
    },
    SelectItem {
        value: "line-loops",
        label: "Line Loops",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Duration", 2.0, 2000.0, 1.0, 0, "300").inverted(),
    Opt::select("surface", "Surface", SURFACE_ITEMS, "random"),
    Opt::select("mode", "Display mode", MODE_ITEMS, "random"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "surfaces",
    label: "Surfaces",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Andrey Mirtchovski and Carsten Steger",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=Q412lxz3fTg"),
        blurb: "Parametric surfaces.",
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

    /// Every one of the twelve draws a mesh, and none of them lands on a
    /// coordinate that is not a number: several take a logarithm or a tangent
    /// near where it blows up, and the ranges are chosen to stop just short.
    #[test]
    fn every_surface_is_finite() {
        for s in SURFACE_ITEMS.iter().skip(1) {
            let r = run(&format!("surface={}&mode=lines", s.value), 3);
            let f = r.frame();
            assert!(!f.vertices.is_empty(), "{} drew nothing", s.value);
            for v in &f.vertices {
                assert!(
                    v.pos.iter().all(|x| x.is_finite()),
                    "{} put {:?} on the screen",
                    s.value,
                    v.pos
                );
            }
        }
    }

    /// The colour is the position shifted by 0.7, which is what makes the
    /// pinch points dark and the far edges saturate.
    #[test]
    fn the_colour_is_the_position() {
        let r = run("surface=moebius&mode=points", 2);
        let f = r.frame();
        for v in &f.vertices {
            for k in 0..3 {
                assert!(
                    (v.color[k] - (v.pos[k] + 0.7)).abs() < 1e-6,
                    "colour {:?} is not position {:?} plus 0.7",
                    v.color,
                    v.pos
                );
            }
        }
    }

    /// A surface that closes along its inner parameter is drawn as loops; one
    /// that does not gets strips, or the last segment would cut back across it.
    #[test]
    fn only_the_closed_surfaces_get_loops() {
        for (name, want) in [
            ("bohemian", Primitive::LineLoop),
            ("pluecker", Primitive::LineLoop),
            ("henneberg", Primitive::LineLoop),
            ("moebius", Primitive::LineStrip),
            ("dini", Primitive::LineStrip),
            ("catalan", Primitive::LineStrip),
        ] {
            let r = run(&format!("surface={name}&mode=line-loops"), 2);
            assert!(
                r.frame().batches.iter().all(|b| b.primitive == want),
                "{name} should be drawn as {want:?}"
            );
        }
    }

    /// The three modes are what they say, and the mesh is blocks of the outer
    /// parameter rather than one long run.
    #[test]
    fn each_mode_draws_what_it_says() {
        for (mode, want) in [
            ("points", Primitive::Points),
            ("lines", Primitive::Lines),
            ("line-loops", Primitive::LineStrip),
        ] {
            let r = run(&format!("surface=moebius&mode={mode}"), 2);
            let f = r.frame();
            assert!(f.batches.iter().all(|b| b.primitive == want));
            // Points and lines fold together; the strips stay apart, one a
            // step of the outer parameter.
            if want == Primitive::LineStrip {
                assert!(
                    f.batches.len() > 50,
                    "{} blocks is not a mesh",
                    f.batches.len()
                );
            }
        }
    }

    /// A fixed surface stays put; a random one is swapped out every `speed`
    /// frames, which is what the duration knob sets.
    #[test]
    fn the_surface_changes_on_the_beat() {
        let fixed = run("surface=whitney&mode=points", 40);
        let n = fixed.frame().vertices.len();
        let mut r = start(StartArgs::new(640, 480, "surface=whitney&mode=points", 7));
        for _ in 0..200 {
            r.step();
            assert_eq!(r.frame().vertices.len(), n, "a fixed surface changed");
        }

        // Random, with a short duration: over a long enough run more than one
        // surface has to show up, and they have different vertex counts.
        let mut r = start(StartArgs::new(640, 480, "mode=points&speed=2", 20260811));
        let mut sizes = std::collections::BTreeSet::new();
        for _ in 0..300 {
            r.step();
            sizes.insert(r.frame().vertices.len());
        }
        assert!(sizes.len() > 3, "only saw {} shapes", sizes.len());
    }

    /// The shape parameters walk round a sine, two steps a frame, so the
    /// surfaces that use them breathe rather than sit still.
    #[test]
    fn the_shape_parameters_keep_walking() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "surface=seashell&mode=points",
            20260811,
        ));
        let mut widest: Vec<f32> = Vec::new();
        for _ in 0..80 {
            r.step();
            let f = r.frame();
            widest.push(f.vertices.iter().map(|v| v.pos[0]).fold(f32::MIN, f32::max));
        }
        let lo = widest.iter().copied().fold(f32::MAX, f32::min);
        let hi = widest.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 0.01, "the seashell never changed size");
    }
}
