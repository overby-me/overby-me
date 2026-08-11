//! Port of `hacks/glx/etruscanvenus.c`.
//!
//! ```text
//! etruscanvenus --- Shows a 3d immersion of a Klein bottle that
//!   rotates in 3d or on which you can walk and that can deform smoothly
//!   between the Etruscan Venus surface, the Roman surface, the Boy
//!   surface surface, and the Ida surface.
//!
//! Copyright (c) 2019-2026 Carsten Steger <carsten@mirsanmir.org>.
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
//! A Klein bottle immersed in three dimensions, deforming around a loop
//! through four named surfaces: the Etruscan Venus, the Roman surface, Boy's
//! surface and the Ida surface. The deformation between them was constructed
//! by George Francis; the Etruscan Venus takes its name from a video by Donna
//! Cox, George Francis and Ray Idaszak shown at SIGGRAPH in 1989.
//!
//! All four are the same bottle, which is why the loop closes. Two of them,
//! Roman and Boy, are doubly covered, so they look like an immersed projective
//! plane instead: they are exactly what [`super::romanboy`] draws, with the
//! parameter square wrapped round twice.
//!
//! Two numbers control the whole family. One bends the surface and the other
//! pinches it, and the deformation just walks a square loop around the two of
//! them, which is what puts four surfaces on the tour rather than two.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape, TexEnv};
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

const NUMU: usize = 192;
const NUMV: usize = 128;
/// The period of the bands, in mesh lines. Direction and distance bands are
/// counted differently because the mesh is not square in the two directions.
const NUMBDIR: usize = 8;
const NUMBDIST: usize = 4;

/// How far below the walker's feet the camera sits.
const DELTAY: f32 = 0.01;
const DRHO: f32 = 0.7;
const DSIGMA: f32 = 1.1;
const DTAU: f32 = 1.7;

const TEX_DIMENSION: usize = 64;

/// Fitted constants for the centre of the surface in z. Upstream computed
/// them once, offline, so that the deforming surface stays in the middle of
/// the screen.
const Z1: f32 = 0.814_117_9;
const Z2: f32 = 0.135_927_69;
const Z3: f32 = 1.158_109_7;
const Z4: f32 = 0.718_654_9;
const Z5: f32 = 2.539_340_2;

/// The same, for its radius.
const R1: f32 = 1.308_007;
const R2: f32 = 4.005_206;
const R3: f32 = -2.893_994_6;
const R4: f32 = -1.266_709_5;

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

struct EtruscanVenus {
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
    walk_direction: f32,
    walk_speed: f32,

    /// Where we are on the four-stage loop, and which way round it is going.
    dd: f32,
    defdir: f32,
    deform_speed: f32,

    offset3d: [f32; 3],
    col: Vec<[f32; 4]>,
    tex: Vec<[f32; 2]>,
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

/// The surface written as a scale `f` times a direction, and the derivatives
/// of both. Split out because the degenerate case recomputes it at a nudged
/// parameter.
struct Terms {
    f: f32,
    fdir: [f32; 3],
    fu: f32,
    fv: f32,
    fdiru: [f32; 3],
    fdirv: [f32; 3],
}

fn terms(u: f32, v: f32, db: f32, dl: f32) -> Terms {
    let bosqrt2 = db / std::f32::consts::SQRT_2;
    let (su, cu) = (u.sin(), u.cos());
    let (s2u, c2u) = ((2.0 * u).sin(), (2.0 * u).cos());
    let (s3u, c3u) = ((3.0 * u).sin(), (3.0 * u).cos());
    let (sv, cv) = (v.sin(), v.cos());
    let (s2v, c2v) = ((2.0 * v).sin(), (2.0 * v).cos());
    let nom = 1.0 - dl + dl * cv;
    let den = 1.0 - bosqrt2 * s3u * s2v;
    let den2 = 1.0 / (den * den);
    let nomv = -dl * sv;
    let denu = -3.0 * bosqrt2 * c3u * s2v;
    let denv = -2.0 * bosqrt2 * s3u * c2v;
    Terms {
        f: nom / den,
        fdir: [
            c2u * cv + cu * sv,
            s2u * cv - su * sv,
            std::f32::consts::SQRT_2 * cv,
        ],
        fu: -nom * denu * den2,
        fv: (den * nomv - nom * denv) * den2,
        fdiru: [-su * sv - 2.0 * s2u * cv, 2.0 * c2u * cv - cu * sv, 0.0],
        fdirv: [
            cu * cv - c2u * sv,
            -s2u * sv - su * cv,
            -std::f32::consts::SQRT_2 * sv,
        ],
    }
}

fn derivatives(t: &Terms) -> ([f32; 3], [f32; 3]) {
    (
        std::array::from_fn(|l| t.fu * t.fdir[l] + t.f * t.fdiru[l]),
        std::array::from_fn(|l| t.fv * t.fdir[l] + t.f * t.fdirv[l]),
    )
}

/// The surface and its two tangents at one point of the parameter square.
/// `db` bends it and `dl` pinches it; the four named surfaces are the four
/// corners of that unit square.
fn surface(u: f32, v: f32, db: f32, dl: f32, oz: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
    // The parameter square is twice as long in u as the surface needs, which
    // is what doubly covers the Roman and Boy surfaces.
    let u = 0.5 * u;
    let t = terms(u, v, db, dl);
    let x = [t.f * t.fdir[0], t.f * t.fdir[1], t.f * t.fdir[2] - oz];
    let (du, dv) = derivatives(&t);

    // Avoid degenerate tangential plane basis vectors as much as possible: at
    // the pinch points the two tangents are parallel and there is no normal to
    // be had, so take the one a little way off instead.
    let n = cross(du, dv);
    if n[0] * n[0] + n[1] * n[1] + n[2] * n[2] < 10.0 * f32::EPSILON {
        let (du, dv) = derivatives(&terms(u + 0.01, v + 0.01, db, dl));
        return (x, du, dv);
    }
    (x, du, dv)
}

/// The two deformation parameters, and the centre and scale that keep the
/// result on screen, for a position `dd` on the four-stage loop.
fn deformation(dd: f32) -> (f32, f32, f32, f32) {
    let (bb, ll) = if dd < 1.0 {
        (0.0, dd)
    } else if dd < 2.0 {
        (dd - 1.0, 1.0)
    } else if dd < 3.0 {
        (1.0, 3.0 - dd)
    } else {
        (4.0 - dd, 0.0)
    };
    // A quintic, so the deformation eases in and out at each corner.
    let db = ((6.0 * bb - 15.0) * bb + 10.0) * bb * bb * bb;
    let dl = ((6.0 * ll - 15.0) * ll + 10.0) * ll * ll * ll;
    let oz = Z1
        * ((0.5 * std::f32::consts::PI * dl.powf(Z3)).sin()
            + Z2 * (1.5 * std::f32::consts::PI * dl.powf(Z3)).sin())
        * (Z4 * db.powf(Z5)).exp();
    let r = R1 + (db - 0.5) * (dl - 0.5) + R2 * (R3 * (1.0 - db)).exp() * (R4 * dl).exp();
    (db, dl, oz, 0.8 / r)
}

impl EtruscanVenus {
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

    /// The angle the colour wheel is read at, for a point of the mesh. The
    /// distance colours run up and back down again so that they meet where the
    /// parameter square wraps.
    fn angle_at(&self, u: f32, v: f32) -> f32 {
        if self.colors != Colors::Distance {
            return u;
        }
        let two_pi = 2.0 * std::f32::consts::PI;
        let mut vc = if self.appearance == Appearance::DistanceBands {
            -4.0 * v
        } else {
            4.0 * v
        };
        if vc >= 2.0 * two_pi {
            vc -= 2.0 * two_pi;
        }
        if vc >= two_pi {
            vc = 2.0 * two_pi - vc;
        }
        vc
    }

    /// `setup_etruscan_venus_color_texture`: the parts of the mesh the
    /// deformation does not touch.
    fn setup(&mut self) {
        let two_pi = 2.0 * std::f32::consts::PI;
        for i in 0..=NUMV {
            for j in 0..=NUMU {
                let k = i * (NUMU + 1) + j;
                let u = two_pi * j as f32 / NUMU as f32;
                let v = if self.appearance == Appearance::DistanceBands {
                    -two_pi * i as f32 / NUMV as f32
                } else {
                    two_pi * i as f32 / NUMV as f32
                };
                if !self.change_colors {
                    self.col[k] = self.color(self.angle_at(u, v), &rotateall(0.0, 0.0, 0.0));
                }
                self.tex[k][0] = 48.0 * u / two_pi;
                self.tex[k][1] = 64.0 * v / two_pi
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
    fn compute_walk_frame(&mut self, db: f32, dl: f32, radius: f32, oz: f32) -> [[f32; 3]; 3] {
        let (xx, xxu, xxv) = surface(self.umove, self.vmove, db, dl, oz);
        let mut pu = xxu;
        let mut pv = xxv;
        for l in 0..3 {
            pu[l] *= radius;
            pv[l] *= radius;
        }

        let mut n = cross(pu, pv);
        scale_to(&mut n, 1.0 / (self.side * 4.0));
        // The half on dumove is the chain rule for u, which the surface halves
        // before using it.
        let mut pm = std::array::from_fn(|l| 0.5 * pu[l] * self.dumove + pv[l] * self.dvmove);
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
        self.offset3d = [-p[0] * radius, -p[1] * radius - DELTAY, -p[2] * radius];
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
        "orthographic" => walking,
        _ => walking || random().is_multiple_of(2),
    };

    let n = (NUMU + 1) * (NUMV + 1);
    let walk_direction = g.res.float("walkDirection") as f32;
    let turning = !walking;
    let init_deform = g.res.float("initDeform") as f32;

    let mut this = EtruscanVenus {
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
        walk_direction,
        walk_speed: g.res.float("walkSpeed") as f32,
        dd: if (0.0..4000.0).contains(&init_deform) {
            init_deform * 0.001
        } else {
            0.0
        },
        defdir: 1.0,
        deform_speed: g.res.float("deformSpeed") as f32,
        offset3d: [0.0, 0.0, -2.0],
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
        .flat_map(|&v| [v, v, v, 255])
        .collect();
    g.glx
        .tex_image_2d(TEX_DIMENSION as i32, TEX_DIMENSION as i32, rgba);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for EtruscanVenus {
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
                    self.dd += 4.0;
                }
                if self.dd >= 4.0 {
                    self.dd -= 4.0;
                }
                // Randomly change the deformation direction at one of the four
                // surfaces in a tenth of the cases, so the tour is not always
                // the same way round.
                if (self.dd.round() - self.dd).abs() <= self.deform_speed * 0.0005
                    && random().is_multiple_of(10)
                {
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
                self.dumove = rad.cos() * self.walk_speed * std::f32::consts::PI / 4096.0;
                self.dvmove = rad.sin() * self.walk_speed * std::f32::consts::PI / 4096.0;
                self.umove += self.dumove;
                // A full turn in u comes back mirrored in v and on the other
                // side of the surface, which is what makes it a Klein bottle.
                if self.umove >= two_pi {
                    self.umove -= two_pi;
                    self.vmove = two_pi - self.vmove;
                    self.side = -self.side;
                }
                if self.umove < 0.0 {
                    self.umove += two_pi;
                    self.vmove = two_pi - self.vmove;
                    self.side = -self.side;
                }
                self.vmove += self.dvmove;
                if self.vmove >= two_pi {
                    self.vmove -= two_pi;
                }
                if self.vmove < 0.0 {
                    self.vmove += two_pi;
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

        let (db, dl, oz, radius) = deformation(self.dd);
        let color_mat = rotateall(self.rho, self.sigma, self.tau);

        let mat = if self.walking {
            self.compute_walk_frame(db, dl, radius, oz)
        } else {
            g.glx.mult_matrix(self.trackball.matrix());
            rotateall(self.alpha, self.beta, self.delta)
        };
        let offset3d = self.offset3d;

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
        let distance_bands = self.appearance == Appearance::DistanceBands;

        // Rebuild the surface. The deformation moves every point, so there is
        // nothing here to precompute.
        let point = |this: &mut Self, i: usize, j: usize| {
            let o = i * (NUMU + 1) + j;
            let u = two_pi * j as f32 / NUMU as f32;
            let v = if distance_bands {
                -two_pi * i as f32 / NUMV as f32
            } else {
                two_pi * i as f32 / NUMV as f32
            };
            if this.change_colors && per_vertex {
                let c = this.color(this.angle_at(u, v), &color_mat);
                this.col[o] = c;
            }
            let (xx, xxu, xxv) = surface(u, v, db, dl, oz);
            let p = apply(&mat, xx);
            let pu = apply(&mat, xxu);
            let pv = apply(&mat, xxv);
            this.pos[o] = std::array::from_fn(|l| p[l] * radius + offset3d[l]);
            let mut n = cross(pu, pv);
            scale_to(&mut n, 1.0);
            this.pnorm[o] = n;
        };

        if distance_bands {
            for i in 0..=NUMV {
                if (i & (NUMBDIST - 1)) > NUMBDIST / 4 && (i & (NUMBDIST - 1)) < 3 * NUMBDIST / 4 {
                    continue;
                }
                for j in 0..=NUMU {
                    point(self, i, j);
                }
            }
        } else {
            for j in 0..=NUMU {
                if self.appearance == Appearance::DirectionBands
                    && (j & (NUMBDIR - 1)) > NUMBDIR / 2
                {
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

        let mut strip: Vec<usize> = Vec::with_capacity(2 * (NUMU + 1));
        if distance_bands {
            for i in 0..NUMV {
                if (i & (NUMBDIST - 1)) >= NUMBDIST / 4 && (i & (NUMBDIST - 1)) < 3 * NUMBDIST / 4 {
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
        } else {
            for j in 0..NUMU {
                if self.appearance == Appearance::DirectionBands
                    && (j & (NUMBDIR - 1)) >= NUMBDIR / 2
                {
                    continue;
                }
                strip.clear();
                for i in 0..=NUMV {
                    for k in 0..=1 {
                        strip.push(i * (NUMU + 1) + j + k);
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
    "*changeColors:  True",
    "*deform:        True",
    "*projection:    random",
    "*speedx:        1.1",
    "*speedy:        1.3",
    "*speedz:        1.5",
    "*walkDirection: 83.0",
    "*walkSpeed:     20.0",
    "*deformSpeed:   10.0",
    "*initDeform:    0.0",
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
    Opt::boolean("marks", "Show orientation marks", "false"),
    Opt::boolean("deform", "Deform the Klein bottle", "true"),
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
        4000.0,
        1.0,
        0,
        "0.0",
    ),
    Opt::select("mode", "Display mode", MODES, "random"),
    Opt::select("appearance", "Appearance", APPEARANCES, "random"),
    Opt::select("colors", "Colors", COLOR_MODES, "random"),
    Opt::select("projection", "Projection", PROJ, "random"),
    Opt::boolean("changeColors", "Change colors", "true"),
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
    slug: "etruscanvenus",
    label: "Etruscan Venus",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Steger",
        year: "2020",
        video: Some("https://www.youtube.com/watch?v=p3MgGyie6-I"),
        blurb: "A 3d immersion of a Klein bottle that deforms between the \
                Etruscan Venus, Roman, Boy and Ida surfaces.",
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

    /// The four corners of the deformation square are the four named
    /// surfaces, and the tour visits each of them exactly once.
    #[test]
    fn the_loop_passes_through_four_surfaces_and_closes() {
        let corners: Vec<(f32, f32)> = (0..4)
            .map(|k| {
                let (db, dl, _, _) = deformation(k as f32);
                (db, dl)
            })
            .collect();
        assert_eq!(
            corners,
            vec![(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)],
            "the tour is not the square"
        );
        let (a, b, _, _) = deformation(0.0);
        let (c, d, _, _) = deformation(4.0 - 1e-6);
        assert!(
            (a - c).abs() < 1e-3 && (b - d).abs() < 1e-3,
            "the loop does not close"
        );
    }

    #[test]
    fn the_surface_is_a_klein_bottle() {
        // Going a full turn round u comes back to the same points, but
        // traversed the other way in v, which is the Klein bottle's gluing.
        let two_pi = 2.0 * std::f32::consts::PI;
        let (db, dl, oz, _) = deformation(1.5);
        for v in [0.4f32, 2.0, 5.1] {
            let (a, _, _) = surface(0.0, v, db, dl, oz);
            let (b, _, _) = surface(two_pi, two_pi - v, db, dl, oz);
            let dist: f32 = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
            assert!(dist < 1e-4, "at v={v} the ends are {dist} apart");
        }
    }

    #[test]
    fn the_fitted_scale_keeps_it_on_screen() {
        // The whole point of the fitted radius and centre is that no stage of
        // the deformation grows out of the frustum.
        for step in 0..40 {
            let dd = step as f32 / 10.0;
            let (db, dl, oz, radius) = deformation(dd);
            let mut max = 0.0f32;
            for i in 0..=24 {
                for j in 0..=24 {
                    let two_pi = 2.0 * std::f32::consts::PI;
                    let (u, v) = (two_pi * j as f32 / 24.0, two_pi * i as f32 / 24.0);
                    let (x, _, _) = surface(u, v, db, dl, oz);
                    for c in x {
                        max = max.max((c * radius).abs());
                    }
                }
            }
            assert!(max < 1.2, "at {dd} the surface reaches {max}");
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
        assert_eq!(
            strips("mode=surface&viewMode=turn&appearance=solid&deform=false"),
            NUMU,
            "the solid surface is one strip per column"
        );
        // Direction bands keep half of every group of eight columns.
        assert_eq!(
            strips("mode=surface&viewMode=turn&appearance=direction-bands&deform=false"),
            NUMU / 2
        );
        // Distance bands run the other way and keep half of every four rows.
        assert_eq!(
            strips("mode=surface&viewMode=turn&appearance=distance-bands&deform=false"),
            NUMV / 2
        );
    }

    #[test]
    fn walking_survives_the_pinch_points() {
        // The Roman and Boy stages have points where the normal is undefined;
        // the walker has to cross them without producing a NaN.
        let mut r = start(StartArgs::new(
            640,
            480,
            "mode=surface&viewMode=walk&colors=direction&appearance=solid&deformSpeed=100",
            20260811,
        ));
        for _ in 0..200 {
            r.step();
            let f = r.frame();
            assert!(
                f.vertices.iter().all(|v| v
                    .pos
                    .iter()
                    .chain(v.normal.iter())
                    .all(|c| c.is_finite())),
                "a vertex or normal went to NaN"
            );
        }
    }
}
