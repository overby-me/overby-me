//! Port of `hacks/glx/hypertorus.c`.
//!
//! ```text
//! hypertorus --- Shows a hypertorus that rotates in 4d
//!
//! Copyright (c) 2003-2026 Carsten Steger <carsten@mirsanmir.org>.
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
//! This program is inspired by Thomas Banchoff's book "Beyond the
//! Third Dimension: Geometry, Computer Graphics, and Higher
//! Dimensions", Scientific American Library, 1990.
//! ```
//!
//! The Clifford torus, turning in four dimensions.
//!
//! The surface itself is trivial: a point on it is `(cos u, sin u, cos v,
//! sin v)`, which lies on the unit hypersphere for every `u` and `v`. All the
//! work is in getting it onto a screen. It is turned by a 4x4 rotation built
//! from the six independent planes a four-dimensional object can turn in, each
//! with its own speed, and then divided down by its fourth coordinate to land
//! in three dimensions before the usual projection to two.
//!
//! Dividing by the fourth coordinate is what makes it worth watching. The
//! parts of the torus that are further away in the fourth dimension come out
//! smaller, so a shape that is rigid in 4D appears to swell, turn inside out
//! and pass through itself. Choosing the orthographic 4D projection instead
//! drops that coordinate rather than dividing by it, and the torus collapses
//! into a doubly covered cylinder at some angles.
//!
//! The normals are not the surface's own; they are the cross product of the
//! two projected tangents, differentiated through the projection with the
//! quotient rule. Getting that right is what keeps the shading correct while
//! the object is turning itself inside out.
//!
//! In two-sided colouring the inside is drawn green and the outside red, which
//! is the plainest way to see the inversion happen.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
};

const NUMU: usize = 64;
const NUMV: usize = 64;

/// Where the torus sits in 4D and in 3D. The fourth-dimension offset is what
/// keeps the divisor away from zero under the perspective projection.
const OFFSET4D: [f32; 4] = [0.0, 0.0, 0.0, 2.0];
const OFFSET3D: [f32; 4] = [0.0, 0.0, -2.0, 0.0];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Display {
    Wireframe,
    Surface,
    Transparent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Appearance {
    Solid,
    Bands,
    Spirals,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Colors {
    OneSided,
    TwoSided,
    ColorWheel,
}

struct Hypertorus {
    /// The six rotation angles, one per plane a 4D object can turn in.
    angles: [f32; 6],
    speeds: [f32; 6],
    speed_scale: f32,
    /// The three angles that drive the changing colours.
    rho: f32,
    sigma: f32,
    tau: f32,

    trackball: Trackball,
    aspect: f32,

    display: Display,
    appearance: Appearance,
    num_spirals: usize,
    colors: Colors,
    change_colors: bool,
    perspective_3d: bool,
    perspective_4d: bool,
}

/// The six plane rotations, each of which mixes two of the four coordinates.
/// Upstream writes them out one function at a time; the axis pairs and the
/// sign of the off-diagonal term are the whole of the difference.
fn rotate_plane(m: &mut [[f32; 4]; 4], a: usize, b: usize, phi: f32, flip: bool) {
    let phi = phi * std::f32::consts::PI / 180.0;
    let (c, s) = (phi.cos(), phi.sin());
    for row in m.iter_mut() {
        let (u, v) = (row[a], row[b]);
        if flip {
            row[a] = c * u - s * v;
            row[b] = s * u + c * v;
        } else {
            row[a] = c * u + s * v;
            row[b] = -s * u + c * v;
        }
    }
}

/// `rotateall`. The order matters: this is the one upstream settled on.
fn rotateall(a: [f32; 6]) -> [[f32; 4]; 4] {
    let mut m = [[0.0f32; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    rotate_plane(&mut m, 1, 2, a[0], false); // wx
    rotate_plane(&mut m, 0, 2, a[1], true); // wy
    rotate_plane(&mut m, 0, 1, a[2], false); // wz
    rotate_plane(&mut m, 1, 3, a[4], true); // xz
    rotate_plane(&mut m, 2, 3, a[3], false); // xy
    rotate_plane(&mut m, 0, 3, a[5], true); // yz
    m
}

/// `rotateall3d`, for the colour rotation.
fn rotateall3d(al: f32, be: f32, de: f32) -> [[f32; 3]; 3] {
    let mut m = [[0.0f32; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    let mut turn = |a: usize, b: usize, phi: f32, flip: bool| {
        let phi = phi * std::f32::consts::PI / 180.0;
        let (c, s) = (phi.cos(), phi.sin());
        for row in m.iter_mut() {
            let (u, v) = (row[a], row[b]);
            if flip {
                row[a] = c * u - s * v;
                row[b] = s * u + c * v;
            } else {
                row[a] = c * u + s * v;
                row[b] = -s * u + c * v;
            }
        }
    };
    turn(1, 2, al, false);
    turn(0, 2, be, true);
    turn(0, 1, de, false);
    m
}

impl Hypertorus {
    /// `color`. A fully saturated wheel by angle, or, when the colours are
    /// changing, that wheel run through a rotating basis.
    fn color(&self, angle: f32, color_mat: &[[f32; 3]; 3]) -> [f32; 4] {
        let mut col = [0.0f32; 4];
        if !self.change_colors {
            if self.colors != Colors::ColorWheel {
                return col;
            }
            let two_pi = 2.0 * std::f32::consts::PI;
            let angle = if angle >= 0.0 {
                angle % two_pi
            } else {
                angle % -two_pi
            };
            let sixth = std::f32::consts::PI / 3.0;
            let mut s = (angle / sixth).floor() as i32;
            let t = angle / sixth - s as f32;
            if s >= 6 {
                s = 0;
            }
            let rgb = match s {
                0 => [1.0, t, 0.0],
                1 => [1.0 - t, 1.0, 0.0],
                2 => [0.0, 1.0, t],
                3 => [0.0, 1.0 - t, 1.0],
                4 => [t, 0.0, 1.0],
                _ => [1.0, 0.0, 1.0 - t],
            };
            col[..3].copy_from_slice(&rgb);
        } else {
            if self.colors != Colors::ColorWheel {
                for k in 0..3 {
                    col[k] = color_mat[k][2];
                }
            } else {
                let (ca, sa) = (angle.cos(), angle.sin());
                for k in 0..3 {
                    col[k] = ca * color_mat[k][0] + sa * color_mat[k][1];
                }
            }
            let m = 0.5 / col[0].abs().max(col[1].abs()).max(col[2].abs());
            for c in col.iter_mut().take(3) {
                *c = m * *c + 0.5;
            }
        }
        col[3] = if self.display == Display::Transparent {
            0.7
        } else {
            1.0
        };
        col
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let display = match g.res.string("mode") {
        "wireframe" => Display::Wireframe,
        "transparent" => Display::Transparent,
        _ => Display::Surface,
    };
    let appear = g.res.string("appearance").to_string();
    let (appearance, num_spirals) = match appear.as_str() {
        "solid" => (Appearance::Solid, 0),
        "spirals-1" => (Appearance::Spirals, 1),
        "spirals-2" => (Appearance::Spirals, 2),
        "spirals-4" => (Appearance::Spirals, 4),
        "spirals-8" => (Appearance::Spirals, 8),
        "spirals-16" => (Appearance::Spirals, 16),
        _ => (Appearance::Bands, 0),
    };
    let colors = match g.res.string("colors") {
        "onesided" => Colors::OneSided,
        "twosided" => Colors::TwoSided,
        _ => Colors::ColorWheel,
    };

    let mut this = Hypertorus {
        angles: [0.0; 6],
        speeds: [
            g.res.float("speedwx") as f32,
            g.res.float("speedwy") as f32,
            g.res.float("speedwz") as f32,
            g.res.float("speedxy") as f32,
            g.res.float("speedxz") as f32,
            g.res.float("speedyz") as f32,
        ],
        speed_scale: 0.9 + frand(0.3) as f32,
        rho: frand(360.0) as f32,
        sigma: frand(360.0) as f32,
        tau: frand(360.0) as f32,
        trackball: Trackball::new(),
        aspect: 1.0,
        display,
        appearance,
        num_spirals,
        colors,
        change_colors: g.res.bool("changeColors"),
        perspective_3d: g.res.string("projection3d") != "orthographic",
        perspective_4d: g.res.string("projection4d") != "orthographic",
    };

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Hypertorus {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        if !self.trackball.button_down() {
            for (a, s) in self.angles.iter_mut().zip(self.speeds) {
                *a += s * self.speed_scale;
                if *a >= 360.0 {
                    *a -= 360.0;
                }
            }
            if self.change_colors {
                // Upstream's DRHO, DSIGMA and DTAU.
                self.rho = (self.rho + 1.1) % 360.0;
                self.sigma = (self.sigma + 1.3) % 360.0;
                self.tau = (self.tau + 1.5) % 360.0;
            }
        }

        g.glx.clear();
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        if self.perspective_3d {
            g.glx.perspective(60.0, self.aspect, 0.1, 10.0);
        } else if self.aspect >= 1.0 {
            g.glx.ortho(-self.aspect, self.aspect, -1.0, 1.0, 0.1, 10.0);
        } else {
            g.glx
                .ortho(-1.0, 1.0, -1.0 / self.aspect, 1.0 / self.aspect, 0.1, 10.0);
        }
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        // Upstream's fourth dimension is turned by two trackballs at once;
        // this one drives the ordinary 3D view instead.
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let wire = self.display == Display::Wireframe;
        g.glx.depth_test(self.display != Display::Transparent);
        g.glx.cull_face(false);
        g.glx.lighting(!wire);
        if !wire {
            g.glx.light_enable(0, true);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
            g.glx.light_model_ambient([0.2, 0.2, 0.2, 1.0]);
            g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
            g.glx.material_shininess(50.0);
        }
        g.glx.blend(if self.display == Display::Transparent {
            Blend::Add
        } else {
            Blend::Off
        });

        let transparent = self.display == Display::Transparent;
        let alpha = if transparent { 0.7 } else { 1.0 };
        let color_mat = rotateall3d(self.rho, self.sigma, self.tau);

        // Only the colour wheel needs a colour per vertex; the other two set
        // one material for the whole surface, which upstream does too.
        let wheel = self.colors == Colors::ColorWheel;
        g.glx.color_material(wheel || wire);
        if !wheel {
            let dyn_col = self.color(0.0, &color_mat);
            match self.colors {
                Colors::OneSided => {
                    let c = if self.change_colors {
                        dyn_col
                    } else {
                        [0.9, 0.4, 0.3, alpha]
                    };
                    g.glx.color4f(c[0], c[1], c[2], c[3]);
                    g.glx.material_ambient_diffuse(c);
                }
                _ => {
                    let (front, back) = if self.change_colors {
                        (
                            dyn_col,
                            [
                                1.0 - dyn_col[0],
                                1.0 - dyn_col[1],
                                1.0 - dyn_col[2],
                                dyn_col[3],
                            ],
                        )
                    } else {
                        ([1.0, 0.0, 0.0, alpha], [0.0, 1.0, 0.0, alpha])
                    };
                    g.glx.color4f(front[0], front[1], front[2], front[3]);
                    g.glx.material_ambient_diffuse(front);
                    g.glx.material_back_ambient_diffuse(back);
                }
            }
        }

        let mat = rotateall(self.angles);
        let skew = self.num_spirals.max(1);
        let (umin, umax) = (0.0f32, 2.0 * std::f32::consts::PI);
        let (vmin, vmax) = (0.0f32, 2.0 * std::f32::consts::PI);
        let (ur, vr) = (umax - umin, vmax - vmin);
        let banded = self.appearance == Appearance::Bands || self.appearance == Appearance::Spirals;

        // The wireframe is upstream's quad strip under a polygon mode of
        // GL_LINE, which draws all four edges of every quad. There is no
        // polygon mode here, so the edges are emitted as lines directly.
        let mut strip: Vec<([f32; 3], [f32; 3], [f32; 4])> = Vec::with_capacity(2 * (NUMV + 1));

        for i in 0..NUMU {
            if banded && (i & 3) >= 2 {
                continue;
            }
            strip.clear();

            for j in 0..=NUMV {
                for k in 0..=1 {
                    let l = i + k;
                    let mut u = ur * l as f32 / NUMU as f32 + umin;
                    let v = vr * j as f32 / NUMV as f32 + vmin;
                    let col = if self.appearance == Appearance::Spirals {
                        u += 4.0 * skew as f32 / NUMV as f32 * v;
                        let b = ((i / 4) & (skew - 1)) * (NUMU / (4 * skew));
                        self.color(ur * 4.0 * b as f32 / NUMU as f32 + umin, &color_mat)
                    } else {
                        self.color(u, &color_mat)
                    };

                    let (cu, su) = (u.cos(), u.sin());
                    let (cv, sv) = (v.cos(), v.sin());
                    // The Clifford torus, and its two tangents.
                    let xx = [cu, su, cv, sv];
                    let xxu = [-su, cu, 0.0, 0.0];
                    let xxv = [0.0, 0.0, -sv, cv];

                    let apply = |src: [f32; 4]| {
                        let mut out = [0.0f32; 4];
                        for (l, o) in out.iter_mut().enumerate() {
                            *o = (0..4).map(|m| mat[l][m] * src[m]).sum();
                        }
                        out
                    };
                    let x = apply(xx);
                    let xu = apply(xxu);
                    let xv = apply(xxv);

                    let (mut p, mut pu, mut pv) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
                    if !self.perspective_4d {
                        for l in 0..3 {
                            p[l] = (x[l] + OFFSET4D[l]) / 1.5 + OFFSET3D[l];
                            pu[l] = xu[l];
                            pv[l] = xv[l];
                        }
                    } else {
                        // Divide by the fourth coordinate, and differentiate
                        // the division so the tangents stay tangents.
                        let s = x[3] + OFFSET4D[3];
                        let t = s * s;
                        for l in 0..3 {
                            let r = x[l] + OFFSET4D[l];
                            p[l] = r / s + OFFSET3D[l];
                            pu[l] = (xu[l] * s - r * xu[3]) / t;
                            pv[l] = (xv[l] * s - r * xv[3]) / t;
                        }
                    }

                    let mut n = [
                        pu[1] * pv[2] - pu[2] * pv[1],
                        pu[2] * pv[0] - pu[0] * pv[2],
                        pu[0] * pv[1] - pu[1] * pv[0],
                    ];
                    let t = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    for c in &mut n {
                        *c /= t;
                    }
                    strip.push((p, n, col));
                }
            }

            if !wire {
                g.glx.begin(Shape::TriangleStrip);
                for (p, n, col) in &strip {
                    if wheel {
                        g.glx.color4f(col[0], col[1], col[2], col[3]);
                    }
                    g.glx.normal3f(n[0], n[1], n[2]);
                    g.glx.vertex3f(p[0], p[1], p[2]);
                }
                g.glx.end();
                continue;
            }

            g.glx.begin(Shape::Lines);
            for q in strip.chunks_exact(2).collect::<Vec<_>>().windows(2) {
                // The four corners of one quad of the strip, in order round
                // it, so every edge is drawn once from each of its two ends.
                let corners = [q[0][0], q[0][1], q[1][1], q[1][0]];
                for e in 0..4 {
                    for c in [corners[e], corners[(e + 1) % 4]] {
                        if wheel {
                            g.glx.color4f(c.2[0], c.2[1], c.2[2], c.2[3]);
                        }
                        g.glx.vertex3f(c.0[0], c.0[1], c.0[2]);
                    }
                }
            }
            g.glx.end();
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        25000",
    "*showFPS:      False",
    "*mode:         surface",
    "*appearance:   bands",
    "*colors:       colorwheel",
    "*changeColors: False",
    "*projection3d: perspective",
    "*projection4d: perspective",
    "*speedwx:      1.1",
    "*speedwy:      1.3",
    "*speedwz:      1.5",
    "*speedxy:      1.7",
    "*speedxz:      1.9",
    "*speedyz:      2.1",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "surface",
        label: "Solid",
    },
    SelectItem {
        value: "wireframe",
        label: "Wireframe",
    },
    SelectItem {
        value: "transparent",
        label: "Transparent",
    },
];

const APPEARANCES: &[SelectItem] = &[
    SelectItem {
        value: "bands",
        label: "Transparent bands",
    },
    SelectItem {
        value: "solid",
        label: "Solid object",
    },
    SelectItem {
        value: "spirals-1",
        label: "1 transparent spiral",
    },
    SelectItem {
        value: "spirals-2",
        label: "2 transparent spirals",
    },
    SelectItem {
        value: "spirals-4",
        label: "4 transparent spirals",
    },
    SelectItem {
        value: "spirals-8",
        label: "8 transparent spirals",
    },
    SelectItem {
        value: "spirals-16",
        label: "16 transparent spirals",
    },
];

const COLOR_MODES: &[SelectItem] = &[
    SelectItem {
        value: "colorwheel",
        label: "Color wheel",
    },
    SelectItem {
        value: "onesided",
        label: "One-sided",
    },
    SelectItem {
        value: "twosided",
        label: "Two-sided",
    },
];

const PROJ_3D: &[SelectItem] = &[
    SelectItem {
        value: "perspective",
        label: "Perspective 3D",
    },
    SelectItem {
        value: "orthographic",
        label: "Orthographic 3D",
    },
];

const PROJ_4D: &[SelectItem] = &[
    SelectItem {
        value: "perspective",
        label: "Perspective 4D",
    },
    SelectItem {
        value: "orthographic",
        label: "Orthographic 4D",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "25000").inverted(),
    Opt::select("mode", "Display mode", MODES, "surface"),
    Opt::select("appearance", "Appearance", APPEARANCES, "bands"),
    Opt::select("colors", "Colors", COLOR_MODES, "colorwheel"),
    Opt::boolean("changeColors", "Change colors", "false"),
    Opt::select("projection3d", "3D projection", PROJ_3D, "perspective"),
    Opt::select("projection4d", "4D projection", PROJ_4D, "perspective"),
    Opt::slider("speedwx", "WX rotation speed", -4.0, 4.0, 0.1, 1, "1.1"),
    Opt::slider("speedwy", "WY rotation speed", -4.0, 4.0, 0.1, 1, "1.3"),
    Opt::slider("speedwz", "WZ rotation speed", -4.0, 4.0, 0.1, 1, "1.5"),
    Opt::slider("speedxy", "XY rotation speed", -4.0, 4.0, 0.1, 1, "1.7"),
    Opt::slider("speedxz", "XZ rotation speed", -4.0, 4.0, 0.1, 1, "1.9"),
    Opt::slider("speedyz", "YZ rotation speed", -4.0, 4.0, 0.1, 1, "2.1"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hypertorus",
    label: "Hypertorus",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Steger",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=KJWe4G4Qa1Q"),
        blurb: "This shows the Clifford torus as it rotates in 4d, projected \
                to 3d and then to the screen.",
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
    fn a_four_dimensional_rotation_keeps_lengths() {
        // Every one of the six plane rotations is a rotation, so the matrix
        // they build is orthogonal and the torus stays on the hypersphere.
        let m = rotateall([13.0, 27.0, 41.0, 59.0, 71.0, 97.0]);
        for i in 0..4 {
            let len: f32 = (0..4).map(|j| m[i][j] * m[i][j]).sum();
            assert!((len - 1.0).abs() < 1e-4, "row {i} has length {len}");
            for k in i + 1..4 {
                let dot: f32 = (0..4).map(|j| m[i][j] * m[k][j]).sum();
                assert!(dot.abs() < 1e-4, "rows {i} and {k} are not square");
            }
        }
    }

    #[test]
    fn the_bands_leave_half_the_surface_out() {
        let strips = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::TriangleStrip)
                .count()
        };
        // Two strips of every four are skipped, which is what makes the
        // banded appearance see-through.
        assert_eq!(strips("appearance=solid"), NUMU);
        assert_eq!(strips("appearance=bands"), NUMU / 2);
    }

    #[test]
    fn the_surface_stays_on_the_unit_hypersphere_before_it_is_projected() {
        // The whole point of the Clifford torus. Check it directly rather
        // than through the projection.
        for (u, v) in [(0.3f32, 1.1f32), (2.0, 5.0), (4.4, 0.2)] {
            let p = [u.cos(), u.sin(), v.cos(), v.sin()];
            let r: f32 = p.iter().map(|c| c * c).sum();
            assert!((r - 2.0).abs() < 1e-5, "radius squared {r}");
        }
    }

    #[test]
    fn two_sided_colouring_paints_the_inside_a_different_colour() {
        let mut r = start(StartArgs::new(640, 480, "colors=twosided", 20260811));
        r.step();
        let b = &r.frame().batches[0];
        assert_eq!(b.material.ambient_diffuse, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(b.material.back_ambient_diffuse, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn the_colour_wheel_runs_all_the_way_round() {
        let mut r = start(StartArgs::new(640, 480, "colors=colorwheel", 20260811));
        r.step();
        let f = r.frame();
        // Fully saturated: every vertex has one channel at zero and one at
        // one, which is what a walk round the wheel looks like.
        let mut reds = 0;
        let mut greens = 0;
        let mut blues = 0;
        for v in &f.vertices {
            if v.color[0] > 0.99 {
                reds += 1;
            }
            if v.color[1] > 0.99 {
                greens += 1;
            }
            if v.color[2] > 0.99 {
                blues += 1;
            }
        }
        assert!(
            reds > 0 && greens > 0 && blues > 0,
            "{reds} {greens} {blues}"
        );
    }

    #[test]
    fn the_projection_makes_a_rigid_object_appear_to_breathe() {
        // Dividing by the fourth coordinate is what makes it worth watching.
        // The overall extent barely moves, because the mesh always contains
        // its own extremes wherever it has been turned to; what changes is the
        // distance between two given points of it, which a rigid motion would
        // leave alone.
        let mut r = start(StartArgs::new(640, 480, "appearance=solid", 20260811));
        let (mut lo, mut hi) = (f32::MAX, 0.0f32);
        for _ in 0..120 {
            r.step();
            let f = r.frame();
            // The mesh is emitted in a fixed order, so these are the same two
            // points of the surface on every frame.
            let (a, b) = (f.vertices[0].pos, f.vertices[64].pos);
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            lo = lo.min(d);
            hi = hi.max(d);
        }
        assert!(hi / lo > 1.2, "the object stayed rigid, {lo} to {hi}");
    }

    #[test]
    fn the_wireframe_draws_every_edge_of_every_quad() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "mode=wireframe&appearance=solid",
            20260811,
        ));
        r.step();
        let f = r.frame();
        let lines: usize = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Lines)
            .map(|b| b.count)
            .sum();
        // Four edges a quad, two vertices an edge, NUMV quads a strip.
        assert_eq!(lines, NUMU * NUMV * 4 * 2, "got {lines} line vertices");
    }
}
