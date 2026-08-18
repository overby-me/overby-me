//! Port of `hacks/glx/klein.c`.
//!
//! ```text
//! klein --- Shows a Klein bottle that rotates in 4d or on which you
//!   can walk
//!
//! Copyright (c) 2005-2026 Carsten Steger <carsten@mirsanmir.org>.
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
//! A Klein bottle in four dimensions, in three different guises. Unlike the
//! familiar glass bottle with its neck pushed through its side, none of these
//! passes through itself, because in four dimensions it does not have to.
//!
//! The figure-8 bottle is a figure-8 cross-section swept round a circle with
//! a half twist, lifted into 4D so the crossing comes apart. The pinched torus
//! is a torus whose tube shrinks to nothing at one point, with the pinch
//! resolved in the fourth coordinate. The Lawson bottle is the one that lives
//! naturally on the 3-sphere and is the most symmetric of the three.
//!
//! Like its siblings [`super::projectiveplane`] and [`super::etruscanvenus`],
//! it can be walked on as well as watched, and its two-sided colouring or its
//! orientation markers show that it has no inside and no outside.

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
const DELTAY: f32 = 0.02;
const DRHO: f32 = 0.7;
const DSIGMA: f32 = 1.1;
const DTAU: f32 = 1.7;

const TEX_DIMENSION: usize = 64;

/// The radius of the circle the figure-8 cross-section is swept around, and
/// the same for the pinched torus. `RADIUS_INCR` is how much bigger than that
/// the surface gets, and dividing by the sum is what scales it to fit.
const FIGURE_8_RADIUS: f32 = 2.0;
const PINCHED_TORUS_RADIUS: f32 = 2.0;
const RADIUS_INCR: f32 = 1.25;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bottle {
    Figure8,
    PinchedTorus,
    Lawson,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Display {
    Wireframe,
    Surface,
    Transparent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Colors {
    OneSided,
    TwoSided,
    Rainbow,
    Depth,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Walk,
    Turn,
    WalkTurn,
}

struct Klein {
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
    walk_direction: f32,
    walk_speed: f32,

    offset4d: [f32; 4],
    offset3d: [f32; 3],

    /// The surface and its two tangents at every mesh point, in 4D, plus the
    /// static colour and texture coordinate. None of it moves, so all of it is
    /// computed once.
    x: Vec<[f32; 4]>,
    xu: Vec<[f32; 4]>,
    xv: Vec<[f32; 4]>,
    col: Vec<[f32; 4]>,
    tex: Vec<[f32; 2]>,
    /// The projection of the surface into 3D, rebuilt every frame.
    pos: Vec<[f32; 3]>,
    pnorm: Vec<[f32; 3]>,

    trackball: Trackball,
    aspect: f32,
    texture: u32,

    bottle: Bottle,
    display: Display,
    bands: bool,
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

/// `rotateall`, all six planes of a four-dimensional rotation.
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

fn apply4(m: &[[f32; 4]; 4], v: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|l| (0..4).map(|k| m[l][k] * v[k]).sum())
}

fn scale_to(v: &mut [f32; 3], len: f32) {
    let t = len / (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    for c in v.iter_mut() {
        *c *= t;
    }
}

/// The surface and its two tangents at one point, in four dimensions.
fn surface(bottle: Bottle, u: f32, v: f32) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let (su, cu) = (u.sin(), u.cos());
    let (sv, cv) = (v.sin(), v.cos());
    let (sv2, cv2) = ((0.5 * v).sin(), (0.5 * v).cos());
    let (s2u, c2u) = ((2.0 * u).sin(), (2.0 * u).cos());
    let (mut x, mut xu, mut xv);
    let scale;
    match bottle {
        Bottle::Figure8 => {
            // A figure-8 cross-section swept round a circle with a half twist.
            let ring = su * cv2 - s2u * sv2 + FIGURE_8_RADIUS;
            x = [ring * cv, ring * sv, su * sv2 + s2u * cv2, cu];
            let d = cu * cv2 - 2.0 * c2u * sv2;
            xu = [d * cv, d * sv, cu * sv2 + 2.0 * c2u * cv2, -su];
            let e = -0.5 * su * sv2 - 0.5 * s2u * cv2;
            xv = [
                e * cv - ring * sv,
                e * sv + ring * cv,
                0.5 * su * cv2 - 0.5 * s2u * sv2,
                0.0,
            ];
            scale = FIGURE_8_RADIUS + RADIUS_INCR;
        }
        Bottle::PinchedTorus => {
            // A torus whose tube shrinks to nothing at one point, with the
            // pinch pulled apart in the fourth coordinate.
            let ring = PINCHED_TORUS_RADIUS + cu;
            x = [ring * cv, ring * sv, su * cv2, su * sv2];
            xu = [-su * cv, -su * sv, cu * cv2, cu * sv2];
            xv = [-ring * sv, ring * cv, -0.5 * su * sv2, 0.5 * su * cv2];
            scale = PINCHED_TORUS_RADIUS + RADIUS_INCR;
        }
        Bottle::Lawson => {
            x = [cu * cv, cu * sv, su * sv2, su * cv2];
            xu = [-su * cv, -su * sv, cu * sv2, cu * cv2];
            xv = [-cu * sv, cu * cv, 0.5 * su * cv2, -0.5 * su * sv2];
            scale = 1.0;
        }
    }
    for l in 0..4 {
        x[l] /= scale;
        xu[l] /= scale;
        xv[l] /= scale;
    }
    (x, xu, xv)
}

/// The angle the depth colouring reads: the fourth coordinate of the surface,
/// before it is scaled to fit, mapped onto two thirds of the colour wheel.
fn depth_angle(bottle: Bottle, u: f32, v: f32) -> f32 {
    let w = match bottle {
        Bottle::Figure8 => u.cos(),
        Bottle::PinchedTorus => u.sin() * (0.5 * v).sin(),
        Bottle::Lawson => u.sin() * (0.5 * v).cos(),
    };
    (w + 1.0) * std::f32::consts::PI * 2.0 / 3.0
}

impl Klein {
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

    /// The two parameters of a mesh point. `u` runs backwards, which is
    /// upstream's, and decides which way round the surface is wound.
    fn params(&self, i: usize, j: usize) -> (f32, f32) {
        let two_pi = 2.0 * std::f32::consts::PI;
        (
            -two_pi * j as f32 / NUMU as f32,
            two_pi * i as f32 / NUMV as f32,
        )
    }

    fn angle_at(&self, u: f32, v: f32) -> f32 {
        if self.colors == Colors::Depth {
            depth_angle(self.bottle, u, v)
        } else {
            v
        }
    }

    /// `setup_figure8` and its two siblings: the surface never moves, so all
    /// of it is computed once.
    fn setup(&mut self) {
        let two_pi = 2.0 * std::f32::consts::PI;
        for i in 0..=NUMV {
            for j in 0..=NUMU {
                let k = i * (NUMU + 1) + j;
                let (u, v) = self.params(i, j);
                if !self.change_colors {
                    self.col[k] = self.color(self.angle_at(u, v), &rotateall3d(0.0, 0.0, 0.0));
                }
                self.tex[k][0] = -32.0 * u / two_pi;
                self.tex[k][1] = 32.0 * v / two_pi;
                let (x, xu, xv) = surface(self.bottle, u, v);
                self.x[k] = x;
                self.xu[k] = xu;
                self.xv[k] = xv;
            }
        }
    }

    /// `project_4d_point_to_3d`.
    fn project(&self, y: [f32; 4]) -> [f32; 3] {
        if !self.perspective_4d {
            std::array::from_fn(|l| y[l] + self.offset4d[l])
        } else {
            let s = y[3] + self.offset4d[3];
            std::array::from_fn(|l| (y[l] + self.offset4d[l]) / s)
        }
    }

    /// `compute_tangent_space_basis_rotation`, then the offset that puts the
    /// walker's own position at the origin.
    fn compute_walk_frame(&mut self) -> [[f32; 4]; 4] {
        let mat = rotateall4d(self.zeta, self.eta, self.theta);
        let (xx, xxu, xxv) = surface(self.bottle, self.umove, self.vmove);
        let y = apply4(&mat, xx);
        let yu = apply4(&mat, xxu);
        let yv = apply4(&mat, xxv);

        let (mut pu, mut pv) = ([0.0f32; 3], [0.0f32; 3]);
        if !self.perspective_4d {
            pu.copy_from_slice(&yu[..3]);
            pv.copy_from_slice(&yv[..3]);
        } else {
            let s = y[3] + self.offset4d[3];
            let q = 1.0 / s;
            let t = q * q;
            for l in 0..3 {
                let r = y[l] + self.offset4d[l];
                pu[l] = (yu[l] * s - r * yu[3]) * t;
                pv[l] = (yv[l] * s - r * yv[3]) * t;
            }
        }

        let mut n = [
            pu[1] * pv[2] - pu[2] * pv[1],
            pu[2] * pv[0] - pu[0] * pv[2],
            pu[0] * pv[1] - pu[1] * pv[0],
        ];
        scale_to(&mut n, 1.0 / (self.side * 4.0));
        let mut pm = std::array::from_fn(|l| pu[l] * self.dumove + pv[l] * self.dvmove);
        scale_to(&mut pm, 0.25);
        let mut b = [
            n[1] * pm[2] - n[2] * pm[1],
            n[2] * pm[0] - n[0] * pm[2],
            n[0] * pm[1] - n[1] * pm[0],
        ];
        scale_to(&mut b, 0.25);

        // Read the three Euler angles back out of the frame the three basis
        // vectors make.
        let deg = 180.0 / std::f32::consts::PI;
        self.alpha = (-n[2]).atan2(-pm[2]) * deg;
        self.beta = (-b[2]).atan2((b[0] * b[0] + b[1] * b[1]).sqrt()) * deg;
        self.delta = b[1].atan2(-b[0]) * deg;

        let mat = rotateall(
            self.alpha, self.beta, self.delta, self.zeta, self.eta, self.theta,
        );
        let p = self.project(apply4(&mat, xx));
        self.offset3d = [-p[0], -p[1] - DELTAY, -p[2]];
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

    let kb = g.res.string("kleinBottle").to_string();
    let bottle = match kb.as_str() {
        "figure-8" => Bottle::Figure8,
        "pinched-torus" => Bottle::PinchedTorus,
        "lawson" => Bottle::Lawson,
        _ => match pick(&kb, 3) {
            0 => Bottle::Figure8,
            1 => Bottle::PinchedTorus,
            _ => Bottle::Lawson,
        },
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
    let bands = match appear.as_str() {
        "solid" => false,
        "bands" => true,
        _ => random().is_multiple_of(2),
    };

    let cols = g.res.string("colors").to_string();
    let colors = match cols.as_str() {
        "one-sided" => Colors::OneSided,
        "two-sided" => Colors::TwoSided,
        "rainbow" => Colors::Rainbow,
        "depth" => Colors::Depth,
        _ => match pick(&cols, 4) {
            0 => Colors::OneSided,
            1 => Colors::TwoSided,
            2 => Colors::Rainbow,
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

    let p3 = g.res.string("projection3d").to_string();
    let perspective_3d = match p3.as_str() {
        "perspective" => true,
        "orthographic" => false,
        _ => random().is_multiple_of(2),
    };
    let p4 = g.res.string("projection4d").to_string();
    let perspective_4d = match p4.as_str() {
        "perspective" => true,
        "orthographic" => false,
        _ => random().is_multiple_of(2),
    };

    // Each bottle sits at its own distance, and how far depends on how the
    // fourth dimension is being flattened.
    let (offset4d, offset3d) = match bottle {
        Bottle::Figure8 => (
            [0.0, 0.0, 0.0, 1.5],
            [0.0, 0.0, if perspective_4d { -1.9 } else { -2.1 }],
        ),
        Bottle::PinchedTorus => ([0.0, 0.0, 0.0, 1.4], [0.0, 0.0, -2.0]),
        Bottle::Lawson => (
            [
                0.0,
                0.0,
                0.0,
                if perspective_4d && !perspective_3d {
                    1.5
                } else {
                    1.1
                },
            ],
            [0.0, 0.0, if perspective_4d { -5.0 } else { -2.0 }],
        ),
    };

    let n = (NUMU + 1) * (NUMV + 1);
    let turning = view == View::Turn;

    let mut this = Klein {
        alpha: if turning { frand(360.0) as f32 } else { 0.0 },
        beta: if turning { frand(360.0) as f32 } else { 0.0 },
        delta: if turning { frand(360.0) as f32 } else { 0.0 },
        zeta: 0.0,
        // The Lawson bottle is turned in the xz plane to start with, which is
        // how it is usually drawn.
        eta: if bottle == Bottle::Lawson { 45.0 } else { 0.0 },
        theta: 0.0,
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
        walk_direction: g.res.float("walkDirection") as f32,
        walk_speed: g.res.float("walkSpeed") as f32,
        offset4d,
        offset3d,
        x: vec![[0.0; 4]; n],
        xu: vec![[0.0; 4]; n],
        xv: vec![[0.0; 4]; n],
        col: vec![[1.0; 4]; n],
        tex: vec![[0.0; 2]; n],
        pos: vec![[0.0; 3]; n],
        pnorm: vec![[0.0; 3]; n],
        trackball: Trackball::new(),
        aspect: 1.0,
        texture: 0,
        bottle,
        display,
        bands,
        colors,
        view,
        marks: g.res.bool("marks") && display != Display::Wireframe,
        change_colors: g.res.bool("changeColors"),
        perspective_3d,
        perspective_4d,
    };
    this.setup();

    this.texture = g.glx.gen_texture();
    g.glx.bind_texture(this.texture);
    let rgba: Vec<u8> = crate::images::CURLICUE
        .iter()
        .flat_map(|&v| [v, v, v, 255])
        .collect();
    g.glx
        .tex_image_2d(TEX_DIMENSION as i32, TEX_DIMENSION as i32, rgba);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Klein {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let walking = self.view == View::Walk || self.view == View::WalkTurn;
        if !self.trackball.button_down() {
            let s = self.speed_scale;
            if self.view == View::Turn {
                self.alpha = (self.alpha + self.speeds[0] * s) % 360.0;
                self.beta = (self.beta + self.speeds[1] * s) % 360.0;
                self.delta = (self.delta + self.speeds[2] * s) % 360.0;
            }
            if self.view == View::Turn || self.view == View::WalkTurn {
                self.zeta = (self.zeta + self.speeds[3] * s) % 360.0;
                self.eta = (self.eta + self.speeds[4] * s) % 360.0;
                self.theta = (self.theta + self.speeds[5] * s) % 360.0;
            }
            if walking {
                let two_pi = 2.0 * std::f32::consts::PI;
                let rad = self.walk_direction * std::f32::consts::PI / 180.0;
                self.dvmove = rad.cos() * self.walk_speed * std::f32::consts::PI / 4096.0;
                self.vmove += self.dvmove;
                // A full turn round v comes back with u reversed and the
                // walker on the other side, which is the bottle's gluing.
                if self.vmove >= two_pi {
                    self.vmove -= two_pi;
                    self.umove = two_pi - self.umove;
                    self.side = -self.side;
                }
                self.dumove =
                    self.side * rad.sin() * self.walk_speed * std::f32::consts::PI / 4096.0;
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
        if self.perspective_3d || walking {
            let near = if walking { 0.01 } else { 0.1 };
            g.glx.perspective(60.0, self.aspect, near, 10.0);
        } else if self.aspect >= 1.0 {
            g.glx.ortho(-self.aspect, self.aspect, -1.0, 1.0, 0.1, 10.0);
        } else {
            g.glx
                .ortho(-1.0, 1.0, -1.0 / self.aspect, 1.0 / self.aspect, 0.1, 10.0);
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
            g.glx.mult_matrix(self.trackball.matrix());
            rotateall(
                self.alpha, self.beta, self.delta, self.zeta, self.eta, self.theta,
            )
        };

        // Project every mesh point from 4D to 3D, and differentiate the
        // projection so the normals stay normals.
        for o in 0..self.x.len() {
            let y = apply4(&mat, self.x[o]);
            let yu = apply4(&mat, self.xu[o]);
            let yv = apply4(&mat, self.xv[o]);
            let (mut pu, mut pv) = ([0.0f32; 3], [0.0f32; 3]);
            let p = self.project(y);
            if !self.perspective_4d {
                pu.copy_from_slice(&yu[..3]);
                pv.copy_from_slice(&yv[..3]);
            } else {
                let s = y[3] + self.offset4d[3];
                let q = 1.0 / s;
                let t = q * q;
                for l in 0..3 {
                    let r = y[l] + self.offset4d[l];
                    pu[l] = (yu[l] * s - r * yu[3]) * t;
                    pv[l] = (yv[l] * s - r * yv[3]) * t;
                }
            }
            self.pos[o] = std::array::from_fn(|l| p[l] + self.offset3d[l]);
            let mut n = [
                pu[1] * pv[2] - pu[2] * pv[1],
                pu[2] * pv[0] - pu[0] * pv[2],
                pu[0] * pv[1] - pu[1] * pv[0],
            ];
            scale_to(&mut n, 1.0);
            self.pnorm[o] = n;
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

        if self.change_colors && per_vertex {
            for i in 0..=NUMV {
                for j in 0..=NUMU {
                    let o = i * (NUMU + 1) + j;
                    let (u, v) = self.params(i, j);
                    let c = self.color(self.angle_at(u, v), &color_mat);
                    self.col[o] = c;
                }
            }
        }

        // A wireframe strip is upstream's quad strip under a polygon mode of
        // GL_LINE; there is no polygon mode here, so the edges go out as lines.
        let emit = |g: &mut Gl, strip: &[usize]| {
            if !wire {
                g.glx.begin(Shape::TriangleStrip);
                for &o in strip {
                    g.glx
                        .normal3f(self.pnorm[o][0], self.pnorm[o][1], self.pnorm[o][2]);
                    g.glx.tex_coord2f(self.tex[o][0], self.tex[o][1]);
                    if per_vertex {
                        let c = self.col[o];
                        g.glx.color4f(c[0], c[1], c[2], c[3]);
                    }
                    let p = self.pos[o];
                    g.glx.vertex3f(p[0], p[1], p[2]);
                }
                g.glx.end();
                return;
            }
            g.glx.begin(Shape::Lines);
            for q in strip.chunks_exact(2).collect::<Vec<_>>().windows(2) {
                let corners = [q[0][0], q[0][1], q[1][1], q[1][0]];
                for e in 0..4 {
                    for o in [corners[e], corners[(e + 1) % 4]] {
                        if per_vertex {
                            let c = self.col[o];
                            g.glx.color4f(c[0], c[1], c[2], c[3]);
                        }
                        let p = self.pos[o];
                        g.glx.vertex3f(p[0], p[1], p[2]);
                    }
                }
            }
            g.glx.end();
        };

        let mut strip: Vec<usize> = Vec::with_capacity(2 * (NUMU + 1));
        for i in 0..NUMV {
            if self.bands && (i & (NUMB - 1)) >= NUMB / 2 {
                continue;
            }
            strip.clear();
            for j in 0..=NUMU {
                for k in 0..=1 {
                    strip.push((i + k) * (NUMU + 1) + j);
                }
            }
            emit(g, &strip);
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:         25000",
    "*showFPS:       False",
    "*kleinBottle:   random",
    "*mode:          random",
    "*appearance:    random",
    "*colors:        random",
    "*viewMode:      random",
    "*marks:         False",
    "*changeColors:  False",
    "*projection3d:  random",
    "*projection4d:  random",
    "*speedwx:       1.1",
    "*speedwy:       1.3",
    "*speedwz:       1.5",
    "*speedxy:       1.7",
    "*speedxz:       1.9",
    "*speedyz:       2.1",
    "*walkDirection: 7.0",
    "*walkSpeed:     20.0",
];

const BOTTLES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random shape",
    },
    SelectItem {
        value: "figure-8",
        label: "Figure 8",
    },
    SelectItem {
        value: "pinched-torus",
        label: "Pinched torus",
    },
    SelectItem {
        value: "lawson",
        label: "Lawson",
    },
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random surface",
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
        label: "Random pattern",
    },
    SelectItem {
        value: "solid",
        label: "Solid object",
    },
    SelectItem {
        value: "bands",
        label: "See-through bands",
    },
];

const COLOR_MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random coloration",
    },
    SelectItem {
        value: "one-sided",
        label: "One-sided",
    },
    SelectItem {
        value: "two-sided",
        label: "Two-sided",
    },
    SelectItem {
        value: "rainbow",
        label: "Rainbow colors",
    },
    SelectItem {
        value: "depth",
        label: "4d depth colors",
    },
];

const VIEW_MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random motion",
    },
    SelectItem {
        value: "walk",
        label: "Walk",
    },
    SelectItem {
        value: "turn",
        label: "Turn",
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
    Opt::select("kleinBottle", "Shape", BOTTLES, "random"),
    Opt::select("viewMode", "View mode", VIEW_MODES, "random"),
    Opt::boolean("marks", "Show orientation marks", "false"),
    Opt::select("mode", "Display mode", MODES, "random"),
    Opt::select("appearance", "Appearance", APPEARANCES, "random"),
    Opt::select("colors", "Colors", COLOR_MODES, "random"),
    Opt::select("projection3d", "3D projection", PROJ, "random"),
    Opt::select("projection4d", "4D projection", PROJ, "random"),
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
        "7.0",
    ),
    Opt::slider("walkSpeed", "Walking speed", 1.0, 100.0, 1.0, 0, "20.0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "klein",
    label: "Klein",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Steger",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=c2gvyGVNG80"),
        blurb: "A Klein bottle in four dimensions, as a figure-8, a pinched \
                torus or a Lawson bottle, which you can walk on.",
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

    /// All three bottles close up in both parameters, and all three do it the
    /// Klein bottle's way: a full turn in v comes back with u reversed.
    #[test]
    fn every_bottle_glues_up_the_same_way() {
        let two_pi = 2.0 * std::f32::consts::PI;
        for bottle in [Bottle::Figure8, Bottle::PinchedTorus, Bottle::Lawson] {
            for u in [0.3f32, 2.2, 4.5] {
                let (a, _, _) = surface(bottle, u, 0.0);
                let (b, _, _) = surface(bottle, two_pi - u, two_pi);
                let dist: f32 = (0..4).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
                assert!(dist < 1e-5, "the v seam is {dist} apart at u={u}");
            }
        }
    }

    /// The Lawson bottle is the one that lives on the unit 3-sphere, which is
    /// what makes it the most symmetric of the three, and also what stops it
    /// being embedded: no closed non-orientable surface fits in a 3-sphere, so
    /// this one has to cross itself even with four dimensions to work in.
    #[test]
    fn the_lawson_bottle_lies_on_the_three_sphere() {
        let two_pi = 2.0 * std::f32::consts::PI;
        for i in 0..16 {
            for j in 0..16 {
                let u = two_pi * j as f32 / 16.0;
                let v = two_pi * i as f32 / 16.0;
                let (x, _, _) = surface(Bottle::Lawson, u, v);
                let r: f32 = x.iter().map(|c| c * c).sum::<f32>().sqrt();
                assert!((r - 1.0).abs() < 1e-5, "radius {r} at ({u}, {v})");
            }
        }
        // And it does cross itself: the circle at u = 0 is the circle at pi.
        let (a, _, _) = surface(Bottle::Lawson, 0.0, 0.7);
        let (b, _, _) = surface(Bottle::Lawson, std::f32::consts::PI, 0.7 + two_pi / 2.0);
        let dist: f32 = (0..4).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
        assert!(dist < 1e-5, "the two sheets are {dist} apart");
    }

    /// The other two are what four dimensions buys you: they do not pass
    /// through themselves at all, so no two distinct mesh points share a
    /// position.
    #[test]
    fn nothing_passes_through_itself() {
        for bottle in [Bottle::Figure8, Bottle::PinchedTorus] {
            let n = 24;
            let mut points = Vec::new();
            for i in 0..n {
                for j in 0..n {
                    let two_pi = 2.0 * std::f32::consts::PI;
                    let u = two_pi * j as f32 / n as f32;
                    let v = two_pi * i as f32 / n as f32;
                    let (x, _, _) = surface(bottle, u, v);
                    points.push(x);
                }
            }
            let mut min = f32::MAX;
            for (a, p) in points.iter().enumerate() {
                for q in points.iter().skip(a + 1) {
                    let d: f32 = (0..4).map(|k| (p[k] - q[k]).powi(2)).sum();
                    min = min.min(d.sqrt());
                }
            }
            assert!(min > 0.01, "two points are {min} apart in 4d");
        }
    }

    /// The figure-8 bottle is a figure-8 swept round a circle, and a figure-8
    /// crosses itself once. In three dimensions the two strokes of the 8 meet;
    /// the fourth coordinate is what pulls them apart, and it does it all the
    /// way round the ring.
    #[test]
    fn the_stroke_of_the_eight_is_undone_in_the_fourth_dimension() {
        for step in 0..8 {
            let v = 2.0 * std::f32::consts::PI * step as f32 / 8.0;
            let (a, _, _) = surface(Bottle::Figure8, 0.0, v);
            let (b, _, _) = surface(Bottle::Figure8, std::f32::consts::PI, v);
            let in_3d: f32 = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
            let in_4d: f32 = (0..4).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
            assert!(in_3d < 1e-6, "at v={v} the strokes miss by {in_3d} in 3d");
            assert!(in_4d > 0.5, "at v={v} the strokes are only {in_4d} apart");
            // And the swept circle is where it should be.
            let ring = (a[0] * a[0] + a[1] * a[1]).sqrt();
            let expect = FIGURE_8_RADIUS / (FIGURE_8_RADIUS + RADIUS_INCR);
            assert!((ring - expect).abs() < 1e-6, "the ring is {ring} across");
        }
    }

    /// In three dimensions the pinched torus really is pinched: half way round
    /// the ring its tube collapses to a line traversed twice, and the two
    /// sheets only come apart in the fourth coordinate.
    #[test]
    fn the_pinch_is_only_a_pinch_in_three_dimensions() {
        let pinch = std::f32::consts::PI;
        let (a, _, _) = surface(Bottle::PinchedTorus, 1.0, pinch);
        let (b, _, _) = surface(Bottle::PinchedTorus, -1.0, pinch);
        let in_3d: f32 = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
        let in_4d: f32 = (0..4).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
        assert!(in_3d < 1e-5, "the pinch does not meet in 3d: {in_3d}");
        assert!(in_4d > 0.4, "the pinch is not pulled apart in 4d: {in_4d}");
    }

    #[test]
    fn the_bands_are_half_the_strips() {
        let strips = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::TriangleStrip)
                .count()
        };
        let base = "mode=surface&viewMode=turn&kleinBottle=lawson";
        assert_eq!(strips(&format!("{base}&appearance=solid")), NUMV);
        assert_eq!(strips(&format!("{base}&appearance=bands")), NUMV / 2);
    }

    #[test]
    fn walking_works_on_all_three() {
        for bottle in ["figure-8", "pinched-torus", "lawson"] {
            let mut r = start(StartArgs::new(
                640,
                480,
                &format!("mode=surface&viewMode=walk&colors=depth&kleinBottle={bottle}"),
                20260811,
            ));
            for _ in 0..150 {
                r.step();
                assert!(
                    r.frame()
                        .vertices
                        .iter()
                        .all(|v| v.pos.iter().all(|c| c.is_finite())),
                    "{bottle} produced a NaN while walking"
                );
            }
        }
    }
}
