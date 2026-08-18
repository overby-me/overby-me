//! Port of `hacks/glx/spheremonics.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 2002-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Algorithm by Paul Bourke <pbourke@swin.edu.au>
//! Screensaver veneer and parameter selection by jwz.
//! ```
//!
//! Closed surfaces from a formula with eight small integers in it.
//!
//! The whole shape is one line: at each latitude and longitude the radius is
//!
//! ```text
//! r = sin(m0 phi)^m1 + cos(m2 phi)^m3 + sin(m4 theta)^m5 + cos(m6 theta)^m7
//! ```
//!
//! and the eight exponents and multipliers are integers from zero to four. All
//! eight at zero is a sphere; raise one and the surface grows lobes, pinches
//! into a star, folds into a shell. As the degree rises it gets more pointed
//! and needs more polygons to stay honest.
//!
//! The veneer is what makes it a screen saver rather than a plotter. Only one
//! of the eight parameters changes at a time, and always in the direction it
//! was already going, bouncing off nought and four; so one shape leads to the
//! next instead of jumping somewhere unrelated. The change itself is hidden:
//! the object shrinks to nothing, is rebuilt, and grows back.
//!
//! The equation sometimes produces surfaces that are inside out, and telling
//! which is which is harder than it is worth, so nothing is culled and the
//! lighting is two-sided. That halves the dynamic range of the shading and
//! nobody minds.
//!
//! Two things happen at random and briefly: a circle sweeps down the object
//! like a contour line, or a wireframe copy is drawn slightly larger than the
//! solid one for a few frames.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::opts::SelectItem;
use crate::runtime::shapes::calc_normal;
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

/// The largest any one parameter is allowed to get. Upstream wonders about
/// nine and settles on four.
const M_MAX: i32 = 4;

struct Spheremonics {
    rot: Rotator,
    trackball: Trackball,
    font: Option<TexFont>,

    /// The eight parameters, and which way each of them is currently moving.
    m: [i32; 8],
    dm: [i32; 8],
    /// Fixed parameters from the knob, which stop the animation entirely.
    static_parms: bool,

    resolution: i32,
    colors: Vec<XColor>,
    /// Set by building the object: one over the largest side of its bounding
    /// box, so a spiky one is drawn no bigger than a round one.
    scale: f32,
    solid: u32,
    mesh: u32,
    grid: u32,
    polys: usize,

    /// How far through a sweeping contour line, or nothing.
    tracer: i32,
    /// How many frames of wireframe overlay are left, or nothing.
    mesher: i32,
    change_tick: i32,
    /// Positive while shrinking away, negative while growing back.
    fade: f32,
    done_once: bool,

    wireframe: bool,
    do_bbox: bool,
    do_grid: bool,
    smooth: bool,
    duration: i32,
}

/// `sphere_eval`: the formula, in one line, at one point.
fn sphere_eval(theta: f64, phi: f64, m: &[i32; 8]) -> [f32; 3] {
    let mut r = 0.0;
    r += (m[0] as f64 * phi).sin().powi(m[1]);
    r += (m[2] as f64 * phi).cos().powi(m[3]);
    r += (m[4] as f64 * theta).sin().powi(m[5]);
    r += (m[6] as f64 * theta).cos().powi(m[7]);
    [
        (r * phi.sin() * theta.cos()) as f32,
        (r * phi.cos()) as f32,
        (r * phi.sin() * theta.sin()) as f32,
    ]
}

impl Spheremonics {
    /// `draw_circle`: a ring, and optionally the tick marks around it that make
    /// it read as a dial.
    fn draw_circle(g: &mut Gl, teeth: bool) {
        let step = std::f32::consts::PI / 180.0;
        g.glx.begin(Shape::LineLoop);
        let mut th = 0.0f32;
        while th < std::f32::consts::PI * 2.0 {
            g.glx.vertex3f(th.cos() * 0.5, th.sin() * 0.5, 0.0);
            th += step * 5.0;
        }
        g.glx.end();

        if !teeth {
            return;
        }

        g.glx.begin(Shape::Lines);
        let mut th = 0.0f32;
        let mut tick = 0u32;
        while th < std::f32::consts::PI * 2.0 {
            let r1 = 0.5;
            let mut r2 = r1 - 0.01;
            if tick.is_multiple_of(10) {
                r2 -= 0.02;
            } else if tick.is_multiple_of(5) {
                r2 -= 0.01;
            }
            tick += 1;
            let (x, y) = (th.cos(), th.sin());
            g.glx.vertex3f(x * r1, y * r1, 0.0);
            g.glx.vertex3f(x * r2, y * r2, 0.0);
            th += step;
        }
        g.glx.end();
    }

    /// `draw_bounding_box`. Only ever visible with the bbox knob on, and it is
    /// a fixed unit cube rather than the object's actual extent, which is what
    /// upstream settled on.
    fn draw_bounding_box(&self, g: &mut Gl) {
        if !self.do_bbox || self.wireframe {
            return;
        }
        let (a, b) = (-0.5f32, 0.5f32);
        g.glx.material_ambient_diffuse([0.2, 0.2, 0.6, 1.0]);
        g.glx.front_face_cw(false);
        g.glx.cull_face(true);
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            (
                [0.0, 1.0, 0.0],
                [[a, a, a], [a, a, b], [b, a, b], [b, a, a]],
            ),
            (
                [0.0, -1.0, 0.0],
                [[b, b, a], [b, b, b], [a, b, b], [a, b, a]],
            ),
            (
                [0.0, 0.0, 1.0],
                [[a, a, a], [b, a, a], [b, b, a], [a, b, a]],
            ),
            (
                [0.0, 0.0, -1.0],
                [[a, b, b], [b, b, b], [b, a, b], [a, a, b]],
            ),
            (
                [1.0, 0.0, 0.0],
                [[a, b, a], [a, b, b], [a, a, b], [a, a, a]],
            ),
            (
                [-1.0, 0.0, 0.0],
                [[b, a, a], [b, a, b], [b, b, b], [b, b, a]],
            ),
        ];
        for (n, vs) in faces {
            g.glx.begin(Shape::Quads);
            g.glx.normal3f(n[0], n[1], n[2]);
            for v in vs {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();
        }
        g.glx.cull_face(false);
    }

    /// `unit_spheremonics`: walk the sphere and emit a quad per cell, keeping
    /// the bounding box as it goes so the object can be scaled to fit.
    ///
    /// Upstream sets the material per vertex, inside the block. That is legal
    /// in real GL and the recorder keeps one material per block, so the colour
    /// goes through `GL_COLOR_MATERIAL` instead: the lighting is the same and
    /// the whole surface stays one batch.
    fn unit_spheremonics(&mut self, g: &mut Gl, wire: i32) -> usize {
        let mut polys = 0;
        let mut lo = [0.0f32; 3];
        let mut hi = [0.0f32; 3];

        let res = if wire == 2 {
            self.resolution / 2
        } else {
            self.resolution
        };
        let du = std::f64::consts::PI * 2.0 / res as f64; // Theta
        let dv = std::f64::consts::PI / res as f64; // Phi

        if wire != 0 {
            g.glx.color3f(1.0, 1.0, 1.0);
        }
        g.glx.begin(if wire != 0 {
            Shape::LineLoop
        } else {
            Shape::Quads
        });

        let n = self.colors.len().max(1);
        for i in 0..res {
            let u = i as f64 * du;
            for j in 0..res {
                let v = j as f64 * dv;
                // Each corner takes the colour of the column it is on, so the
                // bands run around the object rather than up it.
                let corners = [
                    (u, v, i as usize),
                    (u + du, v, (i as usize + 1) % res as usize),
                    (u + du, v + dv, (i as usize + 1) % res as usize),
                    (u, v + dv, i as usize),
                ];
                for (cu, cv, ci) in corners {
                    let q = sphere_eval(cu, cv, &self.m);
                    let normal = calc_normal(
                        q,
                        sphere_eval(cu + du / 10.0, cv, &self.m),
                        sphere_eval(cu, cv + dv / 10.0, &self.m),
                    );
                    g.glx.normal3f(normal[0], normal[1], normal[2]);
                    if wire == 0 {
                        let c = &self.colors[ci % n];
                        g.glx.color3f(
                            c.red as f32 / 65535.0,
                            c.green as f32 / 65535.0,
                            c.blue as f32 / 65535.0,
                        );
                    }
                    g.glx.vertex3f(q[0], q[1], q[2]);
                    for k in 0..3 {
                        lo[k] = lo[k].min(q[k]);
                        hi[k] = hi[k].max(q[k]);
                    }
                }
                polys += 1;
            }
        }
        g.glx.end();

        let w = hi[0] - lo[0];
        let h = hi[1] - lo[1];
        let d = hi[2] - lo[2];
        let scale = w.max(h).max(d);
        self.scale = 1.0 / scale;

        if wire < 2 && (self.do_bbox || self.do_grid) {
            let s = scale * 1.5;
            g.glx.push_matrix();
            g.glx.scale(s, s, s);
            self.draw_bounding_box(g);
            g.glx.pop_matrix();
        }
        polys
    }

    /// `init_colors`. Half the colourmap plus a half, so nothing is dark.
    fn init_colors(&mut self) {
        self.colors = make_smooth_colormap(self.resolution as usize);
        for c in &mut self.colors {
            c.red = c.red / 2 + 32767;
            c.green = c.green / 2 + 32767;
            c.blue = c.blue / 2 + 32767;
        }
    }

    /// `tweak_parameters`. Move exactly one of the eight, in the direction it
    /// was already going, and turn it round at either end.
    fn tweak_parameters(&mut self) {
        if self.static_parms {
            return;
        }
        let n = (random() % 8) as usize;
        self.m[n] += self.dm[n];
        if self.m[n] <= 0 {
            self.m[n] = 0;
            self.dm[n] = -self.dm[n];
        } else if self.m[n] >= M_MAX {
            self.m[n] = M_MAX;
            self.dm[n] = -self.dm[n];
        }
    }

    /// `generate_spheremonics`. Pick the next shape and compile it, along with
    /// the slightly larger wireframe copy the mesher draws over it.
    fn generate(&mut self, g: &mut Gl) {
        self.tweak_parameters();
        if !self.done_once || random().is_multiple_of(20) {
            self.init_colors();
        }

        let wire = i32::from(self.wireframe);
        g.glx.new_list(self.solid);
        self.polys = self.unit_spheremonics(g, wire);
        g.glx.end_list();

        g.glx.new_list(self.mesh);
        g.glx.push_matrix();
        g.glx.scale(1.05, 1.05, 1.05);
        self.unit_spheremonics(g, 2);
        g.glx.pop_matrix();
        g.glx.end_list();

        if !self.done_once {
            g.glx.new_list(self.grid);
            if self.do_grid {
                g.glx.push_matrix();
                g.glx.color3f(1.0, 0.0, 0.0);
                g.glx.begin(Shape::Lines);
                g.glx.vertex3f(0.0, -0.66, 0.0);
                g.glx.vertex3f(0.0, 0.66, 0.0);
                g.glx.end();
                Self::draw_circle(g, true);
                g.glx.rotate(90.0, 1.0, 0.0, 0.0);
                Self::draw_circle(g, true);
                g.glx.rotate(90.0, 0.0, 1.0, 0.0);
                Self::draw_circle(g, true);
                g.glx.pop_matrix();
            }
            g.glx.end_list();
        }
        self.done_once = true;
    }

    /// `do_tracer`. Now and then, sweep a ring down the object like a contour
    /// line, or turn the wireframe overlay on for a while.
    fn do_tracer(&mut self, g: &mut Gl) {
        if self.tracer == -1
            && self.mesher == -1
            && random().is_multiple_of(self.duration as u32 * 4)
        {
            if random() & 1 != 0 {
                self.tracer = if random() & 1 != 0 { 0 } else { 180 };
            } else {
                let third = (self.duration / 3 + 1) as u32;
                self.mesher = ((random() % third) + (random() % third)) as i32;
            }
        }

        if self.tracer < 0 {
            return;
        }

        let d = (90 - self.tracer) as f32;
        let th = d * (std::f32::consts::PI / 180.0);
        let (x, y) = (th.cos(), th.sin());
        let mut s = 1.5 / self.scale;

        if s > 0.001 {
            g.glx.lighting(false);
            g.glx.push_matrix();
            g.glx.rotate(90.0, 1.0, 0.0, 0.0);
            g.glx.translate(0.0, 0.0, y * s / 2.0);
            s *= x;
            g.glx.scale(s, s, s);
            g.glx.color3f(0.6, 0.5, 1.0);
            Self::draw_circle(g, false);
            g.glx.pop_matrix();
            if !self.wireframe {
                g.glx.lighting(true);
            }
        }

        self.tracer += 5;
        if self.tracer == 180 || self.tracer == 360 {
            self.tracer = -1;
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.string("spin").to_string();
    let spinx = spin.contains(['x', 'X']);
    let spiny = spin.contains(['y', 'Y']);
    let spinz = spin.contains(['z', 'Z']);
    let do_wander = g.res.bool("wander");

    let parms = g.res.string("parameters").to_string();
    let digits: Vec<i32> = parms
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c as i32 - '0' as i32)
        .collect();
    let static_parms = digits.len() == 8;
    let mut m = [0i32; 8];
    if static_parms {
        m.copy_from_slice(&digits);
    }

    let mut this = Spheremonics {
        rot: Rotator::new(
            if spinx { 1.0 } else { 0.0 },
            if spinz { 1.0 } else { 0.0 },
            if spiny { 1.0 } else { 0.0 },
            1.0,
            if do_wander { 0.03 } else { 0.0 },
            spinx && spiny && spinz,
        ),
        trackball: Trackball::new(),
        font: Some(TexFont::load(&mut g.glx, "sans-serif 18")),
        m,
        dm: [1; 8], // going up!
        static_parms,
        resolution: g.res.int("resolution").max(2),
        colors: Vec::new(),
        scale: 1.0,
        solid: 0,
        mesh: 0,
        grid: 0,
        polys: 0,
        tracer: -1,
        mesher: -1,
        change_tick: 0,
        fade: 0.0,
        done_once: false,
        wireframe: g.res.bool("wireframe"),
        do_bbox: g.res.bool("bbox"),
        do_grid: g.res.bool("grid"),
        smooth: g.res.bool("smooth"),
        duration: g.res.int("duration").max(1),
    };

    this.solid = g.glx.gen_lists(1);
    this.mesh = g.glx.gen_lists(1);
    this.grid = g.glx.gen_lists(1);

    // Generate a few more times so we don't always start off with a sphere.
    for _ in 0..5 {
        this.tweak_parameters();
    }
    this.generate(g);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Spheremonics {
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
            self.change_tick = self.duration;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        // The equations sometimes generate surfaces that are inside out, and
        // working out which is too hard, so nothing is culled and the lighting
        // is two-sided.
        g.glx.cull_face(false);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 5.0, 5.0, 10.0, 1.0);
        }
        g.glx.color_material(true);
        g.glx.blend(if self.smooth {
            Blend::Alpha
        } else {
            Blend::Off
        });

        g.glx.push_matrix();
        g.glx.scale(1.1, 1.1, 1.1);

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 6.0,
            (z as f32 - 0.5) * 8.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(7.0, 7.0, 7.0);

        if self.do_grid {
            g.glx.lighting(false);
            g.glx.push_matrix();
            g.glx.scale(1.5, 1.5, 1.5);
            g.glx.call_list(self.grid);
            g.glx.pop_matrix();
            if !self.wireframe {
                g.glx.lighting(true);
            }
        }

        g.glx.scale(self.scale, self.scale, self.scale);
        g.glx.push_matrix();

        // The change is hidden by shrinking the object away and growing the
        // new one back.
        let fade_speed = 0.15;
        let s = if self.fade == 0.0 {
            1.0
        } else if self.fade > 0.0 {
            let s = self.fade;
            self.fade -= fade_speed;
            self.change_tick = 0;
            if self.fade <= 0.0 {
                self.fade = -1.0;
                self.generate(g);
            }
            s
        } else {
            let s = 1.0 + self.fade;
            self.fade += fade_speed;
            self.change_tick = 0;
            if self.fade >= 0.0 {
                self.fade = 0.0;
            }
            s
        };

        g.glx.scale(s, s, s);
        g.glx.call_list(self.solid);
        g.glx.pop_matrix();

        if self.mesher >= 0 {
            g.glx.lighting(false);
            g.glx.call_list(self.mesh);
            self.mesher -= 1;
            if !self.wireframe {
                g.glx.lighting(true);
            }
        }

        if self.fade == 0.0 {
            self.do_tracer(g);
        }

        if down && let Some(font) = &self.font {
            let all_small = self.m.iter().all(|&v| v < 10);
            let label: String = if all_small {
                self.m.iter().map(|v| v.to_string()).collect()
            } else {
                self.m
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let (w, h) = (g.width(), g.height());
            font.print_label(&mut g.glx, &label, w, h, 1, [1.0, 1.0, 0.0, 1.0]);
        }

        if !self.static_parms {
            self.change_tick += 1;
            if self.change_tick >= self.duration && !down {
                self.change_tick = 0;
                self.fade = 1.0;
                // Turn off the mesh when switching objects.
                self.mesher = -1;
            }
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*labelfont:    sans-serif 18",
    "*suppressRotationAnimation: True",
    "*duration:     200",
    "*spin:         XYZ",
    "*wander:       False",
    "*resolution:   64",
    "*bbox:         False",
    "*grid:         True",
    "*smooth:       True",
    "*parameters:   (default)",
];

const SPINS: &[SelectItem] = &[
    SelectItem {
        value: "XYZ",
        label: "Rotate around all three axes",
    },
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
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("duration", "Duration", 5.0, 1000.0, 1.0, 0, "200"),
    Opt::slider("resolution", "Resolution", 5.0, 100.0, 1.0, 0, "64"),
    Opt::select("spin", "Rotation", SPINS, "XYZ"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("smooth", "Smoothed lines", "true"),
    Opt::boolean("grid", "Draw grid", "true"),
    Opt::boolean("bbox", "Draw bounding box", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "spheremonics",
    label: "Spheremonics",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Paul Bourke and Jamie Zawinski",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=KrNVwyWi0io"),
        blurb: "These closed objects are commonly called spherical harmonics, \
                although they are only remotely related to the mathematical \
                definition found in the solution to certain wave functions.",
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
    fn all_the_parameters_at_zero_is_a_sphere() {
        // Every term is something raised to the zeroth power, which is one
        // even where the something is a sine of zero, so the radius comes out
        // to four everywhere.
        let m = [0; 8];
        for (theta, phi) in [(0.3, 0.7), (2.0, 1.5), (5.0, 3.0)] {
            let p = sphere_eval(theta, phi, &m);
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((r - 4.0).abs() < 1e-5, "radius {r} at {theta},{phi}");
        }
    }

    #[test]
    fn only_one_parameter_moves_at_a_time_and_it_bounces() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut s = Spheremonics {
            m: [2; 8],
            dm: [1; 8],
            ..bare()
        };
        let mut moved_more_than_one = 0;
        for _ in 0..400 {
            let before = s.m;
            s.tweak_parameters();
            let changed = (0..8).filter(|&i| s.m[i] != before[i]).count();
            if changed > 1 {
                moved_more_than_one += 1;
            }
            for v in s.m {
                assert!((0..=M_MAX).contains(&v), "a parameter reached {v}");
            }
        }
        assert_eq!(moved_more_than_one, 0);
        // And the saver itself keeps drawing.
        r.step();
        assert!(!r.frame().batches.is_empty());
    }

    /// A saver with nothing in it, for exercising the parameter walk alone.
    fn bare() -> Spheremonics {
        Spheremonics {
            rot: Rotator::new(0.0, 0.0, 0.0, 1.0, 0.0, false),
            trackball: Trackball::new(),
            font: None,
            m: [0; 8],
            dm: [1; 8],
            static_parms: false,
            resolution: 8,
            colors: Vec::new(),
            scale: 1.0,
            solid: 0,
            mesh: 0,
            grid: 0,
            polys: 0,
            tracer: -1,
            mesher: -1,
            change_tick: 0,
            fade: 0.0,
            done_once: false,
            wireframe: false,
            do_bbox: false,
            do_grid: true,
            smooth: true,
            duration: 200,
        }
    }

    #[test]
    fn the_surface_is_one_quad_per_cell_of_the_grid() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "resolution=16&grid=false",
            20260811,
        ));
        r.step();
        let f = r.frame();
        // Sixteen by sixteen quads, cut into two triangles each, and the whole
        // surface is one batch because the colour rides on the vertices.
        let tris: usize = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
            .map(|b| b.count)
            .sum();
        assert_eq!(tris, 16 * 16 * 6, "got {tris} vertices");
    }

    #[test]
    fn a_spiky_object_is_drawn_no_bigger_than_a_round_one() {
        // The scale is one over the longest side of the bounding box, so
        // whatever the parameters wander into, the object keeps roughly the
        // same size on screen rather than filling it or vanishing.
        let mut r = start(StartArgs::new(
            640,
            480,
            "resolution=12&grid=false&duration=30",
            20260811,
        ));
        let (mut hi, mut settled) = (0.0f32, 0);
        for _ in 0..300 {
            r.step();
            let f = r.frame();
            let mut e = 0.0f32;
            for b in &f.batches {
                for v in &f.vertices[b.first..b.first + b.count] {
                    let p = b.mvp.transform(v.pos);
                    e = e.max(p[0].abs()).max(p[1].abs());
                }
            }
            hi = hi.max(e);
            // The shrink-and-grow between shapes takes it to nothing and back,
            // which is the point of it; the rest of the time it is at size.
            if e > 0.3 {
                settled += 1;
            }
        }
        assert!(hi > 0.3 && hi < 1.5, "the largest it ever got was {hi}");
        assert!(settled > 150, "only {settled} of 300 frames were at size");
    }

    #[test]
    fn fixed_parameters_stop_it_changing_shape() {
        // The knob is free text, so it is not on the panel; it is reachable
        // only by editing the defaults, and all it does is skip the walk.
        let mut s = Spheremonics {
            static_parms: true,
            m: [1, 3, 1, 2, 2, 4, 3, 0],
            ..bare()
        };
        for _ in 0..100 {
            s.tweak_parameters();
        }
        assert_eq!(s.m, [1, 3, 1, 2, 2, 4, 3, 0]);
    }
}
