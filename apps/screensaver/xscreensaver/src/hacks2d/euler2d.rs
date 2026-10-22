//! Port of `hacks/euler2d.c`.
//!
//! ```text
//! euler2d --- 2 Dimensional Incompressible Inviscid Fluid Flow
//!
//! Copyright (c) 2000 by Stephen Montgomery-Smith <stephen@math.missouri.edu>
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
//! Revision History:
//! 04-Nov-2000: Added an option eulerpower.  This allows for example the
//!              quasi-geostrophic equation by setting eulerpower to 2.
//! 01-Nov-2000: Allocation checks.
//! 10-Sep-2000: Added optimizations, and removed subtle_perturb, by stephen.
//! 03-Sep-2000: Changed method of solving ode to Adams-Bashforth of order 2.
//!              Previously used a rather compilcated method of order 4.
//!              This doubles the speed of the program.  Also it seems
//!              to have improved numerical stability.  Done by stephen.
//! 27-Aug-2000: Added rotation of region to maximize screen fill by stephen.
//! 05-Jun-2000: Adapted from flow.c Copyright (c) 1996 by Tim Auckland
//! 18-Jul-1996: Adapted from swarm.c Copyright (c) 1991 by Patrick J. Naughton.
//! 31-Aug-1990: Adapted from xswarm by Jeff Butterworth. (butterwo@ncsc.org)
//! ```
//!
//! Twenty vortex points stir a thousand specks of dust, and the dust is what
//! you see. Each vortex drags the fluid round itself with a pull that falls off
//! with distance, every point is carried by the sum of all twenty, and the
//! trails the specks leave are the flow.
//!
//! Two things make it more than a swarm of springs. The fluid is confined, and
//! confining it exactly is done with an old trick: for each vortex, add a
//! mirror-image vortex at its reflection in the unit circle, and the two
//! together push nothing through the boundary. The boundary is then bent by
//! running the whole disk through a polynomial chosen so that it folds the
//! circle into a lobed blob without ever folding it onto itself, which needs
//! only that the coefficients sum small enough. Motion is computed in the round
//! disk where the mirror trick works and drawn in the bent one, with the speed
//! divided by how much the polynomial stretched that spot.
//!
//! The rest is bookkeeping that keeps it stable. A point that strays onto a
//! vortex, or onto the wall, or takes a step too large to trust, is marked dead
//! and stops being drawn. The first frame steps with the midpoint rule and
//! every frame after it with Adams-Bashforth, which is why the previous frame's
//! derivative is kept. Before drawing, the hack tries eighteen rotations of the
//! blob and keeps whichever one fills the window best.
//!
//! One knob here is not in the upstream XML: `eulerpower`, the exponent in the
//! law by which a vortex pulls. Upstream defines it and documents it but leaves
//! it off the settings panel. It is worth having, because anything other than
//! one is a different equation (two is quasi-geostrophic flow) and switches off
//! the polynomial boundary, leaving the plain disk that the rest of the hack
//! was written against. Its range here is upstream's own clamp.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, MAXRAND, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XSegment};

/// How many vortex points stir the fluid.
const NUMBER_OF_VORTEX_POINTS: usize = 20;
/// How many segments the drawn boundary is made of.
const N_BOUND_P: usize = 500;
/// The degree of the polynomial that bends the disk.
const DEG_P: usize = 6;
/// How many rotations of the region to try when fitting it to the window. Must
/// be even.
const NR_ROTATES: usize = 18;

/// `positive_rand(v)`: uniform on `0..v`.
fn positive_rand(v: f64) -> f64 {
    lrand() as f64 / MAXRAND * v
}

/// `balance_rand(v)`: uniform on `-v/2..v/2`.
fn balance_rand(v: f64) -> f64 {
    lrand() as f64 / MAXRAND * v - v / 2.0
}

/// The `mult` macro: complex multiplication.
fn cmul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

/// `p(z) = z + c_2 z^2 + ... + c_n z^n`, by Horner.
fn calc_p(z: (f64, f64), p_coef: &[f64]) -> (f64, f64) {
    let mut p = (0.0, 0.0);
    for i in (2..=DEG_P).rev() {
        p.0 += p_coef[(i - 2) * 2];
        p.1 += p_coef[(i - 2) * 2 + 1];
        p = cmul(p, z);
    }
    p.0 += 1.0;
    cmul(p, z)
}

/// `|p'(z)|^2`: how much the polynomial stretches the disk at `z`.
fn calc_mod_dp2(z: (f64, f64), p_coef: &[f64]) -> f64 {
    let mut mp = (0.0, 0.0);
    for i in (2..=DEG_P).rev() {
        mp.0 += i as f64 * p_coef[(i - 2) * 2];
        mp.1 += i as f64 * p_coef[(i - 2) * 2 + 1];
        mp = cmul(mp, z);
    }
    mp.0 += 1.0;
    mp.0 * mp.0 + mp.1 * mp.1
}

struct Euler2d {
    mi: ModeInfo,
    /// The window the flow was fitted to, which is not the real one when the
    /// window has a weird aspect.
    width: i32,
    height: i32,
    /// Frames since this flow started. Zero means nothing has been drawn yet,
    /// so there is no previous position to draw from.
    count: i32,
    xshift: f64,
    yshift: f64,
    scale: f64,
    xshift2: f64,
    yshift2: f64,
    /// The radius of the plain circular boundary, used when the polynomial one
    /// is switched off.
    radius: f64,

    /// Total points, vortex points first.
    n: usize,
    nvortex: usize,
    /// Whether the polynomial boundary is in use, which upstream ties to the
    /// interaction power being exactly one.
    variable_boundary: bool,
    power: f64,
    delta_t: f64,
    tail_len: usize,

    /// `x[2i]`, `x[2i+1]`: the position of the nth point in the round disk.
    x: Vec<f64>,
    /// The vorticity of the nth vortex point.
    w: Vec<f64>,

    diffx: Vec<f64>,
    olddiffx: Vec<f64>,
    tempx: Vec<f64>,
    tempdiffx: Vec<f64>,

    /// The reflection of each vortex point in the unit circle, and whether it
    /// sits close enough to the origin to have no usable one.
    xs: Vec<f64>,
    x_is_zero: Vec<bool>,

    /// Each point's image under the polynomial, and `|p'|^2` there.
    p: Vec<f64>,
    mod_dp2: Vec<f64>,

    /// Points that overflowed, hit the wall or hit a vortex. They stop moving
    /// and stop being drawn.
    dead: Vec<bool>,

    csegs: Vec<XSegment>,
    cnsegs: usize,
    /// A ring of `tail_len` frames of segments, kept so they can be erased.
    old_segs: Vec<XSegment>,
    nold_segs: Vec<usize>,
    c_old_seg: usize,
    boundary_color: usize,
    hide_vortex: bool,
    /// Where each point was drawn last frame, truncated to a short the way
    /// upstream's `XSegment` is.
    lastx: Vec<i16>,

    p_coef: [f64; 2 * (DEG_P - 1)],
    boundary: Vec<XSegment>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut st = Euler2d {
        mi,
        width: d.width(),
        height: d.height(),
        count: 0,
        xshift: 0.0,
        yshift: 0.0,
        scale: 1.0,
        xshift2: 0.0,
        yshift2: 0.0,
        radius: 1.0,
        n: 0,
        nvortex: NUMBER_OF_VORTEX_POINTS,
        variable_boundary: true,
        power: 1.0,
        delta_t: 0.001,
        tail_len: 1,
        x: Vec::new(),
        w: Vec::new(),
        diffx: Vec::new(),
        olddiffx: Vec::new(),
        tempx: Vec::new(),
        tempdiffx: Vec::new(),
        xs: Vec::new(),
        x_is_zero: Vec::new(),
        p: Vec::new(),
        mod_dp2: Vec::new(),
        dead: Vec::new(),
        csegs: Vec::new(),
        cnsegs: 0,
        old_segs: Vec::new(),
        nold_segs: Vec::new(),
        c_old_seg: 0,
        boundary_color: 0,
        hide_vortex: false,
        lastx: Vec::new(),
        p_coef: [0.0; 2 * (DEG_P - 1)],
        boundary: Vec::new(),
    };
    st.power = d.res.float("eulerpower");
    st.tail_len = d.res.int("eulertail").max(1) as usize;
    st.restart(d);
    Box::new(st)
}

impl Euler2d {
    /// `init_euler2d`, which is also how a finished flow is replaced.
    fn restart(&mut self, d: &mut Dpy) {
        self.power = self.power.clamp(0.5, 3.0);
        self.variable_boundary = self.power == 1.0;
        self.delta_t = 0.001;
        if self.power > 1.0 {
            self.delta_t *= 0.1f64.powf(self.power - 1.0);
        }

        self.boundary_color = nrand(self.mi.npixels()) as usize;
        self.hide_vortex = nrand(4) != 0;

        self.count = 0;
        self.xshift2 = 0.0;
        self.yshift2 = 0.0;

        self.width = self.mi.width;
        self.height = self.mi.height;

        if self.width > self.height * 5 || self.height > self.width * 5 {
            if self.width > self.height {
                self.height = (self.width as f64 * 0.8) as i32;
                self.yshift2 = -(self.height / 2) as f64;
            } else {
                self.width = (self.height as f64 * 0.8) as i32;
                self.xshift2 = -(self.width / 2) as f64;
            }
        }

        if self.width > 2560 || self.height > 2560 {
            self.mi.gc.set_line_width(3); // Retina displays.
        }

        // The particle count only takes effect the first time, because the
        // buffers are sized from it and upstream never grows them.
        if self.n == 0 {
            self.n = self.mi.count.max(0) as usize + NUMBER_OF_VORTEX_POINTS;
            self.nvortex = NUMBER_OF_VORTEX_POINTS;

            self.tail_len = self.tail_len.clamp(1, self.mi.cycles.max(1) as usize);

            let n = self.n;
            self.csegs = vec![XSegment::default(); n];
            self.old_segs = vec![XSegment::default(); n * self.tail_len];
            self.nold_segs = vec![0; self.tail_len];
            self.lastx = vec![0; n * 2];
            self.x = vec![0.0; n * 2];
            self.diffx = vec![0.0; n * 2];
            self.w = vec![0.0; self.nvortex];
            self.olddiffx = vec![0.0; n * 2];
            self.tempdiffx = vec![0.0; n * 2];
            self.tempx = vec![0.0; n * 2];
            self.dead = vec![false; n];
            self.boundary = vec![XSegment::default(); N_BOUND_P];
            self.xs = vec![0.0; self.nvortex * 2];
            self.x_is_zero = vec![false; self.nvortex];
            self.p = vec![0.0; n * 2];
            self.mod_dp2 = vec![0.0; n];
        }

        self.mi.clear_window(d);

        self.nold_segs.fill(0);
        self.c_old_seg = 0;
        self.dead.fill(false);

        if self.variable_boundary {
            self.init_polynomial();
            self.fit_to_window();
            self.init_boundary();
        } else if self.width > self.height {
            self.radius = self.height as f64 / 2.0 - 5.0;
        } else {
            self.radius = self.width as f64 / 2.0 - 5.0;
        }

        self.init_points();
    }

    /// Roll a polynomial that maps the unit disk onto a lobed blob. It is a
    /// bijection as long as `sum k |c_k| <= 1`, and upstream makes that an
    /// equality to get more interesting shapes.
    fn init_polynomial(&mut self) {
        let mut mag = 0.0;
        for k in 2..=DEG_P {
            let r = positive_rand(1.0 / k as f64);
            let theta = balance_rand(std::f64::consts::TAU);
            self.p_coef[2 * (k - 2)] = r * theta.cos();
            self.p_coef[2 * (k - 2) + 1] = r * theta.sin();
            mag += k as f64 * r;
        }
        if mag > 0.0001 {
            for k in 2..=DEG_P {
                self.p_coef[2 * (k - 2)] /= mag;
                self.p_coef[2 * (k - 2) + 1] /= mag;
            }
        }
    }

    /// Try every rotation of the blob and keep the one that fills the window
    /// best, then fold that rotation into the polynomial itself by replacing
    /// `p(z)` with `a^-1 p(a z)`.
    fn fit_to_window(&mut self) {
        let mut low = [1e5f64; NR_ROTATES];
        let mut high = [-1e5f64; NR_ROTATES];
        let nrot = NR_ROTATES as f64;

        for k in 0..N_BOUND_P as i32 {
            let at = |j: i32| {
                let a = j as f64 / N_BOUND_P as f64 * std::f64::consts::TAU;
                calc_p((a.cos(), a.sin()), &self.p_coef)
            };
            let (p1, p2) = at(k);
            let prev = at(k - 1);
            let next = at(k + 1);

            // The two edges meeting at this boundary point, as angles in the
            // units the search steps through: one per half-turn over
            // NR_ROTATES.
            let mut angle1 =
                nrot / std::f64::consts::PI * (p2 - prev.1).atan2(p1 - prev.0) - nrot / 2.0;
            let mut angle2 =
                nrot / std::f64::consts::PI * (next.1 - p2).atan2(next.0 - p1) - nrot / 2.0;
            while angle1 < 0.0 {
                angle1 += nrot * 2.0;
            }
            while angle2 < 0.0 {
                angle2 += nrot * 2.0;
            }
            if angle1 > nrot * 1.75 && angle2 < nrot * 0.25 {
                angle2 += nrot * 2.0;
            }
            if angle1 < nrot * 0.25 && angle2 > nrot * 1.75 {
                angle1 += nrot * 2.0;
            }
            if angle2 < angle1 {
                std::mem::swap(&mut angle1, &mut angle2);
            }

            let mut i = angle1.floor() as i32;
            while i < angle2.ceil() as i32 {
                let a = i as f64 * std::f64::consts::PI / nrot;
                let dist = a.cos() * p1 + a.sin() * p2;
                let slot = (i as usize) % NR_ROTATES;
                // The far half of the sweep sees the same axis from the other
                // side, so its distances are the negated ones.
                let dist = if (i as usize) % (NR_ROTATES * 2) < NR_ROTATES {
                    dist
                } else {
                    -dist
                };
                high[slot] = high[slot].max(dist);
                low[slot] = low[slot].min(dist);
                i += 1;
            }
        }

        let mut bestscale = 0.0;
        let mut besti = 0;
        for i in 0..NR_ROTATES {
            let xscale = (self.width as f64 - 5.0) / (high[i] - low[i]);
            let j = (i + NR_ROTATES / 2) % NR_ROTATES;
            let yscale = (self.height as f64 - 5.0) / (high[j] - low[j]);
            let scale = xscale.min(yscale);
            if scale > bestscale {
                bestscale = scale;
                besti = i;
            }
        }

        let a = (
            (besti as f64 * std::f64::consts::PI / nrot).cos(),
            (besti as f64 * std::f64::consts::PI / nrot).sin(),
        );
        let mut pow = (1.0, 0.0);
        for k in 2..=DEG_P {
            pow = cmul(pow, a);
            let c = cmul(
                (self.p_coef[2 * (k - 2)], self.p_coef[2 * (k - 2) + 1]),
                pow,
            );
            self.p_coef[2 * (k - 2)] = c.0;
            self.p_coef[2 * (k - 2) + 1] = c.1;
        }

        self.scale = bestscale;
        self.xshift = -(low[besti] + high[besti]) / 2.0 * self.scale + self.width as f64 / 2.0;
        self.yshift = if besti < NR_ROTATES / 2 {
            let j = besti + NR_ROTATES / 2;
            -(low[j] + high[j]) / 2.0 * self.scale + self.height as f64 / 2.0
        } else {
            let j = besti - NR_ROTATES / 2;
            (low[j] + high[j]) / 2.0 * self.scale + self.height as f64 / 2.0
        };

        self.xshift += self.xshift2;
        self.yshift += self.yshift2;
    }

    fn init_boundary(&mut self) {
        for k in 0..N_BOUND_P {
            let a = k as f64 / N_BOUND_P as f64 * std::f64::consts::TAU;
            let (p1, p2) = calc_p((a.cos(), a.sin()), &self.p_coef);
            self.boundary[k].x1 = (p1 * self.scale + self.xshift) as i32 as i16 as i32;
            self.boundary[k].y1 = (p2 * self.scale + self.yshift) as i32 as i16 as i32;
        }
        for k in 1..N_BOUND_P {
            self.boundary[k].x2 = self.boundary[k - 1].x1;
            self.boundary[k].y2 = self.boundary[k - 1].y1;
        }
        self.boundary[0].x2 = self.boundary[N_BOUND_P - 1].x1;
        self.boundary[0].y2 = self.boundary[N_BOUND_P - 1].y1;
    }

    /// Scatter the dust, then plant the vortices in a handful of clumps, some
    /// spinning one way and some the other.
    fn init_points(&mut self) {
        for i in self.nvortex..self.n {
            loop {
                let r = positive_rand(1.0).sqrt();
                let theta = balance_rand(std::f64::consts::TAU);
                self.x[2 * i] = r * theta.cos();
                self.x[2 * i + 1] = r * theta.sin();
                // Reject in proportion to how much the polynomial shrinks the
                // disk here, so the points come out evenly spread in the blob
                // rather than in the disk.
                if !self.variable_boundary
                    || calc_mod_dp2((self.x[2 * i], self.x[2 * i + 1]), &self.p_coef)
                        >= positive_rand(4.0)
                {
                    break;
                }
            }
        }

        let n = nrand(4) + 2;
        // How many clumps spin the other way. When there is an even number of
        // clumps, make an even split twice as likely as anything else.
        let np = if n % 2 != 0 {
            nrand(n + 1)
        } else {
            let np = nrand(n + 2);
            if np == n + 1 { n / 2 } else { np }
        };
        for k in 0..n {
            let r = positive_rand(0.77).sqrt();
            let theta = balance_rand(std::f64::consts::TAU);
            let x = r * theta.cos();
            let y = r * theta.sin();
            let r = 0.02 + positive_rand(0.1);
            let w = (2 * i32::from(k < np) - 1) as f64 * 2.0 / self.nvortex as f64;
            let lo = self.nvortex * k as usize / n as usize;
            let hi = self.nvortex * (k as usize + 1) / n as usize;
            for i in lo..hi {
                let theta = balance_rand(std::f64::consts::TAU);
                self.x[2 * i] = x + r * theta.cos();
                self.x[2 * i + 1] = y + r * theta.sin();
                self.w[i] = w;
            }
        }
    }

    /// `calc_all_mod_dp2`, which upstream always calls on `sp->x`.
    fn calc_all_mod_dp2(&mut self, sx: &[f64]) {
        for j in 0..self.n {
            if !self.dead[j] {
                self.mod_dp2[j] = calc_mod_dp2((sx[2 * j], sx[2 * j + 1]), &self.p_coef);
            }
        }
    }

    fn calc_all_p(&mut self) {
        let first = if self.hide_vortex { self.nvortex } else { 0 };
        for j in first..self.n {
            if !self.dead[j] {
                let p = calc_p((self.x[2 * j], self.x[2 * j + 1]), &self.p_coef);
                self.p[2 * j] = p.0;
                self.p[2 * j + 1] = p.1;
            }
        }
    }

    /// The velocity field at every point: the Biot-Savart kernel of each
    /// vortex, plus the kernel of its reflection in the unit circle, which is
    /// what keeps the flow inside the disk.
    ///
    /// `x` is the positions to evaluate at, and `sx` is `sp->x`, which upstream
    /// reads for `calc_all_mod_dp2` even on the step where it was handed
    /// `tempx`.
    fn derivs(&mut self, x: &[f64], sx: &[f64]) {
        if self.variable_boundary {
            self.calc_all_mod_dp2(sx);
        }

        for j in 0..self.nvortex {
            if self.dead[j] {
                continue;
            }
            let nx = x[2 * j] * x[2 * j] + x[2 * j + 1] * x[2 * j + 1];
            if nx < 1e-10 {
                self.x_is_zero[j] = true;
            } else {
                self.x_is_zero[j] = false;
                self.xs[2 * j] = x[2 * j] / nx;
                self.xs[2 * j + 1] = x[2 * j + 1] / nx;
            }
        }

        self.diffx.fill(0.0);

        for i in 0..self.n {
            if self.dead[i] {
                continue;
            }
            let x1 = x[2 * i];
            let x2 = x[2 * i + 1];
            for j in 0..self.nvortex {
                if self.dead[j] {
                    continue;
                }

                let mut xij1 = x1 - x[2 * j];
                let mut xij2 = x2 - x[2 * j + 1];
                let mut nxij = self.kernel_norm(xij1, xij2);

                let (mut u1, mut u2) = if nxij >= 1e-4 {
                    (xij2 / nxij, -xij1 / nxij)
                } else {
                    (0.0, 0.0)
                };

                if !self.x_is_zero[j] {
                    xij1 = x1 - self.xs[2 * j];
                    xij2 = x2 - self.xs[2 * j + 1];
                    nxij = self.kernel_norm(xij1, xij2);

                    if nxij < 1e-5 {
                        self.dead[i] = true;
                        u1 = 0.0;
                        u2 = 0.0;
                    } else {
                        u1 -= xij2 / nxij;
                        u2 += xij1 / nxij;
                    }
                }

                if !self.dead[i] {
                    self.diffx[2 * i] += u1 * self.w[j];
                    self.diffx[2 * i + 1] += u2 * self.w[j];
                }
            }

            if !self.dead[i] && self.variable_boundary {
                if self.mod_dp2[i] < 1e-5 {
                    self.dead[i] = true;
                } else {
                    self.diffx[2 * i] /= self.mod_dp2[i];
                    self.diffx[2 * i + 1] /= self.mod_dp2[i];
                }
            }
        }
    }

    /// The denominator of the interaction law. At the default power this is
    /// just the squared distance, which is why upstream special-cases it.
    fn kernel_norm(&self, a: f64, b: f64) -> f64 {
        let d2 = a * a + b * b;
        if self.power == 1.0 {
            d2
        } else {
            d2.powf((self.power + 1.0) / 2.0)
        }
    }

    /// Take a step. Upstream always steps from `sp->x` by `sp->tempdiffx`, so
    /// only the destination varies. A step too big to trust, or one that would
    /// land on the wall, kills the point instead.
    fn perturb(&mut self, into_tempx: bool) {
        for i in 0..self.n {
            if self.dead[i] {
                continue;
            }
            let x1 = self.x[2 * i];
            let x2 = self.x[2 * i + 1];
            let k1 = self.tempdiffx[2 * i];
            let k2 = self.tempdiffx[2 * i + 1];
            if k1 * k1 + k2 * k2 > 0.1 || x1 * x1 + x2 * x2 > 1.0 - 1e-5 {
                self.dead[i] = true;
            } else {
                let dst = if into_tempx {
                    &mut self.tempx
                } else {
                    &mut self.x
                };
                dst[2 * i] = x1 + k1;
                dst[2 * i + 1] = x2 + k2;
            }
        }
    }

    fn ode_solve(&mut self) {
        if self.count < 1 {
            // Midpoint method, for the one step that has no previous
            // derivative to lean on.
            let x = std::mem::take(&mut self.x);
            self.derivs(&x, &x);
            self.x = x;
            self.olddiffx.copy_from_slice(&self.diffx);
            for i in 0..self.n {
                if !self.dead[i] {
                    self.tempdiffx[2 * i] = 0.5 * self.delta_t * self.diffx[2 * i];
                    self.tempdiffx[2 * i + 1] = 0.5 * self.delta_t * self.diffx[2 * i + 1];
                }
            }
            self.perturb(true);

            let tempx = std::mem::take(&mut self.tempx);
            let x = std::mem::take(&mut self.x);
            self.derivs(&tempx, &x);
            self.x = x;
            self.tempx = tempx;

            for i in 0..self.n {
                if !self.dead[i] {
                    self.tempdiffx[2 * i] = self.delta_t * self.diffx[2 * i];
                    self.tempdiffx[2 * i + 1] = self.delta_t * self.diffx[2 * i + 1];
                }
            }
            self.perturb(false);
        } else {
            // Adams-Bashforth of order two.
            let x = std::mem::take(&mut self.x);
            self.derivs(&x, &x);
            self.x = x;
            for i in 0..self.n {
                if !self.dead[i] {
                    self.tempdiffx[2 * i] =
                        self.delta_t * (1.5 * self.diffx[2 * i] - 0.5 * self.olddiffx[2 * i]);
                    self.tempdiffx[2 * i + 1] = self.delta_t
                        * (1.5 * self.diffx[2 * i + 1] - 0.5 * self.olddiffx[2 * i + 1]);
                }
            }
            self.perturb(false);
            std::mem::swap(&mut self.olddiffx, &mut self.diffx);
        }
    }

    /// Where a point is drawn: through the polynomial when the boundary is
    /// bent, and straight onto the circle when it is not.
    fn screen_pos(&self, b: usize) -> (i32, i32) {
        let (x, y) = if self.variable_boundary {
            (
                self.p[2 * b] * self.scale + self.xshift,
                self.p[2 * b + 1] * self.scale + self.yshift,
            )
        } else {
            (
                self.x[2 * b] * self.radius + self.width as f64 / 2.0,
                self.x[2 * b + 1] * self.radius + self.height as f64 / 2.0,
            )
        };
        (x as i32 as i16 as i32, y as i32 as i16 as i32)
    }

    /// Append this frame's segment for point `b`: from where it was drawn last
    /// frame to where it is now.
    fn push_seg(&mut self, b: usize) {
        let (x2, y2) = self.screen_pos(b);
        let k = self.cnsegs;
        self.csegs[k].x1 = self.lastx[2 * b] as i32;
        self.csegs[k].y1 = self.lastx[2 * b + 1] as i32;
        self.csegs[k].x2 = x2;
        self.csegs[k].y2 = y2;
        self.lastx[2 * b] = x2 as i16;
        self.lastx[2 * b + 1] = y2 as i16;
        self.cnsegs += 1;
    }
}

impl Screenhack for Euler2d {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.ode_solve();
        if self.variable_boundary {
            self.calc_all_p();
        }

        self.cnsegs = 0;
        for b in self.nvortex..self.n {
            if !self.dead[b] {
                self.push_seg(b);
            }
        }
        let n_non_vortex_segs = self.cnsegs;

        if !self.hide_vortex {
            for b in 0..self.nvortex {
                if !self.dead[b] {
                    self.push_seg(b);
                }
            }
        }

        if self.count != 0 {
            let slot = self.c_old_seg * self.n;
            let nold = self.nold_segs[self.c_old_seg];
            self.mi.gc.set_foreground(self.mi.black);
            d.win()
                .draw_segments(&self.mi.gc, &self.old_segs[slot..slot + nold]);

            let npixels = self.mi.npixels();
            if npixels > 2 {
                for col in 0..npixels {
                    let start = (col * n_non_vortex_segs as i32 / npixels) as usize;
                    let finish = ((col + 1) * n_non_vortex_segs as i32 / npixels) as usize;
                    self.mi.gc.set_foreground(self.mi.pixel(col as usize));
                    d.win()
                        .draw_segments(&self.mi.gc, &self.csegs[start..finish]);
                }
                if !self.hide_vortex {
                    self.mi.gc.set_foreground(self.mi.white);
                    d.win()
                        .draw_segments(&self.mi.gc, &self.csegs[n_non_vortex_segs..self.cnsegs]);
                }
            } else {
                self.mi.gc.set_foreground(self.mi.white);
                d.win()
                    .draw_segments(&self.mi.gc, &self.csegs[..self.cnsegs]);
            }

            if npixels > 2 {
                self.mi
                    .gc
                    .set_foreground(self.mi.pixel(self.boundary_color));
            } else {
                self.mi.gc.set_foreground(self.mi.white);
            }
            if self.variable_boundary {
                d.win().draw_segments(&self.mi.gc, &self.boundary);
            } else {
                let r = self.radius as i32;
                d.win().draw_arc(
                    &self.mi.gc,
                    self.width / 2 - r - 1,
                    self.height / 2 - r - 1,
                    2 * r + 2,
                    2 * r + 2,
                    0,
                    64 * 360,
                );
            }

            // Copy to erase-list.
            self.old_segs[slot..slot + self.cnsegs].copy_from_slice(&self.csegs[..self.cnsegs]);
            self.nold_segs[self.c_old_seg] = self.cnsegs;
            self.c_old_seg += 1;
            if self.c_old_seg >= self.tail_len {
                self.c_old_seg = 0;
            }
        }

        self.count += 1;
        if self.count > self.mi.cycles {
            self.restart(d); // Pick a new flow.
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.restart(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 10000",
    "*count: 1024",
    "*cycles: 3000",
    "*ncolors: 64",
    "*fpsSolid: true",
    "*ignoreRotation: True",
    "*eulertail: 10",
    "*eulerpower: 1",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Particles", 2.0, 5000.0, 1.0, 0, "1024"),
    Opt::slider("eulertail", "Trail length", 2.0, 500.0, 1.0, 0, "10"),
    Opt::slider("cycles", "Duration", 100.0, 5000.0, 10.0, 0, "3000"),
    Opt::slider("ncolors", "Number of colors", 2.0, 255.0, 1.0, 0, "64"),
    Opt::slider("eulerpower", "Interaction power", 0.5, 3.0, 0.1, 1, "1"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "euler2d",
    label: "Euler 2D",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Stephen Montgomery-Smith",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=ZH1ZtfId0iA"),
        blurb: "Simulates two dimensional incompressible inviscid fluid flow.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
