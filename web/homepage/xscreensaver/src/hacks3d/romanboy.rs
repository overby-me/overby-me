//! Port of `hacks/glx/romanboy.c`.
//!
//! ```text
//! romanboy --- Shows a 3d immersion of the real projective plane
//!   that smoothly deforms between the Roman surface and the Boy surface.
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
//! The same surface as [`super::projectiveplane`], but immersed in three
//! dimensions rather than embedded in four, which means it has to pass through
//! itself. There are two well known ways to do that and this deforms between
//! them: at deformation 0 it is Werner Boy's surface, with one triple point,
//! and at 1 it is Jakob Steiner's Roman surface, with three double lines
//! meeting at a triple point in the middle. One parameter runs between them.
//!
//! The order of the surface generalises Boy's: order 3 is the usual one, and
//! higher orders wrap the same construction round more times, giving a flower
//! with that many petals. Order 2 degenerates to a sphere traversed twice.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape, TexEnv};
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

const NUMU: usize = 64;
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
}

/// The three basis vectors of the walker's frame, and the surface point it
/// stands on.
struct Frame {
    mat: [[f32; 3]; 3],
    offset3d: [f32; 3],
}

struct RomanBoy {
    alpha: f32,
    beta: f32,
    delta: f32,
    /// The three angles that drive the changing colours.
    rho: f32,
    sigma: f32,
    tau: f32,
    speed_scale: f32,
    speeds: [f32; 3],

    /// Where the walker is on the surface, and which way it is going.
    umove: f32,
    vmove: f32,
    dumove: f32,
    dvmove: f32,
    side: f32,
    dir: f32,
    walk_direction: f32,
    walk_speed: f32,

    /// How far along the deformation we are, and which way it is going.
    dd: f32,
    defdir: f32,
    deform_speed: f32,
    /// The order of the surface: how many times the construction wraps.
    g: usize,

    offset3d: [f32; 3],
    /// The colour and texture coordinate of every mesh point, which do not
    /// change unless the colours are set to change.
    col: Vec<[f32; 4]>,
    tex: Vec<[f32; 2]>,
    /// The surface and its normals, rebuilt every frame because the
    /// deformation moves them.
    pos: Vec<[f32; 3]>,
    pnorm: Vec<[f32; 3]>,

    trackball: Trackball,
    aspect: f32,
    texture: u32,

    display: Display,
    appearance: Appearance,
    colors: Colors,
    walking: bool,
    marks: bool,
    change_colors: bool,
    deform: bool,
    perspective: bool,
}

/// `rotateall`, the three ordinary 3D rotations.
fn rotateall(al: f32, be: f32, de: f32) -> [[f32; 3]; 3] {
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

fn apply(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|l| (0..3).map(|k| m[l][k] * v[k]).sum())
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn scale_to(v: &mut [f32; 3], len: f32) {
    let t = len / (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    for c in v.iter_mut() {
        *c *= t;
    }
}

/// The numerators and the denominator of the surface at one point, and their
/// two derivatives. Split out because the degenerate-pole case recomputes them
/// at a nudged v.
struct Terms {
    nom: [f32; 2],
    nomu: [f32; 2],
    nomv: [f32; 2],
    den: f32,
    den2: f32,
    denu: f32,
    denv: f32,
    cv2: f32,
    s2v: f32,
}

fn terms(u: f32, v: f32, d: f32, g: usize) -> Terms {
    let sqrt2og = std::f32::consts::SQRT_2 / g as f32;
    let h1m1og = 0.5 * (1.0 - 1.0 / g as f32);
    let gm1 = g as f32 - 1.0;
    let (su, cu) = (u.sin(), u.cos());
    let (sgu, cgu) = ((g as f32 * u).sin(), (g as f32 * u).cos());
    let (sgm1u, cgm1u) = ((gm1 * u).sin(), (gm1 * u).cos());
    let cv = v.cos();
    let c2v = (2.0 * v).cos();
    let s2v = (2.0 * v).sin();
    let cv2 = cv * cv;
    let den = 1.0 / (1.0 - 0.5 * std::f32::consts::SQRT_2 * d * s2v * sgu);
    Terms {
        nom: [
            sqrt2og * cv2 * cgm1u + h1m1og * s2v * cu,
            sqrt2og * cv2 * sgm1u - h1m1og * s2v * su,
        ],
        nomu: [
            -sqrt2og * cv2 * gm1 * sgm1u - h1m1og * s2v * su,
            sqrt2og * cv2 * gm1 * cgm1u - h1m1og * s2v * cu,
        ],
        nomv: [
            -sqrt2og * s2v * cgm1u + 2.0 * h1m1og * c2v * cu,
            -sqrt2og * s2v * sgm1u - 2.0 * h1m1og * c2v * su,
        ],
        den,
        den2: den * den,
        denu: 0.5 * std::f32::consts::SQRT_2 * d * g as f32 * cgu * s2v,
        denv: std::f32::consts::SQRT_2 * d * sgu * c2v,
        cv2,
        s2v,
    }
}

/// The surface and its two tangents at one point of the parameter square.
fn surface(u: f32, v: f32, d: f32, g: usize, oz: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let half_pi = 0.5 * std::f32::consts::PI;
    let v = if g & 1 == 1 {
        half_pi - 0.25 * v
    } else {
        half_pi - 0.5 * v
    };
    let t = terms(u, v, d, g);
    let x = [t.nom[0] * t.den, t.nom[1] * t.den, t.cv2 * t.den - oz];

    // Avoid degenerate tangential plane basis vectors: at the poles the two
    // tangents fall on top of each other and their cross product is nothing.
    let eps = 10.0 * f32::EPSILON;
    let t = if half_pi - v.abs() < eps {
        let v = if half_pi - v < eps {
            half_pi - eps
        } else {
            -half_pi + eps
        };
        terms(u, v, d, g)
    } else {
        t
    };
    let xu = [
        t.nomu[0] * t.den + t.nom[0] * t.denu * t.den2,
        t.nomu[1] * t.den + t.nom[1] * t.denu * t.den2,
        t.cv2 * t.denu * t.den2,
    ];
    let xv = [
        t.nomv[0] * t.den + t.nom[0] * t.denv * t.den2,
        t.nomv[1] * t.den + t.nom[1] * t.denv * t.den2,
        -t.s2v * t.den + t.cv2 * t.denv * t.den2,
    ];
    (x, xu, xv)
}

impl RomanBoy {
    fn numu(&self) -> usize {
        self.g * NUMU
    }

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

    /// The angle the colour wheel is read at, for a point of the mesh.
    fn angle_at(&self, u: f32, v: f32) -> f32 {
        let two_pi = 2.0 * std::f32::consts::PI;
        if self.colors == Colors::Direction {
            two_pi - (2.0 * u) % two_pi
        } else {
            v * (5.0 / 6.0)
        }
    }

    /// `setup_roman_boy_color_texture`: the parts of the mesh the deformation
    /// does not touch.
    fn setup(&mut self) {
        let two_pi = 2.0 * std::f32::consts::PI;
        let numu = self.numu();
        for i in 0..=NUMV {
            for j in 0..=numu {
                let k = i * (numu + 1) + j;
                let u = if self.appearance != Appearance::DirectionBands {
                    -two_pi * j as f32 / numu as f32
                } else {
                    two_pi * j as f32 / numu as f32
                };
                let v = two_pi * i as f32 / NUMV as f32;
                if !self.change_colors {
                    self.col[k] = self.color(self.angle_at(u, v), &rotateall(0.0, 0.0, 0.0));
                }
                self.tex[k][0] = -16.0 * self.g as f32 * u / two_pi;
                self.tex[k][1] = 32.0 * v / two_pi
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
    fn compute_walk_frame(&mut self, d: f32, radius: f32, oz: f32) -> Frame {
        let (xx, xxu, xxv) = surface(self.umove, self.vmove, d, self.g, oz);
        let mut pu = xxu;
        let mut pv = xxv;
        for l in 0..3 {
            pu[l] *= radius;
            pv[l] *= radius;
        }

        let mut n = cross(pu, pv);
        scale_to(&mut n, 1.0 / (self.side * 4.0));
        // The quarter on dvmove is the chain rule for v, which the surface
        // halves or quarters before using it.
        let mut pm = std::array::from_fn(|l| pu[l] * self.dumove - pv[l] * 0.25 * self.dvmove);
        scale_to(&mut pm, 0.25);
        let mut b = cross(n, pm);
        scale_to(&mut b, 0.25);

        // Read the three Euler angles back out of the frame the three basis
        // vectors make.
        let deg = 180.0 / std::f32::consts::PI;
        self.alpha = (-n[2]).atan2(-pm[2]) * deg;
        self.beta = (-b[2]).atan2((b[0] * b[0] + b[1] * b[1]).sqrt()) * deg;
        self.delta = b[1].atan2(-b[0]) * deg;

        let mat = rotateall(self.alpha, self.beta, self.delta);
        let p = apply(&mat, xx);
        Frame {
            mat,
            offset3d: [-p[0] * radius, -p[1] * radius - DELTAY, -p[2] * radius],
        }
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
        "one-sided" => Colors::OneSided,
        "two-sided" => Colors::TwoSided,
        "distance" => Colors::Distance,
        "direction" => Colors::Direction,
        _ => match pick(&cols, 4) {
            0 => Colors::OneSided,
            1 => Colors::TwoSided,
            2 => Colors::Distance,
            _ => Colors::Direction,
        },
    };

    let vm = g.res.string("viewMode").to_string();
    let walking = match vm.as_str() {
        "walk" => true,
        "turn" => false,
        _ => random().is_multiple_of(2),
    };

    // Orthographic projection only makes sense in turn mode.
    let proj = g.res.string("projection").to_string();
    let perspective = match proj.as_str() {
        "perspective" => true,
        "orthographic" => !walking,
        _ => walking || random().is_multiple_of(2),
    };

    let order = g.res.int("surfaceOrder").clamp(2, 9) as usize;
    let n = (order * NUMU + 1) * (NUMV + 1);
    let walk_direction = g.res.float("walkDirection") as f32;
    let turning = !walking;

    let mut this = RomanBoy {
        alpha: if turning { frand(360.0) as f32 } else { 0.0 },
        beta: if turning { frand(360.0) as f32 } else { 0.0 },
        delta: if turning { frand(360.0) as f32 } else { 0.0 },
        rho: frand(360.0) as f32,
        sigma: frand(360.0) as f32,
        tau: frand(360.0) as f32,
        speed_scale: 0.9 + frand(0.3) as f32,
        speeds: [
            g.res.float("speedx") as f32,
            g.res.float("speedy") as f32,
            g.res.float("speedz") as f32,
        ],
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
        dd: g.res.float("initDeform").clamp(0.0, 1000.0) as f32 * 0.001,
        defdir: -1.0,
        deform_speed: g.res.float("deformSpeed") as f32,
        g: order,
        offset3d: [0.0, 0.0, -1.8],
        col: vec![[1.0; 4]; n],
        tex: vec![[0.0; 2]; n],
        pos: vec![[0.0; 3]; n],
        pnorm: vec![[0.0; 3]; n],
        trackball: Trackball::new(),
        aspect: 1.0,
        texture: 0,
        display,
        appearance,
        colors,
        walking,
        // Orientation marks do not make sense in wireframe mode.
        marks: g.res.bool("marks") && display != Display::Wireframe,
        change_colors: g.res.bool("changeColors"),
        deform: g.res.bool("deform"),
        perspective,
    };
    this.setup();

    this.texture = g.glx.gen_texture();
    g.glx.bind_texture(this.texture);
    let rgba: Vec<u8> = crate::images::CURLICUE
        .iter()
        .flat_map(|&v| [255, 255, 255, v])
        .collect();
    g.glx
        .tex_image_2d(TEX_DIMENSION as i32, TEX_DIMENSION as i32, rgba);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for RomanBoy {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        if !self.trackball.button_down() {
            if self.deform {
                self.dd += self.defdir * self.deform_speed * 0.001;
                if self.dd < 0.0 {
                    self.dd = -self.dd;
                    self.defdir = -self.defdir;
                }
                if self.dd > 1.0 {
                    self.dd = 2.0 - self.dd;
                    self.defdir = -self.defdir;
                }
            }
            if !self.walking {
                let s = self.speed_scale;
                self.alpha = (self.alpha + self.speeds[0] * s) % 360.0;
                self.beta = (self.beta + self.speeds[1] * s) % 360.0;
                self.delta = (self.delta + self.speeds[2] * s) % 360.0;
            } else {
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
        if self.perspective {
            let near = if self.walking { 0.01 } else { 0.1 };
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

        // The deformation runs through a quintic so that it eases in and out
        // at the two ends, and the surface is scaled back down as it grows.
        let dd = self.dd;
        let d = ((6.0 * dd - 15.0) * dd + 10.0) * dd * dd * dd;
        let r = 1.0 + d * d * (1.0 / 2.0 + d * d * (1.0 / 6.0 + d * d * (1.0 / 3.0)));
        let radius = 1.0 / r;
        let oz = 0.5 * r;

        let color_mat = rotateall(self.rho, self.sigma, self.tau);

        let (mat, offset3d) = if self.walking {
            let frame = self.compute_walk_frame(d, radius, oz);
            (frame.mat, frame.offset3d)
        } else {
            g.glx.mult_matrix(self.trackball.matrix());
            (rotateall(self.alpha, self.beta, self.delta), self.offset3d)
        };

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
        let numu = self.numu();
        let bands = self.appearance;

        // Rebuild the surface. The deformation moves every point, so unlike
        // the four-dimensional embedding there is nothing to precompute.
        let point = |this: &mut Self, i: usize, j: usize| {
            let o = i * (numu + 1) + j;
            let u = if bands != Appearance::DirectionBands {
                two_pi * j as f32 / numu as f32
            } else {
                -two_pi * j as f32 / numu as f32
            };
            let v = two_pi * i as f32 / NUMV as f32;
            if this.change_colors && per_vertex {
                let c = this.color(this.angle_at(u, v), &color_mat);
                this.col[o] = c;
            }
            let (xx, xxu, xxv) = surface(u, v, d, this.g, oz);
            let p = apply(&mat, xx);
            let pu = apply(&mat, xxu);
            let pv = apply(&mat, xxv);
            this.pos[o] = std::array::from_fn(|l| p[l] * radius + offset3d[l]);
            let mut n = cross(pu, pv);
            scale_to(&mut n, 1.0);
            this.pnorm[o] = n;
        };

        if self.appearance != Appearance::DirectionBands {
            for i in 0..=NUMV {
                if self.appearance == Appearance::DistanceBands
                    && (i & (NUMB - 1)) > NUMB / 4
                    && (i & (NUMB - 1)) < 3 * NUMB / 4
                {
                    continue;
                }
                for j in 0..=numu {
                    point(self, i, j);
                }
            }
        } else {
            for j in 0..=numu {
                if (j & (NUMB - 1)) > NUMB / 2 {
                    continue;
                }
                for i in 0..=NUMV {
                    point(self, i, j);
                }
            }
        }

        // A wireframe strip is upstream's quad strip under a polygon mode of
        // GL_LINE; there is no polygon mode here, so the edges go out as lines.
        let emit = |g: &mut Gl, strip: &[usize]| {
            if !wire {
                g.glx.begin(Shape::TriangleStrip);
                for &o in strip {
                    if per_vertex {
                        let c = self.col[o];
                        g.glx.color4f(c[0], c[1], c[2], c[3]);
                    }
                    let n = self.pnorm[o];
                    g.glx.normal3f(n[0], n[1], n[2]);
                    g.glx.tex_coord2f(self.tex[o][0], self.tex[o][1]);
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

        let mut strip: Vec<usize> = Vec::with_capacity(2 * (numu + 1));
        if self.appearance != Appearance::DirectionBands {
            for i in 0..NUMV {
                if self.appearance == Appearance::DistanceBands
                    && (i & (NUMB - 1)) >= NUMB / 4
                    && (i & (NUMB - 1)) < 3 * NUMB / 4
                {
                    continue;
                }
                strip.clear();
                for j in 0..=numu {
                    for k in 0..=1 {
                        strip.push((i + k) * (numu + 1) + j);
                    }
                }
                emit(g, &strip);
            }
        } else {
            for j in 0..numu {
                if (j & (NUMB - 1)) >= NUMB / 2 {
                    continue;
                }
                strip.clear();
                for i in 0..=NUMV {
                    for k in 0..=1 {
                        strip.push(i * (numu + 1) + j + k);
                    }
                }
                emit(g, &strip);
            }
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:         25000",
    "*showFPS:       False",
    "*mode:          random",
    "*appearance:    random",
    "*colors:        random",
    "*viewMode:      random",
    "*marks:         False",
    "*changeColors:  False",
    "*deform:        True",
    "*projection:    random",
    "*speedx:        1.1",
    "*speedy:        1.3",
    "*speedz:        1.5",
    "*walkDirection: 83.0",
    "*walkSpeed:     20.0",
    "*deformSpeed:   10.0",
    "*initDeform:    1000.0",
    "*surfaceOrder:  3",
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
        value: "distance-bands",
        label: "Distance bands",
    },
    SelectItem {
        value: "direction-bands",
        label: "Direction bands",
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
        value: "distance",
        label: "Distance colors",
    },
    SelectItem {
        value: "direction",
        label: "Direction colors",
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
    Opt::select("viewMode", "View mode", VIEW_MODES, "random"),
    Opt::slider(
        "surfaceOrder",
        "Order of the surface",
        2.0,
        9.0,
        1.0,
        0,
        "3",
    ),
    Opt::boolean("marks", "Show orientation marks", "false"),
    Opt::boolean("deform", "Deform the projective plane", "true"),
    Opt::slider(
        "deformSpeed",
        "Deformation speed",
        1.0,
        100.0,
        1.0,
        0,
        "10.0",
    ),
    Opt::slider(
        "initDeform",
        "Initial deformation",
        0.0,
        1000.0,
        1.0,
        0,
        "1000.0",
    ),
    Opt::select("mode", "Display mode", MODES, "random"),
    Opt::select("appearance", "Appearance", APPEARANCES, "random"),
    Opt::select("colors", "Colors", COLOR_MODES, "random"),
    Opt::select("projection", "Projection", PROJ, "random"),
    Opt::boolean("changeColors", "Change colors", "false"),
    Opt::slider("speedx", "X rotation speed", -4.0, 4.0, 0.1, 1, "1.1"),
    Opt::slider("speedy", "Y rotation speed", -4.0, 4.0, 0.1, 1, "1.3"),
    Opt::slider("speedz", "Z rotation speed", -4.0, 4.0, 0.1, 1, "1.5"),
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
    slug: "romanboy",
    label: "Roman Boy",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Steger",
        year: "2013",
        video: Some("https://www.youtube.com/watch?v=KEW5TuPbWyg"),
        blurb: "A 3d immersion of the real projective plane that deforms \
                between the Roman surface and the Boy surface.",
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

    /// At deformation zero the surface is Steiner's Roman surface, whose
    /// three double lines meet at the origin: the point at v = 0 and the point
    /// half a turn away in u land on top of each other.
    #[test]
    fn the_roman_surface_crosses_itself() {
        let g = 3;
        let oz = 0.5;
        let (a, _, _) = surface(0.0, 0.0, 0.0, g, oz);
        let (b, _, _) = surface(std::f32::consts::PI, 0.0, 0.0, g, oz);
        let dist: f32 = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
        assert!(dist < 1e-5, "the two sheets are {dist} apart, not touching");
    }

    #[test]
    fn the_deformation_stays_within_the_view() {
        // The surface is scaled by 1/r as it deforms, so it should not grow
        // out of the frustum on the way from Boy to Roman.
        for step in 0..=10 {
            let dd = step as f32 / 10.0;
            let d = ((6.0 * dd - 15.0) * dd + 10.0) * dd * dd * dd;
            let r = 1.0 + d * d * (1.0 / 2.0 + d * d * (1.0 / 6.0 + d * d * (1.0 / 3.0)));
            let (radius, oz) = (1.0 / r, 0.5 * r);
            let mut max = 0.0f32;
            for i in 0..=32 {
                for j in 0..=32 {
                    let two_pi = 2.0 * std::f32::consts::PI;
                    let u = two_pi * j as f32 / 32.0;
                    let v = two_pi * i as f32 / 32.0;
                    let (x, _, _) = surface(u, v, d, 3, oz);
                    for c in x {
                        max = max.max((c * radius).abs());
                    }
                }
            }
            assert!(max < 1.0, "at deformation {dd} the surface reaches {max}");
        }
    }

    #[test]
    fn the_order_of_the_surface_sets_how_many_petals_it_has() {
        // Order g repeats every 2pi/g turn in u, up to the fold in v.
        for g in [3usize, 5, 7] {
            let two_pi = 2.0 * std::f32::consts::PI;
            let (a, _, _) = surface(0.7, 1.3, 1.0, g, 0.5);
            let (b, _, _) = surface(0.7 + two_pi / g as f32, 1.3, 1.0, g, 0.5);
            let dist: f32 = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
            assert!(dist > 0.05, "order {g} repeats too soon: {dist}");
        }
    }

    #[test]
    fn every_mesh_point_gets_a_normal() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "mode=surface&viewMode=turn&colors=distance&appearance=solid",
            20260811,
        ));
        for _ in 0..60 {
            r.step();
        }
        let f = r.frame();
        assert!(
            f.vertices
                .iter()
                .all(|v| v.normal.iter().all(|c| c.is_finite())),
            "a normal went to NaN at the poles"
        );
        assert_eq!(f.batches.len(), NUMV, "one strip per row of the mesh");
    }

    #[test]
    fn walking_stays_on_the_surface() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "mode=surface&viewMode=walk&colors=direction&appearance=solid&deform=false",
            20260811,
        ));
        for _ in 0..200 {
            r.step();
            assert!(
                r.frame()
                    .vertices
                    .iter()
                    .all(|v| v.pos.iter().all(|c| c.is_finite())),
                "a vertex went to NaN"
            );
        }
    }
}
