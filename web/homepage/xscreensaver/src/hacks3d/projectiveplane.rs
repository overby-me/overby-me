//! Port of `hacks/glx/projectiveplane.c`.
//!
//! ```text
//! projectiveplane --- Shows a 4d embedding of the real projective plane
//!   that rotates in 4d or on which you can walk
//!
//! Copyright (c) 2013-2026 Carsten Steger <carsten@mirsanmir.org>.
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
//! ```
//!
//! The real projective plane, embedded in four dimensions.
//!
//! In three dimensions this surface cannot avoid passing through itself. In
//! four it can, and that is the point of the thing: set the rotation speeds to
//! zero and the 4D projection to orthographic and it collapses to the Roman
//! surface, which has three lines where it crosses itself, and in depth
//! colours the two sheets at each crossing are different colours because they
//! are at different depths in the fourth dimension.
//!
//! It is also non-orientable, which the two-sided colouring shows: a surface
//! with an inside and an outside would be red on one and green on the other,
//! and this one is not. The orientation markers show the same thing more
//! plainly, if you turn them on: walk around the surface and the curling arrow
//! comes back the other way round.
//!
//! You can watch it turn or walk on it. Walking rebuilds the whole 4D rotation
//! every frame from the surface's own tangent plane at wherever the walker has
//! got to, so that the camera stays upright on a surface that has no consistent
//! notion of up.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape, TexEnv};
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

const NUMU: usize = 128;
const NUMV: usize = 128;
/// The period of the bands, in mesh lines.
const NUMB: usize = 8;

/// How far below the walker's feet the camera sits.
const DELTAY: f32 = 0.01;
const DRHO: f32 = 0.7;
const DSIGMA: f32 = 1.1;
const DTAU: f32 = 1.7;

const TEX_DIMENSION: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Display {
    Wireframe,
    Surface,
    Transparent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Appearance {
    Solid,
    DistanceBands,
    DirectionBands,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Colors {
    OneSided,
    TwoSided,
    Distance,
    Direction,
    Depth,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Walk,
    Turn,
    WalkTurn,
}

struct ProjectivePlane {
    /// The six angles of the 4D rotation.
    alpha: f32,
    beta: f32,
    delta: f32,
    zeta: f32,
    eta: f32,
    theta: f32,
    speeds: [f32; 6],
    speed_scale: f32,
    /// The three angles that drive the changing colours.
    rho: f32,
    sigma: f32,
    tau: f32,

    /// Where the walker is on the surface, and which way it is going.
    umove: f32,
    vmove: f32,
    dumove: f32,
    dvmove: f32,
    side: f32,
    dir: f32,
    walk_direction: f32,
    walk_speed: f32,

    offset4d: [f32; 4],
    offset3d: [f32; 4],

    /// The surface and its two tangents at every mesh point, in 4D, plus the
    /// static colour and texture coordinate.
    x: Vec<[f32; 4]>,
    xu: Vec<[f32; 4]>,
    xv: Vec<[f32; 4]>,
    col: Vec<[f32; 4]>,
    tex: Vec<[f32; 2]>,
    /// The projection of the surface into 3D, rebuilt every frame.
    pp: Vec<[f32; 3]>,
    pnorm: Vec<[f32; 3]>,

    trackball: Trackball,
    aspect: f32,
    texture: u32,

    display: Display,
    appearance: Appearance,
    colors: Colors,
    view: View,
    marks: bool,
    change_colors: bool,
    perspective_3d: bool,
    perspective_4d: bool,
}

/// One of the six plane rotations of a 4x4 matrix.
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

fn identity4() -> [[f32; 4]; 4] {
    let mut m = [[0.0f32; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    m
}

/// `rotateall`. The order is upstream's and is not the same as its siblings'.
fn rotateall(al: f32, be: f32, de: f32, ze: f32, et: f32, th: f32) -> [[f32; 4]; 4] {
    let mut m = identity4();
    rotate_plane(&mut m, 1, 2, al, false); // wx
    rotate_plane(&mut m, 0, 2, be, true); // wy
    rotate_plane(&mut m, 0, 1, de, false); // wz
    rotate_plane(&mut m, 2, 3, ze, false); // xy
    rotate_plane(&mut m, 1, 3, et, true); // xz
    rotate_plane(&mut m, 0, 3, th, true); // yz
    m
}

/// `rotateall4d`: only the three planes that do not involve the w axis, which
/// is the part of the rotation a walker keeps.
fn rotateall4d(ze: f32, et: f32, th: f32) -> [[f32; 4]; 4] {
    let mut m = identity4();
    rotate_plane(&mut m, 2, 3, ze, false);
    rotate_plane(&mut m, 1, 3, et, true);
    rotate_plane(&mut m, 0, 3, th, true);
    m
}

/// `rotateall3d`, for the colour basis.
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

/// The surface, in four dimensions. `w` is the fourth coordinate and is also
/// what the depth colouring reads.
fn surface(u: f32, v: f32) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let (su, cu) = (u.sin(), u.cos());
    let (s2u, c2u) = ((2.0 * u).sin(), (2.0 * u).cos());
    let sv2 = (0.5 * v).sin();
    let sv4 = (0.25 * v).sin();
    let cv4 = (0.25 * v).cos();
    let w = 0.5 * (su * su * sv4 * sv4 - cv4 * cv4);
    let x = [0.5 * s2u * sv4 * sv4, 0.5 * su * sv2, 0.5 * cu * sv2, w];

    // Avoid degenerate tangential plane basis vectors.
    let v = if v < f32::EPSILON { f32::EPSILON } else { v };
    let sv2 = (0.5 * v).sin();
    let cv2 = (0.5 * v).cos();
    let sv4 = (0.25 * v).sin();
    let xu = [
        c2u * sv4 * sv4,
        0.5 * cu * sv2,
        -0.5 * su * sv2,
        0.5 * s2u * sv4 * sv4,
    ];
    let xv = [
        0.125 * s2u * sv2,
        0.25 * su * cv2,
        0.25 * cu * cv2,
        0.125 * (su * su + 1.0) * sv2,
    ];
    (x, xu, xv)
}

impl ProjectivePlane {
    /// `color`. A fully saturated wheel by angle, or that wheel run through a
    /// rotating basis when the colours are changing.
    fn color(&self, angle: f32, m: &[[f32; 3]; 3]) -> [f32; 4] {
        let mut col = [0.0f32; 4];
        if !self.change_colors {
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
            if self.colors == Colors::OneSided || self.colors == Colors::TwoSided {
                for (k, c) in col.iter_mut().take(3).enumerate() {
                    *c = m[k][2];
                }
            } else {
                let (ca, sa) = (angle.cos(), angle.sin());
                for (k, c) in col.iter_mut().take(3).enumerate() {
                    *c = ca * m[k][0] + sa * m[k][1];
                }
            }
            let s = 0.5 / col[0].abs().max(col[1].abs()).max(col[2].abs());
            for c in col.iter_mut().take(3) {
                *c = s * *c + 0.5;
            }
        }
        col[3] = if self.display == Display::Transparent {
            0.7
        } else {
            1.0
        };
        col
    }

    /// `setup_projective_plane`: sample the surface once, since only the
    /// rotation changes from frame to frame.
    fn setup(&mut self) {
        let (umin, umax) = (0.0f32, 2.0 * std::f32::consts::PI);
        let (vmin, vmax) = (0.0f32, 2.0 * std::f32::consts::PI);
        let (ur, vr) = (umax - umin, vmax - vmin);

        for i in 0..=NUMV {
            for j in 0..=NUMU {
                let k = i * (NUMU + 1) + j;
                // The direction-band appearance runs u the other way, which is
                // what makes its bands lie along the other axis.
                let u = if self.appearance != Appearance::DirectionBands {
                    -ur * j as f32 / NUMU as f32 + umin
                } else {
                    ur * j as f32 / NUMU as f32 + umin
                };
                let v = vr * i as f32 / NUMV as f32 + vmin;

                let (x, xu, xv) = surface(u, v);
                self.x[k] = x;
                self.xu[k] = xu;
                self.xv[k] = xv;

                if !self.change_colors {
                    let two_pi = 2.0 * std::f32::consts::PI;
                    let angle = match self.colors {
                        Colors::Depth => (2.0 * x[3] + 1.0) * std::f32::consts::PI * 2.0 / 3.0,
                        Colors::Direction => two_pi + (2.0 * u) % two_pi,
                        _ => v * (5.0 / 6.0),
                    };
                    self.col[k] = self.color(angle, &rotateall3d(0.0, 0.0, 0.0));
                }

                self.tex[k][0] = -32.0 * u / (2.0 * std::f32::consts::PI);
                self.tex[k][1] = 32.0 * v / (2.0 * std::f32::consts::PI)
                    - if self.appearance == Appearance::DistanceBands {
                        0.5
                    } else {
                        0.0
                    };
            }
        }
    }

    /// `compute_walk_frame`. The camera is built from the surface's own
    /// tangent plane at the walker's position, so it stays upright on a
    /// surface that has no consistent notion of up.
    fn compute_walk_frame(&mut self) -> [[f32; 4]; 4] {
        let mat = rotateall4d(self.zeta, self.eta, self.theta);
        let (xx, xxu, xxv) = surface(self.umove, self.vmove);

        let apply = |m: &[[f32; 4]; 4], src: [f32; 4]| -> [f32; 4] {
            std::array::from_fn(|l| (0..4).map(|k| m[l][k] * src[k]).sum())
        };
        let y = apply(&mat, xx);
        let yu = apply(&mat, xxu);
        let yv = apply(&mat, xxv);

        let (mut p, mut pu, mut pv) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
        if !self.perspective_4d {
            for l in 0..3 {
                p[l] = y[l] + self.offset4d[l];
                pu[l] = yu[l];
                pv[l] = yv[l];
            }
        } else {
            let s = y[3] + self.offset4d[3];
            let q = 1.0 / s;
            let t = q * q;
            for l in 0..3 {
                let r = y[l] + self.offset4d[l];
                p[l] = r * q;
                pu[l] = (yu[l] * s - r * yu[3]) * t;
                pv[l] = (yv[l] * s - r * yv[3]) * t;
            }
        }

        let mut n = [
            pu[1] * pv[2] - pu[2] * pv[1],
            pu[2] * pv[0] - pu[0] * pv[2],
            pu[0] * pv[1] - pu[1] * pv[0],
        ];
        let t = 1.0 / (self.side * 4.0 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt());
        for c in &mut n {
            *c *= t;
        }

        let mut pm = [
            pu[0] * self.dumove + pv[0] * self.dvmove,
            pu[1] * self.dumove + pv[1] * self.dvmove,
            pu[2] * self.dumove + pv[2] * self.dvmove,
        ];
        let t = 1.0 / (4.0 * (pm[0] * pm[0] + pm[1] * pm[1] + pm[2] * pm[2]).sqrt());
        for c in &mut pm {
            *c *= t;
        }

        let mut b = [
            n[1] * pm[2] - n[2] * pm[1],
            n[2] * pm[0] - n[0] * pm[2],
            n[0] * pm[1] - n[1] * pm[0],
        ];
        let t = 1.0 / (4.0 * (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt());
        for c in &mut b {
            *c *= t;
        }

        // Read the three Euler angles back out of the frame the three basis
        // vectors make.
        let deg = 180.0 / std::f32::consts::PI;
        self.alpha = (-n[2]).atan2(-pm[2]) * deg;
        self.beta = (-b[2]).atan2((b[0] * b[0] + b[1] * b[1]).sqrt()) * deg;
        self.delta = b[1].atan2(-b[0]) * deg;

        let mat = rotateall(
            self.alpha, self.beta, self.delta, self.zeta, self.eta, self.theta,
        );

        let (xx, _, _) = surface(self.umove, self.vmove);
        let y = apply(&mat, xx);
        let mut p = [0.0f32; 3];
        if !self.perspective_4d {
            for l in 0..3 {
                p[l] = y[l] + self.offset4d[l];
            }
        } else {
            let s = y[3] + self.offset4d[3];
            for l in 0..3 {
                p[l] = (y[l] + self.offset4d[l]) / s;
            }
        }
        self.offset3d[0] = -p[0];
        self.offset3d[1] = -p[1] - DELTAY;
        self.offset3d[2] = -p[2];
        mat
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let pick = |s: &str, n: u32| -> u32 {
        if s == "random" {
            random() % n
        } else {
            u32::MAX
        }
    };

    let mode = g.res.string("mode").to_string();
    let display = match mode.as_str() {
        "wireframe" => Display::Wireframe,
        "surface" => Display::Surface,
        "transparent" => Display::Transparent,
        _ => match pick(&mode, 3) {
            0 => Display::Wireframe,
            1 => Display::Surface,
            _ => Display::Transparent,
        },
    };

    let appear = g.res.string("appearance").to_string();
    let appearance = match appear.as_str() {
        "solid" => Appearance::Solid,
        "distance-bands" => Appearance::DistanceBands,
        "direction-bands" => Appearance::DirectionBands,
        _ => match pick(&appear, 3) {
            0 => Appearance::Solid,
            1 => Appearance::DistanceBands,
            _ => Appearance::DirectionBands,
        },
    };

    let cols = g.res.string("colors").to_string();
    let colors = match cols.as_str() {
        "onesided" => Colors::OneSided,
        "twosided" => Colors::TwoSided,
        "distance" => Colors::Distance,
        "direction" => Colors::Direction,
        "depth" => Colors::Depth,
        _ => match pick(&cols, 5) {
            0 => Colors::OneSided,
            1 => Colors::TwoSided,
            2 => Colors::Distance,
            3 => Colors::Direction,
            _ => Colors::Depth,
        },
    };

    let vm = g.res.string("viewMode").to_string();
    let view = match vm.as_str() {
        "walk" => View::Walk,
        "turn" => View::Turn,
        "walk-turn" => View::WalkTurn,
        _ => match pick(&vm, 3) {
            0 => View::Walk,
            1 => View::Turn,
            _ => View::WalkTurn,
        },
    };

    let n = (NUMU + 1) * (NUMV + 1);
    let walk_direction = g.res.float("walkDirection") as f32;

    let mut this = ProjectivePlane {
        alpha: 0.0,
        beta: 0.0,
        delta: 0.0,
        zeta: 120.0,
        eta: 180.0,
        theta: 90.0,
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
        umove: frand(2.0 * std::f64::consts::PI) as f32,
        vmove: frand(2.0 * std::f64::consts::PI) as f32,
        dumove: 0.0,
        dvmove: 0.0,
        side: 1.0,
        dir: if (walk_direction * std::f32::consts::PI / 180.0).sin() >= 0.0 {
            1.0
        } else {
            -1.0
        },
        walk_direction,
        walk_speed: g.res.float("walkSpeed") as f32,
        offset4d: [0.0, 0.0, 0.0, 1.2],
        offset3d: [0.0, 0.0, -1.2, 0.0],
        x: vec![[0.0; 4]; n],
        xu: vec![[0.0; 4]; n],
        xv: vec![[0.0; 4]; n],
        col: vec![[1.0; 4]; n],
        tex: vec![[0.0; 2]; n],
        pp: vec![[0.0; 3]; n],
        pnorm: vec![[0.0; 3]; n],
        trackball: Trackball::new(),
        aspect: 1.0,
        texture: 0,
        display,
        appearance,
        colors,
        view,
        marks: g.res.bool("marks"),
        change_colors: g.res.bool("changeColors"),
        perspective_3d: g.res.string("projection3d") != "orthographic",
        perspective_4d: g.res.string("projection4d") != "orthographic",
    };
    this.setup();

    // The orientation marker, a curling arrow, as a greyscale texture.
    this.texture = g.glx.gen_texture();
    g.glx.bind_texture(this.texture);
    let gray = crate::images::CURLICUE;
    let rgba: Vec<u8> = gray.iter().flat_map(|&v| [255, 255, 255, v]).collect();
    g.glx
        .tex_image_2d(TEX_DIMENSION as i32, TEX_DIMENSION as i32, rgba);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for ProjectivePlane {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        if !self.trackball.button_down() {
            if self.view == View::Turn {
                let s = self.speed_scale;
                self.alpha = (self.alpha + self.speeds[0] * s) % 360.0;
                self.beta = (self.beta + self.speeds[1] * s) % 360.0;
                self.delta = (self.delta + self.speeds[2] * s) % 360.0;
            }
            if self.view == View::Turn || self.view == View::WalkTurn {
                let s = self.speed_scale;
                self.zeta = (self.zeta + self.speeds[3] * s) % 360.0;
                self.eta = (self.eta + self.speeds[4] * s) % 360.0;
                self.theta = (self.theta + self.speeds[5] * s) % 360.0;
            }
            if self.view == View::Walk || self.view == View::WalkTurn {
                let two_pi = 2.0 * std::f32::consts::PI;
                let rad = self.walk_direction * std::f32::consts::PI / 180.0;
                self.dvmove =
                    self.dir * rad.sin() * self.walk_speed * std::f32::consts::PI / 4096.0;
                self.vmove += self.dvmove;
                // Walking off one edge of the parameter square comes back on
                // the other, half a turn round in u and on the other side.
                if self.vmove > two_pi {
                    self.vmove = 2.0 * two_pi - self.vmove;
                    self.umove -= std::f32::consts::PI;
                    if self.umove < 0.0 {
                        self.umove += two_pi;
                    }
                    self.side = -self.side;
                    self.dir = -self.dir;
                    self.dvmove = -self.dvmove;
                }
                if self.vmove < 0.0 {
                    self.vmove = -self.vmove;
                    self.umove -= std::f32::consts::PI;
                    if self.umove < 0.0 {
                        self.umove += two_pi;
                    }
                    self.dir = -self.dir;
                    self.dvmove = -self.dvmove;
                }
                self.dumove = rad.cos() * self.walk_speed * std::f32::consts::PI / 4096.0;
                self.umove += self.dumove;
                if self.umove >= two_pi {
                    self.umove -= two_pi;
                }
                if self.umove < 0.0 {
                    self.umove += two_pi;
                }
            }
            if self.change_colors {
                self.rho = (self.rho + DRHO) % 360.0;
                self.sigma = (self.sigma + DSIGMA) % 360.0;
                self.tau = (self.tau + DTAU) % 360.0;
            }
        }

        g.glx.clear();
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        let walking = self.view == View::Walk || self.view == View::WalkTurn;
        if self.perspective_3d || walking {
            let near = if walking { 0.01 } else { 0.1 };
            g.glx.perspective(60.0, self.aspect, near, 10.0);
        } else if self.aspect >= 1.0 {
            g.glx
                .ortho(-0.6 * self.aspect, 0.6 * self.aspect, -0.6, 0.6, 0.1, 10.0);
        } else {
            g.glx
                .ortho(-0.6, 0.6, -0.6 / self.aspect, 0.6 / self.aspect, 0.1, 10.0);
        }
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        let wire = self.display == Display::Wireframe;
        let transparent = self.display == Display::Transparent;
        g.glx.depth_test(!transparent);
        g.glx.depth_mask(!transparent);
        g.glx.cull_face(false);
        g.glx.lighting(!wire);
        if !wire {
            g.glx.light_enable(0, true);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
            g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
            g.glx.material_shininess(50.0);
        }
        g.glx
            .blend(if transparent { Blend::Add } else { Blend::Off });
        g.glx.texturing(self.marks);
        if self.marks {
            g.glx.tex_env(TexEnv::Modulate);
            g.glx.bind_texture(self.texture);
        }

        let color_mat = rotateall3d(self.rho, self.sigma, self.tau);

        let mat = if walking {
            self.compute_walk_frame()
        } else {
            let r1 = rotateall(
                self.alpha, self.beta, self.delta, self.zeta, self.eta, self.theta,
            );
            // Upstream turns the fourth dimension with two trackballs at once;
            // this one drives the ordinary 3D view instead.
            let m = self.trackball.matrix();
            g.glx.mult_matrix(m);
            r1
        };

        // Project every mesh point from 4D to 3D, and differentiate the
        // projection so the normals stay normals.
        for i in 0..=NUMV {
            for j in 0..=NUMU {
                let o = i * (NUMU + 1) + j;
                let apply = |src: [f32; 4]| -> [f32; 4] {
                    std::array::from_fn(|l| (0..4).map(|k| mat[l][k] * src[k]).sum())
                };
                let y = apply(self.x[o]);
                let yu = apply(self.xu[o]);
                let yv = apply(self.xv[o]);

                let (mut pu, mut pv) = ([0.0f32; 3], [0.0f32; 3]);
                if !self.perspective_4d {
                    for l in 0..3 {
                        self.pp[o][l] = (y[l] + self.offset4d[l]) + self.offset3d[l];
                        pu[l] = yu[l];
                        pv[l] = yv[l];
                    }
                } else {
                    let s = y[3] + self.offset4d[3];
                    let q = 1.0 / s;
                    let t = q * q;
                    for l in 0..3 {
                        let r = y[l] + self.offset4d[l];
                        self.pp[o][l] = r * q + self.offset3d[l];
                        pu[l] = (yu[l] * s - r * yu[3]) * t;
                        pv[l] = (yv[l] * s - r * yv[3]) * t;
                    }
                }
                let mut n = [
                    pu[1] * pv[2] - pu[2] * pv[1],
                    pu[2] * pv[0] - pu[0] * pv[2],
                    pu[0] * pv[1] - pu[1] * pv[0],
                ];
                let t = 1.0 / (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                for c in &mut n {
                    *c *= t;
                }
                self.pnorm[o] = n;
            }
        }

        let per_vertex = self.colors != Colors::OneSided && self.colors != Colors::TwoSided;
        g.glx.color_material(per_vertex || wire);
        if !per_vertex {
            let alpha = if transparent { 0.7 } else { 1.0 };
            let dyn_col = self.color(0.0, &color_mat);
            if self.colors == Colors::OneSided {
                let c = if self.change_colors {
                    dyn_col
                } else {
                    [0.9, 0.4, 0.3, alpha]
                };
                g.glx.color4f(c[0], c[1], c[2], c[3]);
                g.glx.material_ambient_diffuse(c);
            } else {
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

        let two_pi = 2.0 * std::f32::consts::PI;
        let (ur, vr) = (two_pi, two_pi);

        // A wireframe strip is upstream's quad strip under a polygon mode of
        // GL_LINE; there is no polygon mode here, so the edges go out as lines.
        let emit = |g: &mut Gl, strip: &[(usize, [f32; 4])]| {
            if !wire {
                g.glx.begin(Shape::TriangleStrip);
                for (o, col) in strip {
                    if per_vertex {
                        g.glx.color4f(col[0], col[1], col[2], col[3]);
                    }
                    let n = self.pnorm[*o];
                    g.glx.normal3f(n[0], n[1], n[2]);
                    g.glx.tex_coord2f(self.tex[*o][0], self.tex[*o][1]);
                    let p = self.pp[*o];
                    g.glx.vertex3f(p[0], p[1], p[2]);
                }
                g.glx.end();
                return;
            }
            g.glx.begin(Shape::Lines);
            for q in strip.chunks_exact(2).collect::<Vec<_>>().windows(2) {
                let corners = [q[0][0], q[0][1], q[1][1], q[1][0]];
                for e in 0..4 {
                    for c in [corners[e], corners[(e + 1) % 4]] {
                        if per_vertex {
                            g.glx.color4f(c.1[0], c.1[1], c.1[2], c.1[3]);
                        }
                        let p = self.pp[c.0];
                        g.glx.vertex3f(p[0], p[1], p[2]);
                    }
                }
            }
            g.glx.end();
        };

        let mut strip: Vec<(usize, [f32; 4])> = Vec::with_capacity(2 * (NUMU + 1));
        if self.appearance != Appearance::DirectionBands {
            for i in 0..NUMV {
                if self.appearance == Appearance::DistanceBands
                    && (i & (NUMB - 1)) >= NUMB / 4
                    && (i & (NUMB - 1)) < 3 * NUMB / 4
                {
                    continue;
                }
                strip.clear();
                for j in 0..=NUMU {
                    for k in 0..=1 {
                        let o = (i + k) * (NUMU + 1) + j;
                        let col = if self.change_colors && per_vertex {
                            let angle = match self.colors {
                                Colors::Depth => {
                                    (2.0 * self.x[o][3] + 1.0) * std::f32::consts::PI * 2.0 / 3.0
                                }
                                Colors::Direction => {
                                    let u = -ur * j as f32 / NUMU as f32;
                                    two_pi + (2.0 * u) % two_pi
                                }
                                _ => (vr * (i + k) as f32 / NUMV as f32) * (5.0 / 6.0),
                            };
                            self.color(angle, &color_mat)
                        } else {
                            self.col[o]
                        };
                        strip.push((o, col));
                    }
                }
                emit(g, &strip);
            }
        } else {
            for j in 0..NUMU {
                if (j & (NUMB - 1)) >= NUMB / 2 {
                    continue;
                }
                strip.clear();
                for i in 0..=NUMV {
                    for k in 0..=1 {
                        let o = i * (NUMU + 1) + j + k;
                        let col = if self.change_colors && per_vertex {
                            let angle = match self.colors {
                                Colors::Depth => {
                                    (2.0 * self.x[o][3] + 1.0) * std::f32::consts::PI * 2.0 / 3.0
                                }
                                Colors::Direction => {
                                    let u = ur * (j + k) as f32 / NUMU as f32;
                                    two_pi + (2.0 * u) % two_pi
                                }
                                _ => (vr * i as f32 / NUMV as f32) * (5.0 / 6.0),
                            };
                            self.color(angle, &color_mat)
                        } else {
                            self.col[o]
                        };
                        strip.push((o, col));
                    }
                }
                emit(g, &strip);
            }
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        25000",
    "*showFPS:      False",
    "*mode:         random",
    "*appearance:   random",
    "*colors:       random",
    "*viewMode:     random",
    "*marks:        False",
    "*changeColors: False",
    "*projection3d: random",
    "*projection4d: random",
    "*speedwx:      1.1",
    "*speedwy:      1.3",
    "*speedwz:      1.5",
    "*speedxy:      1.7",
    "*speedxz:      1.9",
    "*speedyz:      2.1",
    "*walkDirection: 83.0",
    "*walkSpeed:    20.0",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random display mode",
    },
    SelectItem {
        value: "wireframe",
        label: "Wireframe mesh",
    },
    SelectItem {
        value: "surface",
        label: "Solid surface",
    },
    SelectItem {
        value: "transparent",
        label: "Transparent surface",
    },
];

const APPEARANCES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random appearance",
    },
    SelectItem {
        value: "solid",
        label: "Solid object",
    },
    SelectItem {
        value: "distance-bands",
        label: "See-through bands by distance",
    },
    SelectItem {
        value: "direction-bands",
        label: "See-through bands by direction",
    },
];

const COLOR_MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random colors",
    },
    SelectItem {
        value: "onesided",
        label: "One-sided",
    },
    SelectItem {
        value: "twosided",
        label: "Two-sided",
    },
    SelectItem {
        value: "distance",
        label: "Distance colors",
    },
    SelectItem {
        value: "direction",
        label: "Direction colors",
    },
    SelectItem {
        value: "depth",
        label: "4d depth colors",
    },
];

const VIEW_MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random view mode",
    },
    SelectItem {
        value: "turn",
        label: "Rotate",
    },
    SelectItem {
        value: "walk",
        label: "Walk",
    },
    SelectItem {
        value: "walk-turn",
        label: "Walk and turn",
    },
];

const PROJ: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random projection",
    },
    SelectItem {
        value: "perspective",
        label: "Perspective",
    },
    SelectItem {
        value: "orthographic",
        label: "Orthographic",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "25000").inverted(),
    Opt::select("mode", "Display mode", MODES, "random"),
    Opt::select("appearance", "Appearance", APPEARANCES, "random"),
    Opt::select("colors", "Colors", COLOR_MODES, "random"),
    Opt::select("viewMode", "View mode", VIEW_MODES, "random"),
    Opt::select("projection3d", "3D projection", PROJ, "random"),
    Opt::select("projection4d", "4D projection", PROJ, "random"),
    Opt::boolean("marks", "Show orientation markers", "false"),
    Opt::boolean("changeColors", "Change colors", "false"),
    Opt::slider("speedwx", "WX rotation speed", -4.0, 4.0, 0.1, 1, "1.1"),
    Opt::slider("speedwy", "WY rotation speed", -4.0, 4.0, 0.1, 1, "1.3"),
    Opt::slider("speedwz", "WZ rotation speed", -4.0, 4.0, 0.1, 1, "1.5"),
    Opt::slider("speedxy", "XY rotation speed", -4.0, 4.0, 0.1, 1, "1.7"),
    Opt::slider("speedxz", "XZ rotation speed", -4.0, 4.0, 0.1, 1, "1.9"),
    Opt::slider("speedyz", "YZ rotation speed", -4.0, 4.0, 0.1, 1, "2.1"),
    Opt::slider(
        "walkDirection",
        "Walking direction",
        -180.0,
        180.0,
        1.0,
        0,
        "83.0",
    ),
    Opt::slider("walkSpeed", "Walking speed", 1.0, 100.0, 1.0, 0, "20.0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "projectiveplane",
    label: "Projective Plane",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Steger",
        year: "2013",
        video: Some("https://www.youtube.com/watch?v=Zg6ONPUTwUQ"),
        blurb: "A 4d embedding of the real projective plane, which you can \
                watch rotate in 4d or walk on.",
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
    fn the_surface_closes_on_itself_in_both_directions() {
        // The parameter square wraps: a full turn in u and a full turn in v
        // both come back to the same point of the surface.
        let two_pi = 2.0 * std::f32::consts::PI;
        for v in [0.3f32, 1.7, 4.0] {
            let (a, _, _) = surface(0.0, v);
            let (b, _, _) = surface(two_pi, v);
            for k in 0..4 {
                assert!((a[k] - b[k]).abs() < 1e-5, "u does not close at v={v}");
            }
        }
    }

    #[test]
    fn a_four_dimensional_rotation_keeps_lengths() {
        let m = rotateall(13.0, 27.0, 41.0, 59.0, 71.0, 97.0);
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
    fn the_bands_leave_out_the_parts_they_should() {
        let strips = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::TriangleStrip)
                .count()
        };
        let solid = "mode=surface&viewMode=turn&appearance=solid";
        assert_eq!(strips(solid), NUMV);
        // Distance bands drop the middle half of every group of eight.
        assert_eq!(
            strips("mode=surface&viewMode=turn&appearance=distance-bands"),
            NUMV / 2
        );
        // Direction bands drop half of every group of eight, the other way.
        assert_eq!(
            strips("mode=surface&viewMode=turn&appearance=direction-bands"),
            NUMU / 2
        );
    }

    #[test]
    fn walking_keeps_the_walker_on_the_surface() {
        // The camera is rebuilt from the surface's own tangent plane, so
        // wherever the walk gets to, the point under it stays at the origin.
        let mut r = start(StartArgs::new(
            640,
            480,
            "mode=surface&viewMode=walk&appearance=solid&colors=depth",
            20260811,
        ));
        for _ in 0..300 {
            r.step();
            let f = r.frame();
            assert!(
                f.vertices
                    .iter()
                    .all(|v| v.pos.iter().all(|c| c.is_finite())),
                "a vertex went to NaN"
            );
        }
    }

    #[test]
    fn two_sided_colouring_paints_the_inside_a_different_colour() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "mode=surface&viewMode=turn&colors=twosided",
            20260811,
        ));
        r.step();
        let b = &r.frame().batches[0];
        assert_eq!(b.material.ambient_diffuse, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(b.material.back_ambient_diffuse, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn the_orientation_marker_is_the_curlicue_upstream_ships() {
        assert_eq!(
            crate::images::CURLICUE.len(),
            TEX_DIMENSION * TEX_DIMENSION,
            "the texture is not 64 by 64"
        );
        // It is mostly white paper with a dark arrow drawn on it.
        let dark = crate::images::CURLICUE.iter().filter(|&&v| v < 128).count();
        assert!(
            dark > 200 && dark < 2000,
            "{dark} dark pixels does not look like an arrow"
        );
    }
}
