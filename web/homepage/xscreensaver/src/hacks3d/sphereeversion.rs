/* sphereeversion --- Shows a sphere eversion, i.e., a smooth deformation
(homotopy) that turns a sphere inside out.  During the eversion, the
deformed sphere is allowed to intersect itself transversally.  However,
no creases or pinch points are allowed to occur. */

/* Copyright (c) 2020-2026 Carsten Steger <carsten@mirsanmir.org>. */

/*
 * Permission to use, copy, modify, and distribute this software and its
 * documentation for any purpose and without fee is hereby granted,
 * provided that the above copyright notice appear in all copies and that
 * both that copyright notice and this permission notice appear in
 * supporting documentation.
 *
 * This file is provided AS IS with no warranties of any kind.  The author
 * shall have no liability with respect to the infringement of copyrights,
 * trade secrets or any patents by this file or any part thereof.  In no
 * event will the author be liable for any lost revenue or profits or
 * other special, indirect and consequential damages.
 */

//! Port of `hacks/glx/sphereeversion.c` and `sphereeversion-analytic.c`.
//!
//! Turns a sphere inside out: a smooth deformation (homotopy). During the
//! eversion the deformed sphere is allowed to pass through itself, but no
//! crease and no pinch point is ever allowed to form, which is what makes it
//! hard and what makes it worth watching.
//!
//! The surface is not modelled and not simulated. It is a closed-form formula
//! from Adam and Witold Bednorz, "Analytic sphere eversion using ruled
//! surfaces" (2019): a point of the sphere at longitude phi and latitude theta
//! is put through three maps in turn, and every stage of the eversion is one
//! value of a deformation parameter tau running from -6 to 6 and back. So a
//! frame is 257 by 257 evaluations of that formula, plus its two partial
//! derivatives, whose cross product is the surface normal. Nothing is
//! remembered between frames.
//!
//! Upstream has two eversions under this name, chosen at random: the analytic
//! one above, and the corrugations of the 1994 film "Outside In". This is the
//! analytic one; see the README for where the other has got to.
//!
//! Upstream draws it two ways: a vertex shader that evaluates the formula on
//! the card, and a fixed-function path that evaluates it on the CPU. Unlike
//! `timetunnel`, whose fallback was a stub, this one is the whole saver, so it
//! is what is ported.
//!
//! The one thing the fallback cannot do is the earth colouring, which upstream
//! wraps day, night and water textures around the sphere in a fragment shader.
//! Its fixed-function path quietly draws the plain two-sided red and green
//! instead, and so does this.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

/* Shape parameters for the Bednorz sphere eversion. */
const BEDNORZ_OMEGA: f64 = 2.0;
const BEDNORZ_Q: f64 = 2.0 / 3.0;
const BEDNORZ_ETA_MIN: f64 = 3.0 / 4.0;
const BEDNORZ_BETA_MAX: f64 = 0.1;
const BEDNORZ_ALPHA: f64 = 1.0;
const BEDNORZ_EPS2: f64 = 0.001;
const BEDNORZ_EPS3: f64 = 0.002;
const BEDNORZ_EPS4: f64 = 0.001;
const BEDNORZ_EPS5: f64 = 0.002;
const BEDNORZ_TAU1: f64 = 1.0 / BEDNORZ_Q;
const BEDNORZ_TAU2: f64 = 2.5;
const BEDNORZ_TAU3: f64 = 4.5;
const BEDNORZ_TAU4: f64 = 6.0;
const BEDNORZ_TAU_MIN: f32 = -BEDNORZ_TAU4 as f32;
const BEDNORZ_TAU_MAX: f32 = BEDNORZ_TAU4 as f32;

/// Number of subdivisions of the surface.
///
/// Upstream's 256 by 256, which is 66049 evaluations of the Bednorz formula a
/// frame. It is the same arithmetic upstream's own fixed-function path does;
/// its shader path hands it to the card instead.
const NUMTH: usize = 256;
const NUMPH: usize = 256;
/* Number of subdivisions between grid lines */
const NUMGRID: usize = 32;
/* Number of subdivisions per band */
const NUMBDIR: usize = 16;
const NUMBDIST: usize = 16;

/// Angle of a single turn step.
const TURN_STEP: f32 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayMode {
    Surface,
    Transparent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Appearance {
    Solid,
    ParallelBands,
    MeridianBands,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Colors {
    TwoSided,
    Parallel,
    Meridian,
    Earth,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Projection {
    Perspective,
    Orthographic,
}

/// Which half of the animation is running: the sphere is turning itself inside
/// out, or it is turning on the spot between two eversions.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimState {
    Deform,
    Turn,
}

/// The shape parameters of one instant of the eversion, all of them functions
/// of the deformation parameter tau.
#[derive(Clone, Copy, Default)]
struct ShapePar {
    n: i32,
    kappa: f64,
    omega: f64,
    t: f64,
    p: f64,
    q: f64,
    xi: f64,
    eta: f64,
    alpha: f64,
    beta: f64,
    gamma: f64,
    lambda: f64,
    eps: f64,
}

/* -------------------------------------------------------------------------
 * Rotations, quaternions and colours: upstream's sphereeversion.c
 * ---------------------------------------------------------------------- */

/// Add a rotation around the x-axis to the matrix m.
fn rotatex(m: &mut [[f32; 3]; 3], phi: f32) {
    let phi = phi.to_radians();
    let (s, c) = phi.sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[1], row[2]);
        row[1] = c * u + s * v;
        row[2] = -s * u + c * v;
    }
}

/// Add a rotation around the y-axis to the matrix m.
fn rotatey(m: &mut [[f32; 3]; 3], phi: f32) {
    let phi = phi.to_radians();
    let (s, c) = phi.sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[0], row[2]);
        row[0] = c * u - s * v;
        row[2] = s * u + c * v;
    }
}

/// Add a rotation around the z-axis to the matrix m.
fn rotatez(m: &mut [[f32; 3]; 3], phi: f32) {
    let phi = phi.to_radians();
    let (s, c) = phi.sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[0], row[1]);
        row[0] = c * u + s * v;
        row[1] = -s * u + c * v;
    }
}

/// Compute the rotation matrix from the rotation angles.
fn rotateall(al: f32, be: f32, de: f32) -> [[f32; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    rotatex(&mut m, al);
    rotatey(&mut m, be);
    rotatez(&mut m, de);
    m
}

/// Multiply two rotation matrices: `m * n`.
fn mult_rotmat(m: &[[f32; 3]; 3], n: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                o[i][j] += m[i][k] * n[k][j];
            }
        }
    }
    o
}

/// Compute 3D rotation angles from a unit quaternion.
fn quat_to_angles(q: [f32; 4]) -> (f32, f32, f32) {
    let r00 = q[0] * q[0] + q[1] * q[1] - q[2] * q[2] - q[3] * q[3];
    let r01 = 2.0 * (q[1] * q[2] - q[0] * q[3]);
    let r02 = 2.0 * (q[1] * q[3] + q[0] * q[2]);
    let r12 = 2.0 * (q[2] * q[3] - q[0] * q[1]);
    let r22 = q[0] * q[0] - q[1] * q[1] - q[2] * q[2] + q[3] * q[3];
    (
        (-r12).atan2(r22),
        r02.atan2((r00 * r00 + r01 * r01).sqrt()),
        (-r01).atan2(r00),
    )
}

/// Compute a quaternion from angles in degrees.
fn angles_to_quat(alpha: f32, beta: f32, delta: f32) -> [f32; 4] {
    let (sa, ca) = (0.5 * alpha.to_radians()).sin_cos();
    let (sb, cb) = (0.5 * beta.to_radians()).sin_cos();
    let (sd, cd) = (0.5 * delta.to_radians()).sin_cos();
    [
        ca * cb * cd - sa * sb * sd,
        sa * cb * cd + ca * sb * sd,
        ca * sb * cd - sa * cb * sd,
        ca * cb * sd + sa * sb * cd,
    ]
}

/// Perform a spherical linear interpolation between two quaternions.
fn quat_slerp(t: f32, qs: [f32; 4], qe: [f32; 4]) -> [f32; 4] {
    let mut alpha = f64::from(t);
    let cos_t = f64::from(qs[0] * qe[0] + qs[1] * qe[1] + qs[2] * qe[2] + qs[3] * qe[3]);
    let beta;
    if 1.0 - cos_t < f64::from(f32::EPSILON) {
        beta = 1.0 - alpha;
    } else {
        let theta = cos_t.acos();
        let sin_t = theta.sin();
        beta = (theta - alpha * theta).sin() / sin_t;
        alpha = (alpha * theta).sin() / sin_t;
    }
    let mut q = [0.0f32; 4];
    for i in 0..4 {
        q[i] = (beta * f64::from(qs[i]) + alpha * f64::from(qe[i])) as f32;
    }
    let l = 1.0 / (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    for v in &mut q {
        *v *= l;
    }
    q
}

/// Compute a 3D rotation matrix from an xscreensaver unit quaternion. Note
/// that xscreensaver has a different convention for unit quaternions than the
/// one that is used in this hack.
fn quat_to_rotmat(p: [f32; 4]) -> [[f32; 3]; 3] {
    let r00 = 1.0 - 2.0 * (p[1] * p[1] + p[2] * p[2]);
    let r01 = 2.0 * (p[0] * p[1] + p[2] * p[3]);
    let r02 = 2.0 * (p[2] * p[0] - p[1] * p[3]);
    let r12 = 2.0 * (p[1] * p[2] + p[0] * p[3]);
    let r22 = 1.0 - 2.0 * (p[1] * p[1] + p[0] * p[0]);
    let al = (-r12).atan2(r22).to_degrees();
    let be = r02.atan2((r00 * r00 + r01 * r01).sqrt()).to_degrees();
    let de = (-r01).atan2(r00).to_degrees();
    rotateall(al, be, de)
}

/// Compute a fully saturated and bright colour based on an angle and a colour
/// rotation matrix.
fn color(angle: f32, mat: &[[f32; 3]; 3], transparent: bool) -> ([f32; 4], [f32; 4]) {
    let (sa, ca) = angle.sin_cos();
    let alpha = if transparent { 0.7 } else { 1.0 };

    let mut colf = [0.0f32; 4];
    for i in 0..3 {
        colf[i] = ca * mat[i][0] + sa * mat[i][2];
    }
    let m = 0.5 / colf[0].abs().max(colf[1].abs()).max(colf[2].abs());
    for c in colf.iter_mut().take(3) {
        *c = m * *c + 0.5;
    }
    colf[3] = alpha;

    let mut colb = [0.0f32; 4];
    for i in 0..3 {
        colb[i] = -ca * mat[i][1] - sa * mat[i][2];
    }
    let m = 0.5 / colb[0].abs().max(colb[1].abs()).max(colb[2].abs());
    for c in colb.iter_mut().take(3) {
        *c = m * *c + 0.5;
    }
    colb[3] = alpha;

    (colf, colb)
}

/* -------------------------------------------------------------------------
 * The Bednorz eversion
 * ---------------------------------------------------------------------- */

/// `x^n` for small integer `n`. Upstream spells out the squarings; `powi` does
/// the same thing.
fn ipow(x: f64, n: i32) -> f64 {
    x.powi(n)
}

fn bednorz_get_kappa(n: i32) -> f64 {
    (f64::from(n) - 1.0) / (2.0 * f64::from(n))
}

fn bednorz_get_t(tau: f64) -> f64 {
    tau.clamp(-BEDNORZ_TAU1, BEDNORZ_TAU1)
}

fn bednorz_get_q(tau: f64) -> f64 {
    let abs_tau = tau.abs();
    if abs_tau < BEDNORZ_TAU1 {
        0.0
    } else if abs_tau < BEDNORZ_TAU2 {
        BEDNORZ_Q * (abs_tau - BEDNORZ_TAU1) / (BEDNORZ_TAU2 - BEDNORZ_TAU1)
    } else {
        BEDNORZ_Q
    }
}

fn bednorz_get_p(tau: f64) -> f64 {
    1.0 - (bednorz_get_q(tau) * bednorz_get_t(tau)).abs()
}

fn bednorz_get_xi(tau: f64) -> f64 {
    let abs_tau = tau.abs();
    if abs_tau < BEDNORZ_TAU2 {
        1.0
    } else if abs_tau < BEDNORZ_TAU3 {
        (BEDNORZ_TAU3 - abs_tau) / (BEDNORZ_TAU3 - BEDNORZ_TAU2)
    } else {
        0.0
    }
}

fn bednorz_get_eta(tau: f64, eta_min: f64) -> f64 {
    let abs_tau = tau.abs();
    if abs_tau < BEDNORZ_TAU2 {
        eta_min
    } else if abs_tau < BEDNORZ_TAU3 {
        eta_min + (1.0 - eta_min) * (abs_tau - BEDNORZ_TAU2) / (BEDNORZ_TAU3 - BEDNORZ_TAU2)
    } else {
        1.0
    }
}

fn bednorz_get_alpha(tau: f64) -> f64 {
    BEDNORZ_ALPHA * ipow(bednorz_get_xi(tau), 2)
}

fn bednorz_get_beta(tau: f64, beta_max: f64) -> f64 {
    let xi = bednorz_get_xi(tau);
    ipow(1.0 - xi, 2) + beta_max * ipow(xi, 3)
}

fn bednorz_get_gamma(alpha: f64, beta: f64) -> f64 {
    2.0 * (alpha * beta).sqrt()
}

fn bednorz_get_lambda(tau: f64) -> f64 {
    let abs_tau = tau.abs();
    if abs_tau < BEDNORZ_TAU3 {
        1.0
    } else if abs_tau < BEDNORZ_TAU4 {
        (BEDNORZ_TAU4 - abs_tau) / (BEDNORZ_TAU4 - BEDNORZ_TAU3)
    } else {
        0.0
    }
}

/// This is an extension to the original approach that prevents z fighting to
/// some extent in certain stages of the eversion.
fn bednorz_get_eps(tau: f64, n: i32) -> f64 {
    let sgn_tau = if tau < 0.0 {
        -1.0
    } else if tau > 0.0 {
        1.0
    } else {
        0.0
    };
    let abs_tau = tau.abs();
    // The ramp down to nothing at the very end of the deformation is shared by
    // all four orders.
    let tail = |eps: f64| {
        if abs_tau < BEDNORZ_TAU3 {
            eps * sgn_tau
        } else if abs_tau < BEDNORZ_TAU4 {
            eps * sgn_tau * (BEDNORZ_TAU4 - abs_tau) / (BEDNORZ_TAU4 - BEDNORZ_TAU3)
        } else {
            0.0
        }
    };
    match n {
        // Order two holds at nothing until tau1 and then ramps up, where the
        // others start moving at once.
        2 => {
            if abs_tau < BEDNORZ_TAU1 {
                0.0
            } else if abs_tau < BEDNORZ_TAU2 {
                BEDNORZ_EPS2 * sgn_tau * (abs_tau - BEDNORZ_TAU1) / (BEDNORZ_TAU2 - BEDNORZ_TAU1)
            } else {
                tail(BEDNORZ_EPS2)
            }
        }
        3..=5 => {
            let eps = match n {
                3 => BEDNORZ_EPS3,
                4 => BEDNORZ_EPS4,
                _ => BEDNORZ_EPS5,
            };
            if abs_tau < BEDNORZ_TAU1 {
                eps * sgn_tau * abs_tau / BEDNORZ_TAU1
            } else {
                tail(eps)
            }
        }
        _ => 0.0,
    }
}

/// Equations (4), (12) and (15) in the paper: a point and its two partial
/// derivatives.
fn bednorz_get_p0(
    phi: f64,
    theta: f64,
    bsp: &ShapePar,
    x: &mut [f64; 3],
    dxdph: &mut [f64; 3],
    dxdth: &mut [f64; 3],
) {
    let n = bsp.n;
    let nf = f64::from(n);
    let (kappa, omega, t, p, q, eta, lambda) = (
        bsp.kappa, bsp.omega, bsp.t, bsp.p, bsp.q, bsp.eta, bsp.lambda,
    );

    let (st, ct) = theta.sin_cos();
    let (sp, cp) = phi.sin_cos();
    let (snp, cnp) = (nf * phi).sin_cos();
    let ctn = ipow(ct, n);
    let ictn = 1.0 / ctn;
    let ictnp1 = ictn / ct;
    let ct2 = ct * ct;
    let st2 = st * st;
    let ton = t / nf;
    let snpmqt = snp - q * t;
    let ost = omega * st;

    if lambda >= 1.0 {
        let (snm1p, cnm1p) = ((nf - 1.0) * phi).sin_cos();
        let nst2pct2 = nf * st2 + ct2;
        let nm1p = (nf - 1.0) * p;
        let tcp = t * cp;
        let tsp = t * sp;
        let ostictn = ost * ictn;
        let oictnp1 = omega * ictnp1;
        let nst2pct2oictnp1 = nst2pct2 * oictnp1;
        x[0] = p * snm1p - sp * ostictn + tcp;
        x[1] = p * cnm1p + cp * ostictn + tsp;
        x[2] = snpmqt * ostictn - ton * cnp;
        dxdph[0] = nm1p * cnm1p - cp * ostictn - tsp;
        dxdph[1] = -nm1p * snm1p - sp * ostictn + tcp;
        dxdph[2] = nf * cnp * ostictn + t * snp;
        dxdth[0] = -sp * nst2pct2oictnp1;
        dxdth[1] = cp * nst2pct2oictnp1;
        dxdth[2] = snpmqt * nst2pct2oictnp1;
    } else {
        let ct2n = ipow(ct, 2 * n);
        let ict2n = 1.0 / ct2n;
        let ict2np1 = ict2n / ct;
        let oml = 1.0 - lambda;
        let omlplctn = oml + lambda * ctn;
        let pe1pk = eta.powf(1.0 + kappa);
        let tat2k = t * t.abs().powf(2.0 * kappa);
        let lost = lambda * ost;
        let lostcp = lost * cp;
        let lostsp = lost * sp;
        let tomlplctn = t * omlplctn;
        let tomlplctncp = tomlplctn * cp;
        let tomlplctnsp = tomlplctn * sp;
        let tomlplctncpmlostsp = tomlplctncp - lostsp;
        let tomlplctnspplostcp = tomlplctnsp + lostcp;
        let ntctnst = nf * t * ctn * st;
        let oct2 = omega * ct2;
        let omlpe1pktat2k = oml * pe1pk * tat2k;
        let nst2 = nf * st2;
        x[0] = tomlplctncpmlostsp * ictn;
        x[1] = tomlplctnspplostcp * ictn;
        x[2] = lambda * (ost * snpmqt * ictn - ton * cnp) - omlpe1pktat2k * st * ict2n;
        dxdph[0] = -tomlplctnspplostcp * ictn;
        dxdph[1] = tomlplctncpmlostsp * ictn;
        dxdph[2] = lambda * (omega * nf * st * cnp * ictn + t * snp);
        dxdth[0] = (nf * tomlplctncpmlostsp * st - lambda * (ntctnst * cp + oct2 * sp)) * ictnp1;
        dxdth[1] = (nf * tomlplctnspplostcp * st - lambda * (ntctnst * sp - oct2 * cp)) * ictnp1;
        dxdth[2] = lambda * omega * snpmqt * (nst2 + ct2) * ictnp1
            - omlpe1pktat2k * (2.0 * nst2 + ct2) * ict2np1;
    }
}

/// The second map of the three.
fn bednorz_get_p1(
    phi: f64,
    theta: f64,
    bsp: &ShapePar,
    y: &mut [f64; 3],
    dydph: &mut [f64; 3],
    dydth: &mut [f64; 3],
) {
    let (kappa, xi, eta) = (bsp.kappa, bsp.xi, bsp.eta);
    let (mut x, mut dxdph, mut dxdth) = ([0.0; 3], [0.0; 3], [0.0; 3]);
    bednorz_get_p0(phi, theta, bsp, &mut x, &mut dxdph, &mut dxdth);

    let (x0, x1, x2) = (x[0], x[1], x[2]);
    let x02px12 = x0 * x0 + x1 * x1;
    let ex02px12 = eta * x02px12;
    let xipex02px12 = xi + ex02px12;
    let ixipex02px122 = 1.0 / (xipex02px12 * xipex02px12);
    let ixipex02px12k = 1.0 / xipex02px12.powf(kappa);
    let ixipex02px12kp1 = ixipex02px12k / xipex02px12;
    let x0dx0dphpx1dx1dph = x0 * dxdph[0] + x1 * dxdph[1];
    let x0dx0dthpx1dx1dth = x0 * dxdth[0] + x1 * dxdth[1];
    let tex0dx0dphpx1dx1dph = 2.0 * eta * x0dx0dphpx1dx1dph;
    let tex0dx0dthpx1dx1dth = 2.0 * eta * x0dx0dthpx1dx1dth;
    let ktex0dx0dphpx1dx1dph = kappa * tex0dx0dphpx1dx1dph;
    let ktex0dx0dthpx1dx1dth = kappa * tex0dx0dthpx1dx1dth;

    y[0] = x0 * ixipex02px12k;
    y[1] = x1 * ixipex02px12k;
    y[2] = x2 / xipex02px12;
    dydph[0] = (dxdph[0] * xipex02px12 - ktex0dx0dphpx1dx1dph * x0) * ixipex02px12kp1;
    dydph[1] = (dxdph[1] * xipex02px12 - ktex0dx0dphpx1dx1dph * x1) * ixipex02px12kp1;
    dydph[2] = (dxdph[2] * xipex02px12 - tex0dx0dphpx1dx1dph * x2) * ixipex02px122;
    dydth[0] = (dxdth[0] * xipex02px12 - ktex0dx0dthpx1dx1dth * x0) * ixipex02px12kp1;
    dydth[1] = (dxdth[1] * xipex02px12 - ktex0dx0dthpx1dx1dth * x1) * ixipex02px12kp1;
    dydth[2] = (dxdth[2] * xipex02px12 - tex0dx0dthpx1dx1dth * x2) * ixipex02px122;
}

/// Equations (8) and (9) in the paper: the third and last map.
fn bednorz_get_p2(
    phi: f64,
    theta: f64,
    bsp: &ShapePar,
    z: &mut [f64; 3],
    dzdph: &mut [f64; 3],
    dzdth: &mut [f64; 3],
) {
    let (alpha, beta, gamma) = (bsp.alpha, bsp.beta, bsp.gamma);
    let (mut y, mut dydph, mut dydth) = ([0.0; 3], [0.0; 3], [0.0; 3]);
    bednorz_get_p1(phi, theta, bsp, &mut y, &mut dydph, &mut dydth);

    let (y0, y1, y2) = (y[0], y[1], y[2]);
    let y02py12 = y0 * y0 + y1 * y1;
    let y0dy0dphpy1dy1dph = y0 * dydph[0] + y1 * dydph[1];
    let y0dy0dthpy1dy1dth = y0 * dydth[0] + y1 * dydth[1];

    // At the two poles the maps above are singular, and the partial
    // derivatives are parallel and so give no useful normal. Both are
    // special-cased.
    let north = (theta - std::f64::consts::FRAC_PI_2).abs() <= 1.0e-4;
    let south = (theta + std::f64::consts::FRAC_PI_2).abs() <= 1.0e-4;

    if alpha > 0.0 {
        if north || south {
            z[0] = 0.0;
            z[1] = 0.0;
            z[2] = -(alpha / beta).sqrt() / (alpha + beta);
            *dzdph = [1.0, 0.0, 0.0];
            *dzdth = [0.0, if north { 1.0 } else { -1.0 }, 0.0];
            return;
        }
        let egy2 = (gamma * y2).exp();
        let apby02py12 = alpha + beta * y02py12;
        let amby02py12 = alpha - beta * y02py12;
        let iapby02py12 = 1.0 / apby02py12;
        let iapby02py122 = iapby02py12 * iapby02py12;
        let igapby02py12 = iapby02py12 / gamma;
        let igapby02py122 = igapby02py12 * igapby02py12;
        let ambogapb = (alpha - beta) / (gamma * (alpha + beta));
        let egy2apby02py12 = egy2 * apby02py12;
        let egy2amby02py12 = egy2 * amby02py12;
        let tbegy2 = 2.0 * beta * egy2;
        let tbegy2y0dy0dphpy1dy1dph = tbegy2 * y0dy0dphpy1dy1dph;
        let tbegy2y0dy0dthpy1dy1dth = tbegy2 * y0dy0dthpy1dy1dth;
        let gigapby02py122 = gamma * igapby02py122;
        let gegy2amby02py12 = gamma * egy2amby02py12;

        z[0] = y0 * egy2 * iapby02py12;
        z[1] = y1 * egy2 * iapby02py12;
        z[2] = egy2amby02py12 * igapby02py12 - ambogapb;
        dzdph[0] = ((y0 * gamma * dydph[2] + dydph[0]) * egy2apby02py12
            - y0 * tbegy2y0dy0dphpy1dy1dph)
            * iapby02py122;
        dzdph[1] = ((y1 * gamma * dydph[2] + dydph[1]) * egy2apby02py12
            - y1 * tbegy2y0dy0dphpy1dy1dph)
            * iapby02py122;
        dzdph[2] = ((gegy2amby02py12 * dydph[2] - tbegy2y0dy0dphpy1dy1dph) * apby02py12
            - tbegy2y0dy0dphpy1dy1dph * amby02py12)
            * gigapby02py122;
        dzdth[0] = ((y0 * gamma * dydth[2] + dydth[0]) * egy2apby02py12
            - y0 * tbegy2y0dy0dthpy1dy1dth)
            * iapby02py122;
        dzdth[1] = ((y1 * gamma * dydth[2] + dydth[1]) * egy2apby02py12
            - y1 * tbegy2y0dy0dthpy1dy1dth)
            * iapby02py122;
        dzdth[2] = ((gegy2amby02py12 * dydth[2] - tbegy2y0dy0dthpy1dy1dth) * apby02py12
            - tbegy2y0dy0dthpy1dy1dth * amby02py12)
            * gigapby02py122;
    } else {
        let iy02py12 = 1.0 / y02py12;
        let iy02py122 = iy02py12 * iy02py12;
        let ty0dy0dphpy1dy1dph = 2.0 * y0dy0dphpy1dy1dph;
        let ty0dy0dthpy1dy1dth = 2.0 * y0dy0dthpy1dy1dth;

        z[0] = y0 * iy02py12;
        z[1] = y1 * iy02py12;
        z[2] = -y2;

        if north || south {
            *dzdph = [1.0, 0.0, 0.0];
            *dzdth = [0.0, if north { 1.0 } else { -1.0 }, 0.0];
            return;
        }
        dzdph[0] = (dydph[0] * y02py12 - y0 * ty0dy0dphpy1dy1dph) * iy02py122;
        dzdph[1] = (dydph[1] * y02py12 - y1 * ty0dy0dphpy1dy1dph) * iy02py122;
        dzdph[2] = -dydph[2];
        dzdth[0] = (dydth[0] * y02py12 - y0 * ty0dy0dthpy1dy1dth) * iy02py122;
        dzdth[1] = (dydth[1] * y02py12 - y1 * ty0dy0dthpy1dy1dth) * iy02py122;
        dzdth[2] = -dydth[2];
    }
}

/// A point of the surface and the normal there.
fn bednorz_point_normal(phi: f64, theta: f64, bsp: &ShapePar) -> ([f32; 3], [f32; 3]) {
    let (mut z, mut dzdph, mut dzdth) = ([0.0; 3], [0.0; 3], [0.0; 3]);
    bednorz_get_p2(phi, theta, bsp, &mut z, &mut dzdph, &mut dzdth);

    let mut p = [z[0] as f32, z[1] as f32, z[2] as f32];
    let a = [dzdph[0] as f32, dzdph[1] as f32, dzdph[2] as f32];
    let b = [dzdth[0] as f32, dzdth[1] as f32, dzdth[2] as f32];

    // In the original version of the Bednorz sphere eversion, the regions
    // around the north and south poles are deformed to points that lie very
    // close together, which fights for the depth buffer. The shape is nudged
    // very slightly apart to ameliorate that.
    if bsp.lambda == 1.0 {
        p[2] += (bsp.eps * theta.sin()) as f32;
    }

    let mut n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let t = 1.0 / (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    for v in &mut n {
        *v *= t;
    }
    (p, n)
}

/* -------------------------------------------------------------------------
 * The saver
 * ---------------------------------------------------------------------- */

/// How the colour of a band is worked out from its angle. Upstream's `matc`.
const MATC: [[f32; 3]; 3] = [
    [0.577_350_3, -0.577_350_3, 0.577_350_3],
    [0.211_324_87, 0.788_675_1, 0.577_350_3],
    [-0.788_675_1, -0.211_324_87, 0.577_350_3],
];

const LIGHT_POSITION: [f32; 4] = [1.0, 1.0, 1.0, 0.0];
const MAT_DIFF_FRONT: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const MAT_DIFF_BACK: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const MAT_DIFF_TRANS_FRONT: [f32; 4] = [1.0, 0.0, 0.0, 0.7];
const MAT_DIFF_TRANS_BACK: [f32; 4] = [0.0, 1.0, 0.0, 0.7];

struct SphereEversionState {
    display_mode: DisplayMode,
    random_display_mode: bool,
    appearance: Appearance,
    random_appearance: bool,
    colors: Colors,
    random_colors: bool,
    projection: Projection,
    graticule: bool,
    random_graticule: bool,
    /// The order of the eversion, upstream's `g`.
    g: i32,
    random_g: bool,

    /* 3D rotation angles */
    alpha: f32,
    beta: f32,
    delta: f32,
    anim_state: AnimState,
    /// Deformation parameter.
    tau: f32,
    defdir: f32,
    turn_step: i32,
    num_turn: i32,
    qs: [f32; 4],
    qe: [f32; 4],
    /* Two global shape parameters of the analytic sphere eversion */
    eta_min: f64,
    beta_max: f64,
    offset3d: [f32; 3],

    /* The 3d coordinates of the surface and the corresponding normals */
    sp: Vec<f32>,
    sn: Vec<f32>,
    /* The precomputed colours of the surface */
    colf: Vec<f32>,
    colb: Vec<f32>,

    aspect: f32,
    trackball: Trackball,
    speed_scale: f32,
    speed_x: f32,
    speed_y: f32,
    speed_z: f32,
    deform_speed: f32,
}

/// The index of the sample at (phi step `j`, theta step `i`).
fn at(j: usize, i: usize) -> usize {
    j * (NUMTH + 1) + i
}

impl SphereEversionState {
    /// `setup_surface_colors`: the colour of every sample, for the colourings
    /// that vary over the surface.
    fn setup_surface_colors(&mut self) {
        if self.colors == Colors::TwoSided || self.colors == Colors::Earth {
            return;
        }
        let transparent = self.display_mode == DisplayMode::Transparent;
        let phi_range = 2.0 * std::f32::consts::PI;
        let theta_range = std::f32::consts::PI;
        for j in 0..=NUMPH {
            let phi = phi_range * j as f32 / NUMPH as f32 - std::f32::consts::PI;
            for i in 0..=NUMTH {
                let o = at(j, i);
                let theta = theta_range * i as f32 / NUMTH as f32 - std::f32::consts::FRAC_PI_2;
                let angle = if self.colors == Colors::Parallel {
                    (2.0 * theta + 3.0 * std::f32::consts::PI / 4.0) * (2.0 / 3.0)
                } else {
                    phi
                };
                let (cf, cb) = color(angle, &MATC, transparent);
                self.colf[4 * o..4 * o + 4].copy_from_slice(&cf);
                self.colb[4 * o..4 * o + 4].copy_from_slice(&cb);
            }
        }
    }

    /// Evaluate the whole surface at the current tau, then centre and scale it.
    fn compute_surface(&mut self) {
        let mut tau = f64::from(self.tau);
        /* Apply easing functions to the different ranges of tau. */
        if tau.abs() <= BEDNORZ_TAU4 {
            let (tau_min, tau_max) = if tau.abs() <= BEDNORZ_TAU1 {
                (0.0, BEDNORZ_TAU1)
            } else if tau.abs() <= BEDNORZ_TAU2 {
                (BEDNORZ_TAU1, BEDNORZ_TAU2)
            } else if tau.abs() <= BEDNORZ_TAU3 {
                (BEDNORZ_TAU2, BEDNORZ_TAU3)
            } else {
                (BEDNORZ_TAU3, BEDNORZ_TAU4)
            };
            let e = 1.0 / (tau_min * tau_min - 2.0 * tau_min * tau_max + tau_max * tau_max);
            let a = -2.0 * e;
            let b = 3.0 * (tau_min + tau_max) * e;
            let c = -6.0 * tau_min * tau_max * e;
            let d = tau_min * tau_max * (tau_min + tau_max) * e;
            tau = if tau >= 0.0 {
                ((a * tau + b) * tau + c) * tau + d
            } else {
                ((a * tau - b) * tau + c) * tau - d
            };
        }

        let n = self.g;
        let alpha = bednorz_get_alpha(tau);
        let beta = bednorz_get_beta(tau, self.beta_max);
        let bsp = ShapePar {
            n,
            kappa: bednorz_get_kappa(n),
            omega: BEDNORZ_OMEGA,
            t: bednorz_get_t(tau),
            p: bednorz_get_p(tau),
            q: bednorz_get_q(tau),
            xi: bednorz_get_xi(tau),
            eta: bednorz_get_eta(tau, self.eta_min),
            alpha,
            beta,
            gamma: bednorz_get_gamma(alpha, beta),
            lambda: bednorz_get_lambda(tau),
            eps: bednorz_get_eps(tau, n),
        };

        /* Compute the surface points and normals. */
        let phi_range = 2.0 * std::f64::consts::PI;
        let theta_range = std::f64::consts::PI;
        for j in 0..=NUMPH {
            let phi = phi_range * j as f64 / NUMPH as f64 - std::f64::consts::PI;
            for i in 0..=NUMTH {
                let o = at(j, i);
                let theta = theta_range * i as f64 / NUMTH as f64 - std::f64::consts::FRAC_PI_2;
                let (p, nv) = bednorz_point_normal(phi, theta, &bsp);
                self.sp[3 * o..3 * o + 3].copy_from_slice(&p);
                self.sn[3 * o..3 * o + 3].copy_from_slice(&nv);
            }
        }

        /* Compute the z offset. */
        let mut zmin = f32::MAX;
        let mut zmax = -f32::MAX;
        for o in 0..(NUMPH + 1) * (NUMTH + 1) {
            let z = self.sp[3 * o + 2];
            zmin = zmin.min(z);
            zmax = zmax.max(z);
        }
        let offset_z = -0.5 * (zmin + zmax);

        /* Shift the surface in the z direction and compute the scale. */
        let mut rmax = -f32::MAX;
        for o in 0..(NUMPH + 1) * (NUMTH + 1) {
            self.sp[3 * o + 2] += offset_z;
            let (x, y, z) = (self.sp[3 * o], self.sp[3 * o + 1], self.sp[3 * o + 2]);
            rmax = rmax.max(x * x + y * y + z * z);
        }
        let scale = 0.75 / rmax.sqrt();

        /* Scale the surface. */
        for v in &mut self.sp {
            *v *= scale;
        }
    }

    /// One sample, turned into place.
    fn placed(&self, mat: &[[f32; 3]; 3], o: usize) -> ([f32; 3], [f32; 3]) {
        let xx = &self.sp[3 * o..3 * o + 3];
        let xn = &self.sn[3 * o..3 * o + 3];
        let mut p = [0.0f32; 3];
        let mut n = [0.0f32; 3];
        for l in 0..3 {
            let mut r = 0.0;
            let mut s = 0.0;
            for m in 0..3 {
                r += mat[l][m] * xx[m];
                s += mat[l][m] * xn[m];
            }
            p[l] = r + self.offset3d[l];
            n[l] = s;
        }
        (p, n)
    }

    /// Draw the surface, either once or, when the two sides are different
    /// colours at every vertex, once for each side.
    ///
    /// Upstream draws it once with culling off and two-sided lighting, setting
    /// `GL_FRONT` and `GL_BACK` materials *per vertex*, which real GL allows
    /// inside a block and this recorder cannot: a material is state, and
    /// changing it every vertex would put every vertex in its own draw call.
    /// A vertex colour is not state, so what is done instead is to cull one
    /// side and draw the other, twice. A triangle only ever shows one of its
    /// faces to the camera, so the two passes cover exactly the fragments the
    /// single unculled pass would have, with the same colour on each.
    fn draw_surface(&self, g: &mut Gl, mat: &[[f32; 3]; 3]) {
        let per_vertex = self.colors != Colors::TwoSided && self.colors != Colors::Earth;
        let passes = if per_vertex { 2 } else { 1 };

        for pass in 0..passes {
            if per_vertex {
                // Pass 0 keeps the front faces, pass 1 the back ones.
                g.glx.front_face_cw(pass == 1);
                g.glx.cull_face(true);
                g.glx.color_material(true);
            } else {
                g.glx.cull_face(false);
                g.glx.color_material(false);
            }
            let colors = if pass == 0 { &self.colf } else { &self.colb };

            if self.appearance == Appearance::ParallelBands {
                for i in 0..NUMTH {
                    if (i & (NUMBDIST - 1)) >= NUMBDIST / 4
                        && (i & (NUMBDIST - 1)) < 3 * NUMBDIST / 4
                    {
                        continue;
                    }
                    g.glx.begin(Shape::TriangleStrip);
                    for j in (0..=NUMPH).rev() {
                        for k in 0..=1 {
                            let o = at(j, i + k);
                            self.emit(g, mat, o, colors, per_vertex);
                        }
                    }
                    g.glx.end();
                }
            } else {
                for j in 0..NUMPH {
                    if self.appearance == Appearance::MeridianBands
                        && (j & (NUMBDIR - 1)) >= NUMBDIR / 4
                        && (j & (NUMBDIR - 1)) < 3 * NUMBDIR / 4
                    {
                        continue;
                    }
                    g.glx.begin(Shape::TriangleStrip);
                    for i in 0..=NUMTH {
                        for k in 0..=1 {
                            let o = at(j + k, i);
                            self.emit(g, mat, o, colors, per_vertex);
                        }
                    }
                    g.glx.end();
                }
            }
        }
        g.glx.cull_face(false);
        g.glx.front_face_cw(false);
    }

    fn emit(&self, g: &mut Gl, mat: &[[f32; 3]; 3], o: usize, colors: &[f32], per_vertex: bool) {
        let (p, n) = self.placed(mat, o);
        if per_vertex {
            let c = &colors[4 * o..4 * o + 4];
            g.glx.color4f(c[0], c[1], c[2], c[3]);
        }
        g.glx.normal3f(n[0], n[1], n[2]);
        g.glx.vertex3f(p[0], p[1], p[2]);
    }

    /// The white wireframe globe drawn over the surface.
    fn draw_graticule(&self, g: &mut Gl, mat: &[[f32; 3]; 3]) {
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        g.glx.line_width(2.0);
        g.glx.blend(Blend::Alpha);
        g.glx.lighting(false);

        /* Draw meridians. */
        for j in (0..NUMPH).step_by(NUMGRID) {
            g.glx.begin(Shape::LineStrip);
            for i in 0..=NUMTH {
                let (p, _) = self.placed(mat, at(j, i));
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
        }
        /* Draw parallels. */
        for i in (NUMGRID..=NUMTH - NUMGRID).step_by(NUMGRID) {
            g.glx.begin(Shape::LineLoop);
            for j in (0..NUMPH).rev() {
                let (p, _) = self.placed(mat, at(j, i));
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
        }

        g.glx.line_width(1.0);
        g.glx.lighting(true);
        if self.display_mode == DisplayMode::Transparent {
            g.glx.blend(Blend::AlphaAdd);
        } else {
            g.glx.blend(Blend::Off);
        }
    }

    /// `display_sphereeversion_analytic`: move the animation on one step.
    fn animate(&mut self) {
        if self.trackball.button_down() {
            return;
        }
        if self.anim_state == AnimState::Deform {
            self.tau += self.defdir * self.deform_speed * 0.001;
            if self.tau < BEDNORZ_TAU_MIN {
                self.tau = BEDNORZ_TAU_MIN;
                self.defdir = -self.defdir;
                self.anim_state = AnimState::Turn;
            }
            if self.tau > BEDNORZ_TAU_MAX {
                self.tau = BEDNORZ_TAU_MAX;
                self.defdir = -self.defdir;
                self.anim_state = AnimState::Turn;
            }
            if self.anim_state == AnimState::Turn {
                self.qs = angles_to_quat(self.alpha, self.beta, self.delta);
                let alpha = frand(120.0) as f32 - 60.0;
                let beta = frand(120.0) as f32 - 60.0;
                let delta = frand(360.0) as f32;
                self.qe = angles_to_quat(alpha, beta, delta);
                let mut dot = self.qs[0] * self.qe[0]
                    + self.qs[1] * self.qe[1]
                    + self.qs[2] * self.qe[2]
                    + self.qs[3] * self.qe[3];
                if dot < 0.0 {
                    for v in &mut self.qe {
                        *v = -*v;
                    }
                    dot = -dot;
                }
                let a = dot.clamp(-1.0, 1.0).acos().to_degrees();
                self.num_turn = (a / TURN_STEP).ceil() as i32;

                /* Change the parameters randomly after one full eversion when
                a turn to the new orientation starts. */
                if self.random_display_mode {
                    self.display_mode = match random() % 2 {
                        0 => DisplayMode::Surface,
                        _ => DisplayMode::Transparent,
                    };
                }
                if self.random_appearance {
                    self.appearance = match random() % 3 {
                        0 => Appearance::Solid,
                        1 => Appearance::ParallelBands,
                        _ => Appearance::MeridianBands,
                    };
                }
                if self.random_colors {
                    self.colors = match random() % 4 {
                        0 => Colors::TwoSided,
                        1 => Colors::Parallel,
                        2 => Colors::Meridian,
                        _ => Colors::Earth,
                    };
                }
                if self.random_graticule {
                    self.graticule = random() & 1 != 0;
                }
                if self.random_g {
                    self.g = (random() % 4) as i32 + 2;
                }
                self.setup_surface_colors();
            }
        } else {
            let t = self.turn_step as f32 / self.num_turn.max(1) as f32;
            /* Apply an easing function to t. */
            let t = (3.0 - 2.0 * t) * t * t;
            let q = quat_slerp(t, self.qs, self.qe);
            // Upstream reads these back in radians and then hands them to
            // `rotateall`, which takes degrees. That is upstream's arithmetic
            // and it is kept: the effect is that the turn between eversions
            // leaves the surface near the orientation it started this run in,
            // rather than at the random one it aimed for, and that is what the
            // saver looks like.
            let (alpha, beta, delta) = quat_to_angles(q);
            self.alpha = alpha;
            self.beta = beta;
            self.delta = delta;
            self.turn_step += 1;
            if self.turn_step > self.num_turn {
                self.turn_step = 0;
                self.anim_state = AnimState::Deform;
            }
        }

        if self.anim_state == AnimState::Deform {
            self.alpha += self.speed_x * self.speed_scale;
            if self.alpha >= 360.0 {
                self.alpha -= 360.0;
            }
            self.beta += self.speed_y * self.speed_scale;
            if self.beta >= 360.0 {
                self.beta -= 360.0;
            }
            self.delta += self.speed_z * self.speed_scale;
            if self.delta >= 360.0 {
                self.delta -= 360.0;
            }
        }
    }
}

impl Hack3d for SphereEversionState {
    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = width as f32 / height.max(1) as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.animate();
        self.compute_surface();

        /* Compute the rotation that rotates the surface in 3D, including the
        trackball rotations. */
        let r1 = rotateall(self.alpha, self.beta, self.delta);
        let q = self.trackball.quaternion();
        let r2 = quat_to_rotmat([q.x as f32, q.y as f32, q.z as f32, q.w as f32]);
        let mat = mult_rotmat(&r2, &r1);

        g.glx.clear_color(0.0, 0.0, 0.0, 0.0);
        g.glx.clear();

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        if self.projection == Projection::Perspective {
            g.glx.perspective(60.0, self.aspect, 0.1, 10.0);
        } else if self.aspect >= 1.0 {
            g.glx.ortho(-self.aspect, self.aspect, -1.0, 1.0, 0.1, 10.0);
        } else {
            g.glx
                .ortho(-1.0, 1.0, -1.0 / self.aspect, 1.0 / self.aspect, 0.1, 10.0);
        }
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(
            0,
            LIGHT_POSITION[0],
            LIGHT_POSITION[1],
            LIGHT_POSITION[2],
            LIGHT_POSITION[3],
        );
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(50.0);

        if self.display_mode == DisplayMode::Surface {
            g.glx.depth_test(true);
            g.glx.depth_mask(true);
            g.glx.blend(Blend::Off);
        } else {
            g.glx.depth_test(false);
            g.glx.depth_mask(false);
            g.glx.blend(Blend::AlphaAdd);
        }

        if self.colors == Colors::TwoSided || self.colors == Colors::Earth {
            let (front, back) = if self.display_mode == DisplayMode::Transparent {
                (MAT_DIFF_TRANS_FRONT, MAT_DIFF_TRANS_BACK)
            } else {
                (MAT_DIFF_FRONT, MAT_DIFF_BACK)
            };
            g.glx.material_ambient_diffuse(front);
            g.glx.material_back_ambient_diffuse(back);
        }

        g.glx.polygon_offset(Some((1.0, 1.0)));
        self.draw_surface(g, &mat);
        g.glx.polygon_offset(None);

        if self.graticule {
            self.draw_graticule(g, &mat);
        }

        g.res.int("delay").max(0) as u32
    }
}

/// Pick one of `n` at random, for the knobs whose value is "random".
fn pick(n: u32) -> u32 {
    random() % n
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mode = g.res.string("mode").to_string();
    let (display_mode, random_display_mode) = match mode.as_str() {
        "surface" => (DisplayMode::Surface, false),
        "transparent" => (DisplayMode::Transparent, false),
        _ => (
            if pick(2) == 0 {
                DisplayMode::Surface
            } else {
                DisplayMode::Transparent
            },
            true,
        ),
    };

    let appear = g.res.string("appearance").to_string();
    let (appearance, random_appearance) = match appear.as_str() {
        "solid" => (Appearance::Solid, false),
        "parallel-bands" => (Appearance::ParallelBands, false),
        "meridian-bands" => (Appearance::MeridianBands, false),
        _ => (
            match pick(3) {
                0 => Appearance::Solid,
                1 => Appearance::ParallelBands,
                _ => Appearance::MeridianBands,
            },
            true,
        ),
    };

    let color_mode = g.res.string("colors").to_string();
    let (colors, random_colors) = match color_mode.as_str() {
        "two-sided" => (Colors::TwoSided, false),
        "parallel" => (Colors::Parallel, false),
        "meridian" => (Colors::Meridian, false),
        "earth" => (Colors::Earth, false),
        _ => (
            match pick(4) {
                0 => Colors::TwoSided,
                1 => Colors::Parallel,
                2 => Colors::Meridian,
                _ => Colors::Earth,
            },
            true,
        ),
    };

    let proj = g.res.string("projection").to_string();
    let projection = match proj.as_str() {
        "perspective" => Projection::Perspective,
        "orthographic" => Projection::Orthographic,
        _ => {
            if pick(2) == 0 {
                Projection::Perspective
            } else {
                Projection::Orthographic
            }
        }
    };

    let grat = g.res.string("graticule").to_string();
    let (graticule, random_graticule) = match grat.as_str() {
        "on" => (true, false),
        "off" => (false, false),
        _ => (random() & 1 != 0, true),
    };

    let order = g.res.string("surfaceOrder").to_string();
    let (order, random_g) = match order.parse::<i32>() {
        Ok(n) if (2..=5).contains(&n) => (n, false),
        _ => ((pick(4)) as i32 + 2, true),
    };

    let mut deform_speed = g.res.float("deformSpeed") as f32;
    if deform_speed == 0.0 {
        deform_speed = 10.0;
    }

    let n = (NUMPH + 1) * (NUMTH + 1);
    let mut st = SphereEversionState {
        display_mode,
        random_display_mode,
        appearance,
        random_appearance,
        colors,
        random_colors,
        projection,
        graticule,
        random_graticule,
        g: order,
        random_g,

        alpha: frand(120.0) as f32 - 60.0,
        beta: frand(120.0) as f32 - 60.0,
        delta: frand(360.0) as f32,
        anim_state: AnimState::Deform,
        tau: BEDNORZ_TAU_MAX,
        defdir: -1.0,
        turn_step: 0,
        num_turn: 0,
        qs: [0.0; 4],
        qe: [0.0; 4],
        eta_min: BEDNORZ_ETA_MIN,
        beta_max: BEDNORZ_BETA_MAX,
        offset3d: [0.0, 0.0, -1.8],

        sp: vec![0.0; 3 * n],
        sn: vec![0.0; 3 * n],
        colf: vec![0.0; 4 * n],
        colb: vec![0.0; 4 * n],

        aspect: 1.0,
        trackball: Trackball::new(),
        /* Make multiple screens rotate at slightly different rates. */
        speed_scale: 0.9 + frand(0.3) as f32,
        speed_x: g.res.float("speedx") as f32,
        speed_y: g.res.float("speedy") as f32,
        speed_z: g.res.float("speedz") as f32,
        deform_speed,
    };
    st.setup_surface_colors();

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        10000",
    "*showFPS:      False",
    "*mode:         random",
    "*appearance:   random",
    "*colors:       random",
    "*projection:   random",
    "*graticule:    random",
    "*surfaceOrder: random",
    "*speedx:       0.0",
    "*speedy:       0.0",
    "*speedz:       0.0",
    "*deformSpeed:  10.0",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random surface",
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
        value: "parallel-bands",
        label: "Parallel bands",
    },
    SelectItem {
        value: "meridian-bands",
        label: "Meridian bands",
    },
];

const GRATICULES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random graticule",
    },
    SelectItem {
        value: "on",
        label: "With graticule",
    },
    SelectItem {
        value: "off",
        label: "Without graticule",
    },
];

const COLORINGS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random coloration",
    },
    SelectItem {
        value: "two-sided",
        label: "Two-sided",
    },
    SelectItem {
        value: "parallel",
        label: "Parallel colors",
    },
    SelectItem {
        value: "meridian",
        label: "Meridian colors",
    },
    SelectItem {
        value: "earth",
        label: "Earth colors",
    },
];

const PROJECTIONS: &[SelectItem] = &[
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

const ORDERS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random surface order",
    },
    SelectItem {
        value: "2",
        label: "Surface order 2",
    },
    SelectItem {
        value: "3",
        label: "Surface order 3",
    },
    SelectItem {
        value: "4",
        label: "Surface order 4",
    },
    SelectItem {
        value: "5",
        label: "Surface order 5",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider(
        "deformSpeed",
        "Deformation speed",
        1.0,
        100.0,
        1.0,
        1,
        "10.0",
    ),
    Opt::select("mode", "Surface", MODES, "random"),
    Opt::select("appearance", "Pattern", APPEARANCES, "random"),
    Opt::select("graticule", "Graticule", GRATICULES, "random"),
    Opt::select("colors", "Coloration", COLORINGS, "random"),
    Opt::select("projection", "Projection", PROJECTIONS, "random"),
    Opt::select("surfaceOrder", "Surface order", ORDERS, "random"),
    Opt::slider("speedx", "X rotation speed", -4.0, 4.0, 0.1, 1, "0.0"),
    Opt::slider("speedy", "Y rotation speed", -4.0, 4.0, 0.1, 1, "0.0"),
    Opt::slider("speedz", "Z rotation speed", -4.0, 4.0, 0.1, 1, "0.0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "sphereeversion",
    label: "Sphere Eversion",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Steger",
        year: "2020",
        video: Some("https://www.youtube.com/watch?v=CbmIggJ5GdA"),
        blurb: "Turns a sphere inside out: a smooth deformation (homotopy).",
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

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// At the two ends of the deformation the surface really is a sphere: the
    /// whole point of the eversion is that it starts and finishes as one.
    #[test]
    fn it_begins_and_ends_as_a_sphere() {
        for tau in [BEDNORZ_TAU_MIN, BEDNORZ_TAU_MAX] {
            let mut r = start(StartArgs::new(640, 480, "surfaceOrder=2", 20260812));
            r.step();
            // Build the surface by hand at the extreme of tau.
            let n = (NUMPH + 1) * (NUMTH + 1);
            let mut st = SphereEversionState {
                display_mode: DisplayMode::Surface,
                random_display_mode: false,
                appearance: Appearance::Solid,
                random_appearance: false,
                colors: Colors::TwoSided,
                random_colors: false,
                projection: Projection::Perspective,
                graticule: false,
                random_graticule: false,
                g: 2,
                random_g: false,
                alpha: 0.0,
                beta: 0.0,
                delta: 0.0,
                anim_state: AnimState::Deform,
                tau,
                defdir: -1.0,
                turn_step: 0,
                num_turn: 0,
                qs: [0.0; 4],
                qe: [0.0; 4],
                eta_min: BEDNORZ_ETA_MIN,
                beta_max: BEDNORZ_BETA_MAX,
                offset3d: [0.0; 3],
                sp: vec![0.0; 3 * n],
                sn: vec![0.0; 3 * n],
                colf: vec![0.0; 4 * n],
                colb: vec![0.0; 4 * n],
                aspect: 1.0,
                trackball: Trackball::new(),
                speed_scale: 1.0,
                speed_x: 0.0,
                speed_y: 0.0,
                speed_z: 0.0,
                deform_speed: 10.0,
            };
            st.compute_surface();

            // Every point is the same distance from the middle, to a part in a
            // hundred. The surface is scaled to a radius of 0.75.
            let mut lo = f32::MAX;
            let mut hi = 0.0f32;
            for o in 0..n {
                let (x, y, z) = (st.sp[3 * o], st.sp[3 * o + 1], st.sp[3 * o + 2]);
                let r = (x * x + y * y + z * z).sqrt();
                lo = lo.min(r);
                hi = hi.max(r);
            }
            assert!(
                hi - lo < 0.01,
                "at tau {tau} the radius ran from {lo} to {hi}"
            );
            assert!((hi - 0.75).abs() < 0.01, "the sphere is {hi} across");
        }
    }

    /// Halfway through, it is not a sphere at all. If it were, nothing would
    /// be happening.
    #[test]
    fn it_is_not_a_sphere_in_the_middle() {
        let n = (NUMPH + 1) * (NUMTH + 1);
        let bsp = {
            let alpha = bednorz_get_alpha(0.0);
            let beta = bednorz_get_beta(0.0, BEDNORZ_BETA_MAX);
            ShapePar {
                n: 2,
                kappa: bednorz_get_kappa(2),
                omega: BEDNORZ_OMEGA,
                t: bednorz_get_t(0.0),
                p: bednorz_get_p(0.0),
                q: bednorz_get_q(0.0),
                xi: bednorz_get_xi(0.0),
                eta: bednorz_get_eta(0.0, BEDNORZ_ETA_MIN),
                alpha,
                beta,
                gamma: bednorz_get_gamma(alpha, beta),
                lambda: bednorz_get_lambda(0.0),
                eps: bednorz_get_eps(0.0, 2),
            }
        };
        let mut lo = f32::MAX;
        let mut hi = 0.0f32;
        for j in (0..=NUMPH).step_by(8) {
            let phi = 2.0 * std::f64::consts::PI * j as f64 / NUMPH as f64 - std::f64::consts::PI;
            for i in (0..=NUMTH).step_by(8) {
                let theta =
                    std::f64::consts::PI * i as f64 / NUMTH as f64 - std::f64::consts::FRAC_PI_2;
                let (p, _) = bednorz_point_normal(phi, theta, &bsp);
                let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                lo = lo.min(r);
                hi = hi.max(r);
            }
        }
        assert!(hi > lo * 1.5, "halfway through it was still round");
        assert!(n > 0);
    }

    /// Every point and every normal is a real number all the way through the
    /// eversion, including at the poles, where the formula is singular and
    /// upstream substitutes a value by hand.
    #[test]
    fn the_surface_is_finite_everywhere() {
        for order in 2..=5 {
            for step in 0..=24 {
                let tau = -BEDNORZ_TAU4 + 2.0 * BEDNORZ_TAU4 * f64::from(step) / 24.0;
                let alpha = bednorz_get_alpha(tau);
                let beta = bednorz_get_beta(tau, BEDNORZ_BETA_MAX);
                let bsp = ShapePar {
                    n: order,
                    kappa: bednorz_get_kappa(order),
                    omega: BEDNORZ_OMEGA,
                    t: bednorz_get_t(tau),
                    p: bednorz_get_p(tau),
                    q: bednorz_get_q(tau),
                    xi: bednorz_get_xi(tau),
                    eta: bednorz_get_eta(tau, BEDNORZ_ETA_MIN),
                    alpha,
                    beta,
                    gamma: bednorz_get_gamma(alpha, beta),
                    lambda: bednorz_get_lambda(tau),
                    eps: bednorz_get_eps(tau, order),
                };
                for j in (0..=NUMPH).step_by(16) {
                    let phi =
                        2.0 * std::f64::consts::PI * j as f64 / NUMPH as f64 - std::f64::consts::PI;
                    for i in (0..=NUMTH).step_by(16) {
                        let theta = std::f64::consts::PI * i as f64 / NUMTH as f64
                            - std::f64::consts::FRAC_PI_2;
                        let (p, n) = bednorz_point_normal(phi, theta, &bsp);
                        for v in p.iter().chain(n.iter()) {
                            assert!(
                                v.is_finite(),
                                "order {order} tau {tau} phi {phi} theta {theta} gave {v}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The deformation runs, turns around at each end and keeps going.
    #[test]
    fn the_deformation_runs_end_to_end() {
        let mut r = start(StartArgs::new(640, 480, "deformSpeed=100", 20260812));
        let mut lo = f32::MAX;
        let mut hi = -f32::MAX;
        for _ in 0..200 {
            r.step();
            let f = r.frame();
            for v in &f.vertices {
                lo = lo.min(v.pos[0]);
                hi = hi.max(v.pos[0]);
            }
        }
        assert!(hi > lo, "nothing was ever drawn");
    }

    /// Bands leave gaps in the surface, so they draw fewer strips than solid.
    #[test]
    fn bands_leave_gaps() {
        let solid = run("appearance=solid&colors=two-sided&graticule=off", 2)
            .frame()
            .batches
            .len();
        let parallel = run(
            "appearance=parallel-bands&colors=two-sided&graticule=off",
            2,
        )
        .frame()
        .batches
        .len();
        let meridian = run(
            "appearance=meridian-bands&colors=two-sided&graticule=off",
            2,
        )
        .frame()
        .batches
        .len();
        assert!(parallel < solid, "{parallel} against {solid}");
        assert!(meridian < solid, "{meridian} against {solid}");
    }

    /// The two sides of the surface are different colours, which is what makes
    /// an eversion legible at all: when the inside has come out, the colour
    /// facing you has swapped.
    #[test]
    fn the_two_sides_are_different_colours() {
        let r = run("colors=meridian&appearance=solid&graticule=off", 2);
        let f = r.frame();
        let mut front = std::collections::BTreeSet::new();
        let mut back = std::collections::BTreeSet::new();
        for b in &f.batches {
            let set = if b.front_face_cw {
                &mut back
            } else {
                &mut front
            };
            for v in &f.vertices[b.first..b.first + b.count] {
                set.insert(v.color.map(f32::to_bits));
            }
        }
        assert!(front.len() > 4, "the front was one colour");
        assert!(back.len() > 4, "the back was one colour");
        assert!(
            front.intersection(&back).count() < front.len() / 2,
            "the two sides came out the same colour"
        );
    }

    /// The graticule is white lines over the surface, and it can be turned off.
    #[test]
    fn the_graticule_can_be_turned_off() {
        let with = run("graticule=on&colors=two-sided", 2);
        let f = with.frame();
        let lines = f
            .batches
            .iter()
            .filter(|b| {
                matches!(
                    b.primitive,
                    crate::runtime::gl::Primitive::LineStrip
                        | crate::runtime::gl::Primitive::LineLoop
                )
            })
            .count();
        assert!(lines >= 8, "only {lines} graticule lines");

        let without = run("graticule=off&colors=two-sided", 2);
        assert!(
            without.frame().batches.iter().all(|b| !matches!(
                b.primitive,
                crate::runtime::gl::Primitive::LineStrip | crate::runtime::gl::Primitive::LineLoop
            )),
            "the graticule was drawn when it was turned off"
        );
    }

    /// How much geometry a frame comes to, which is the thing to watch on a
    /// saver that rebuilds its whole surface every frame.
    #[test]
    fn a_frame_fits_in_the_budget() {
        // The heaviest setting: a solid surface whose two sides are coloured
        // per vertex, so it is drawn twice, with the graticule over it. 267k
        // vertices in 527 strips, and a strip cannot merge with the strip
        // beside it.
        let r = run("appearance=solid&colors=meridian&graticule=on", 3);
        let f = r.frame();
        assert!(
            f.vertices.len() < 400_000,
            "a frame came to {} vertices",
            f.vertices.len()
        );
        assert!(
            f.batches.len() < 700,
            "a frame came to {} batches",
            f.batches.len()
        );
    }
}
