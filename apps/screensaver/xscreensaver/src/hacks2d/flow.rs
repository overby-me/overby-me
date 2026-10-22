//! Port of `hacks/flow.c`.
//!
//! ```text
//! flow --- flow of strange bees
//!
//! Copyright (c) 1996 by Tim Auckland <tda10.geo@yahoo.com>
//! Incorporating some code from Stephen Davies Copyright (c) 2000
//!
//! Search code based on techniques described in "Strange Attractors:
//! Creating Patterns in Chaos" by Julien C. Sprott
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
//! "flow" shows a variety of continuous phase-space flows around strange
//! attractors.  It includes the well-known Lorentz mask (the "Butterfly"
//! of chaos fame), two forms of Rossler's "Folded Band" and Poincare'
//! sections of the "Birkhoff Bagel" and Duffing's forced occilator.  "flow"
//! can now discover new attractors.
//! ```
//!
//! Thousands of particles are dropped into a three-dimensional flow and left to
//! be carried by it. None of them are told where to go: each one just steps
//! along whatever the differential equation says the velocity is where it
//! happens to be. Where they end up is the attractor, drawn by the traffic
//! rather than by any code that knows its shape.
//!
//! The equation itself is one generic cubic polynomial in three variables,
//! twenty terms per component. Lorentz, both Rosslers, Birkhoff's bagel and
//! Duffing's oscillator are all just different sets of coefficients for it, so
//! nothing here is special-cased to any of them.
//!
//! That generality is what lets the hack look for attractors nobody has named.
//! In the background it rolls a random set of coefficients, flies two particles
//! a hair apart through them for a few thousand steps, and measures how fast the
//! two separate. Divergence that stays bounded is the definition of a strange
//! attractor: fly apart and the flow exploded, converge and it fell into a fixed
//! point or a plain loop. Only a positive Lyapunov exponent with a finite
//! bounding box gets promoted to the one on screen.
//!
//! The camera is two points and a wing, flown rather than positioned. It either
//! orbits the bounding box or rides on the back of the first particle, and it
//! eases between the two over a hundred frames rather than cutting. When it is
//! riding, particle one is not in the flow at all: it is moved each frame to
//! wherever the camera's up-vector needs it, and is hidden.
//!
//! Upstream's double-buffer switch is not a knob here. It picks between clearing
//! a pixmap and erasing last frame's segments in place, and the framebuffer this
//! draws into is blitted whole once a frame either way.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, MAXRAND, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XSegment};

/// Enough for a full cubic, or a periodic cubic.
const N_PARS: usize = 20;

// The parameter indices, named so the standard examples read like their
// published formulae.
const C: usize = 0;
const X: usize = 1;
const XX: usize = 2;
const XXX: usize = 3;
const XXY: usize = 4;
const XXZ: usize = 5;
const XY: usize = 6;
const XYY: usize = 7;
const XYZ: usize = 8;
const XZ: usize = 9;
const XZZ: usize = 10;
const Y: usize = 11;
const YY: usize = 12;
const YYY: usize = 13;
const YYZ: usize = 14;
const YZ: usize = 15;
const YZZ: usize = 16;
const Z: usize = 17;
const ZZ: usize = 18;
const ZZZ: usize = 19;
/// The sine term shares a slot with `XY`, which the periodic form never uses.
const SINY: usize = XY;

const LOST_IN_SPACE: f64 = 2000.0;
const INITIALSTEP: f64 = 0.04;
const EYEHEIGHT: f64 = 0.005;
const MINTRAIL: i32 = 2;
/// How many lines the bounding box is drawn with.
const BOX_L: usize = 36;

/// The corners of the box, and the little brackets inset from each face.
const BOX: [[f64; 3]; 32] = [
    [1.0, 1.0, 1.0],
    [1.0, 1.0, -1.0],
    [1.0, -1.0, -1.0],
    [1.0, -1.0, 1.0],
    [-1.0, 1.0, 1.0],
    [-1.0, 1.0, -1.0],
    [-1.0, -1.0, -1.0],
    [-1.0, -1.0, 1.0],
    [1.0, 0.8, 0.8],
    [1.0, 0.8, -0.8],
    [1.0, -0.8, -0.8],
    [1.0, -0.8, 0.8],
    [0.8, 1.0, 0.8],
    [0.8, 1.0, -0.8],
    [-0.8, 1.0, -0.8],
    [-0.8, 1.0, 0.8],
    [0.8, 0.8, 1.0],
    [0.8, -0.8, 1.0],
    [-0.8, -0.8, 1.0],
    [-0.8, 0.8, 1.0],
    [-1.0, 0.8, 0.8],
    [-1.0, 0.8, -0.8],
    [-1.0, -0.8, -0.8],
    [-1.0, -0.8, 0.8],
    [0.8, -1.0, 0.8],
    [0.8, -1.0, -0.8],
    [-0.8, -1.0, -0.8],
    [-0.8, -1.0, 0.8],
    [0.8, 0.8, -1.0],
    [0.8, -0.8, -1.0],
    [-0.8, -0.8, -1.0],
    [-0.8, 0.8, -1.0],
];

/// Which corners each of the box's lines joins.
const LINES: [[usize; 2]; BOX_L] = [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 0],
    [4, 5],
    [5, 6],
    [6, 7],
    [7, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
    [8, 9],
    [9, 10],
    [10, 11],
    [11, 8],
    [12, 13],
    [13, 14],
    [14, 15],
    [15, 12],
    [16, 17],
    [17, 18],
    [18, 19],
    [19, 16],
    [20, 21],
    [21, 22],
    [22, 23],
    [23, 20],
    [24, 25],
    [25, 26],
    [26, 27],
    [27, 24],
    [28, 29],
    [29, 30],
    [30, 31],
    [31, 28],
];

#[derive(Clone, Copy, Default, PartialEq)]
struct DVector {
    x: f64,
    y: f64,
    z: f64,
}

type Par = [DVector; N_PARS];

/// Where the camera is trying to get to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Chaseto {
    Orbit,
    Bee,
}

/// Which of the two forms of the equation is in use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ode {
    Cubic,
    Periodic,
}

/// `balance_rand(v)`: uniform on `-v/2..v/2`.
fn balance_rand(v: f64) -> f64 {
    lrand() as f64 / MAXRAND * v - v / 2.0
}

/// The twenty monomials of a cubic in three variables, in the order the
/// parameter slots are named.
fn monomials(x: f64, y: f64, z: f64) -> [f64; N_PARS] {
    let mut m = [0.0; N_PARS];
    m[C] = 1.0;
    m[X] = x;
    m[XX] = x * x;
    m[XXX] = x * x * x;
    m[XXY] = x * x * y;
    m[XXZ] = x * x * z;
    m[XY] = x * y;
    m[XYY] = x * y * y;
    m[XYZ] = x * y * z;
    m[XZ] = x * z;
    m[XZZ] = x * z * z;
    m[Y] = y;
    m[YY] = y * y;
    m[YYY] = y * y * y;
    m[YYZ] = y * y * z;
    m[YZ] = y * z;
    m[YZZ] = y * z * z;
    m[Z] = z;
    m[ZZ] = z * z;
    m[ZZZ] = z * z * z;
    m
}

/// The generic 3D cubic polynomial. It includes all the quadratics (Lorentz,
/// Rossler) and much more; upstream notes that a separate quadratic path would
/// not measurably help, because the drawing dominates.
fn cubic(a: &Par, x: f64, y: f64, z: f64) -> DVector {
    let m = monomials(x, y, z);
    let mut d = DVector::default();
    for i in 0..N_PARS {
        d.x += a[i].x * m[i];
        d.y += a[i].y * m[i];
        d.z += a[i].z * m[i];
    }
    d
}

/// A 3D cubic in x and z with a periodic forcing term in x. y is the
/// independent periodic time axis, which is what makes Birkhoff's bagel and
/// Duffing's attractor come out as Poincare sections.
fn periodic(a: &Par, x: f64, y: f64, z: f64) -> DVector {
    // Only the terms in x and z, and only for the x and z components.
    const TERMS: [usize; 10] = [C, X, XX, XXX, XXZ, XZ, XZZ, Z, ZZ, ZZZ];
    let m = monomials(x, y, z);
    let mut d = DVector::default();
    for i in TERMS {
        d.x += a[i].x * m[i];
        d.z += a[i].z * m[i];
    }
    d.x += a[SINY].x * y.sin();
    d.y = a[C].y;
    d
}

/// Second-order Runge-Kutta. Returns the squared length of the step, so the
/// caller can tell when the step size wants reducing.
fn iterate(p: &mut DVector, ode: Ode, par: &Par, step: f64) -> f64 {
    let f = |a: &Par, x, y, z| match ode {
        Ode::Cubic => cubic(a, x, y, z),
        Ode::Periodic => periodic(a, x, y, z),
    };
    let mut k1 = f(par, p.x, p.y, p.z);
    k1.x *= step;
    k1.y *= step;
    k1.z *= step;
    let mut k2 = f(par, p.x + k1.x, p.y + k1.y, p.z + k1.z);
    k2.x *= step;
    k2.y *= step;
    k2.z *= step;
    let k3 = DVector {
        x: (k1.x + k2.x) / 2.0,
        y: (k1.y + k2.y) / 2.0,
        z: (k1.z + k2.z) / 2.0,
    };
    p.x += k3.x;
    p.y += k3.y;
    p.z += k3.z;
    k3.x * k3.x + k3.y * k3.y + k3.z * k3.z
}

/// A Gaussian of mean nought whose "amplitude" is three standard deviations.
/// The Box-Muller transform makes two at a time, so one is kept back.
#[derive(Default)]
struct Gauss {
    spare: f64,
    ready: bool,
}

impl Gauss {
    fn rand(&mut self, a: f64) -> f64 {
        if self.ready {
            self.ready = false;
            return a / 3.0 * self.spare;
        }
        let (mut x, mut y, mut w);
        loop {
            x = 2.0 * lrand() as f64 / MAXRAND - 1.0;
            y = 2.0 * lrand() as f64 / MAXRAND - 1.0;
            w = x * x + y * y;
            if w < 1.0 {
                break;
            }
        }
        w = ((-2.0 * w.ln()) / w).sqrt();
        self.ready = true;
        self.spare = x * w;
        a / 3.0 * y * w
    }
}

struct Flow {
    mi: ModeInfo,
    gauss: Gauss,

    /// The camera's flight path: head, tail and wing.
    cam: [DVector; 3],
    chasetime: i32,
    chaseto: Chaseto,
    /// The viewpoint that circles the scene, this frame and last.
    circle: [DVector; 2],
    centre: DVector,
    beecount: usize,
    /// The segments to draw, one bucket per colour.
    csegs: Vec<Vec<XSegment>>,
    taillen: usize,

    ode: Ode,
    range: DVector,
    yperiod: f64,

    par: Par,
    /// Bee positions, `p[t + b * taillen]`, newest first.
    p: Vec<DVector>,
    count: i64,
    size: f64,
    mid: DVector,
    step: f64,

    /// The second set, which the background search flies in parallel.
    par2: Par,
    p2: [DVector; 2],
    count2: i64,
    lyap2: f64,
    size2: f64,
    mid2: DVector,
    step2: f64,

    rotatep: bool,
    ridep: bool,
    boxp: bool,
    periodicp: bool,
    searchp: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS is not defined, so the default random map.
    let mi = ModeInfo::new(d, ColorScheme::Random);
    let mut st = Flow {
        mi,
        gauss: Gauss::default(),
        cam: [DVector::default(); 3],
        chasetime: 1,
        chaseto: Chaseto::Orbit,
        circle: [DVector::default(); 2],
        centre: DVector::default(),
        beecount: 1,
        csegs: Vec::new(),
        taillen: MINTRAIL as usize,
        ode: Ode::Cubic,
        range: DVector::default(),
        yperiod: 0.0,
        par: [DVector::default(); N_PARS],
        p: Vec::new(),
        count: 0,
        size: 1.0,
        mid: DVector::default(),
        step: INITIALSTEP,
        par2: [DVector::default(); N_PARS],
        p2: [DVector::default(); 2],
        count2: 0,
        lyap2: 0.0,
        size2: 1.0,
        mid2: DVector::default(),
        step2: INITIALSTEP,
        rotatep: d.res.bool("rotate"),
        ridep: d.res.bool("ride"),
        boxp: d.res.bool("box"),
        periodicp: d.res.bool("periodic"),
        searchp: d.res.bool("search"),
    };
    st.restart(d);
    Box::new(st)
}

impl Flow {
    /// `B(t, b)`: where a bee was `t` frames ago.
    fn bee(&self, t: usize, b: usize) -> DVector {
        self.p[t + b * self.taillen]
    }
    fn bee_mut(&mut self, t: usize, b: usize) -> &mut DVector {
        let i = t + b * self.taillen;
        &mut self.p[i]
    }

    /// `init_flow`: pick one of the published attractors, measure it, and drop
    /// the swarm into it.
    fn restart(&mut self, d: &mut Dpy) {
        self.count2 = 0;

        let mut taillen = self.mi.size;
        if taillen < -MINTRAIL {
            // Change by the square root so it seems more variable.
            let n = nrand(((-taillen - MINTRAIL + 1) as f64).sqrt() as i32);
            taillen = n * n + MINTRAIL;
        } else if taillen < MINTRAIL {
            taillen = MINTRAIL;
        }
        self.taillen = taillen as usize;

        if !self.rotatep && !self.ridep {
            self.rotatep = true; // We need at least one viewpoint.
        }
        self.chaseto = if self.rotatep {
            Chaseto::Orbit
        } else {
            Chaseto::Bee
        };
        self.chasetime = 1; // Go directly to the target.

        self.yperiod = 0.0;
        self.step2 = INITIALSTEP;
        self.par2 = [DVector::default(); N_PARS];

        // The published examples, as coefficients of the one generic form.
        match nrand(if self.periodicp { 5 } else { 3 }) {
            0 => {
                // Lorentz: x' = a(y - x), y' = x(b - z) - y, z' = xy - cz
                self.par2[Y].x = 10.0;
                self.par2[X].x = -self.par2[Y].x;
                self.par2[X].y = 28.0;
                self.par2[XZ].y = -1.0;
                self.par2[Y].y = -1.0;
                self.par2[XY].z = 1.0;
                self.par2[Z].z = -2.0;
            }
            1 => {
                // Rossler: x' = -(y + az), y' = x + by, z' = c + z(x - 5.7)
                self.par2[Y].x = -1.0;
                self.par2[Z].x = -2.0 + balance_rand(1.0);
                self.par2[X].y = 1.0;
                self.par2[Y].y = 0.2 + balance_rand(0.1);
                self.par2[C].z = 0.2 + balance_rand(0.1);
                self.par2[XZ].z = 1.0;
                self.par2[Z].z = -5.7;
            }
            2 => {
                // RosslerCone: as Rossler, with a z-squared term in y.
                self.par2[Y].x = -1.0;
                self.par2[Z].x = -2.0;
                self.par2[X].y = 1.0;
                self.par2[Y].y = 0.2;
                self.par2[ZZ].y = -0.331 + balance_rand(0.01);
                self.par2[C].z = 0.2;
                self.par2[XZ].z = 1.0;
                self.par2[Z].z = -5.7;
            }
            3 => {
                // Birkhoff: x' = -z + b sin(y), y' = c, z' = 0.7x + az(0.1 - x^2)
                self.par2[Z].x = -1.0;
                self.par2[SINY].x = 0.35 + balance_rand(0.25);
                self.par2[C].y = 1.57;
                self.par2[X].z = 0.7;
                self.par2[Z].z = 1.0 + balance_rand(0.5);
                self.par2[XXZ].z = -10.0 * self.par2[Z].z;
                self.yperiod = std::f64::consts::TAU;
            }
            _ => {
                // Duffing: x' = -ax - z/2 - z^3/8 + b sin(y), y' = c, z' = 2x
                self.par2[X].x = -0.2 + balance_rand(0.1);
                self.par2[Z].x = -0.5;
                self.par2[ZZZ].x = -0.125;
                self.par2[SINY].x = 27.0 + balance_rand(3.0);
                self.par2[C].y = 1.33;
                self.par2[X].z = 2.0;
                self.yperiod = std::f64::consts::TAU;
            }
        }

        self.range.x = 5.0;
        self.range.z = 5.0;
        if self.yperiod > 0.0 {
            self.ode = Ode::Periodic;
            // A periodic flow shows either a uniform distribution or a
            // snapshot on the time axis.
            self.range.y = if nrand(2) != 0 { self.yperiod } else { 0.0 };
        } else {
            self.range.y = 5.0;
            self.ode = Ode::Cubic;
        }

        // Run the discoverer once to set up the bounding box. The Lyapunov
        // exponent will be inaccurate from a single pass, but these are known
        // strange attractors.
        self.discover();
        self.install();
        self.count2 = 0; // Reset the search.

        self.beecount = match self.mi.count {
            0 => 1, // The camera requires one or more.
            n if n < 0 => (nrand(-n) + 1) as usize,
            n => n as usize,
        };

        let npixels = self.mi.npixels().max(1) as usize;
        self.csegs = vec![Vec::new(); npixels];
        self.p = vec![DVector::default(); self.beecount * self.taillen];

        self.mi
            .gc
            .set_line_width(if self.mi.width > 2560 || self.mi.height > 2560 {
                3 // Retina displays.
            } else {
                1
            });
        self.mi.clear_window(d);

        self.restart_flow();

        // Set up the camera tail.
        *self.bee_mut(1, 0) = DVector::default();
        self.cam[1] = DVector::default();
    }

    /// Fresh initial conditions, without the rest of the setup.
    fn restart_flow(&mut self) {
        self.count = 0;
        for b in 0..self.beecount {
            let v = DVector {
                x: self.gauss.rand(self.range.x),
                y: if self.yperiod > 0.0 {
                    balance_rand(self.range.y)
                } else {
                    self.gauss.rand(self.range.y)
                },
                z: self.gauss.rand(self.range.z),
            };
            *self.bee_mut(0, b) = v;
        }
    }

    fn install(&mut self) {
        self.size = self.size2;
        self.mid = self.mid2;
        self.step = self.step2;
        self.par = self.par2;
    }

    /// Fly a pair of particles a hair apart through the second parameter set
    /// and measure how fast they separate. False means the flow exploded.
    fn discover(&mut self) -> bool {
        let lost = |p: &DVector| {
            p.x.abs() > LOST_IN_SPACE || p.y.abs() > LOST_IN_SPACE || p.z.abs() > LOST_IN_SPACE
        };

        if self.count2 == 0 {
            self.p2[0] = DVector {
                x: self.gauss.rand(self.range.x),
                y: if self.yperiod > 0.0 {
                    balance_rand(self.range.y)
                } else {
                    self.gauss.rand(self.range.y)
                },
                z: self.gauss.rand(self.range.z),
            };

            // A thousand steps to find an attractor. Most cases explode here.
            for _ in 0..1000 {
                iterate(&mut self.p2[0], self.ode, &self.par2, self.step2);
                if self.yperiod > 0.0 && self.p2[0].y > self.yperiod {
                    self.p2[0].y -= self.yperiod;
                }
                if lost(&self.p2[0]) {
                    return false;
                }
                self.count2 += 1;
            }
            // A small perturbation.
            self.p2[1] = self.p2[0];
            self.p2[1].x += 0.000_001;
        }

        let mut max = self.p2[0];
        let mut min = self.p2[0];
        let (mut lsum, mut nl, mut l, mut maxv2) = (0.0, 0i64, 0.0, 0.0);

        for _ in 0..5000 {
            for i in 0..2 {
                let v2 = iterate(&mut self.p2[i], self.ode, &self.par2, self.step2);
                if self.yperiod > 0.0 && self.p2[i].y > self.yperiod {
                    self.p2[i].y -= self.yperiod;
                }
                if lost(&self.p2[i]) {
                    return false;
                }
                if v2 > maxv2 {
                    maxv2 = v2;
                }
            }

            let p = self.p2[0];
            if p.x < min.x {
                min.x = p.x;
            } else if p.x > max.x {
                max.x = p.x;
            }
            if p.y < min.y {
                min.y = p.y;
            } else if p.y > max.y {
                max.y = p.y;
            }
            if p.z < min.z {
                min.z = p.z;
            } else if p.z > max.z {
                max.z = p.z;
            }

            // How hard the two have to be pulled back together is the measure.
            let dl = DVector {
                x: self.p2[1].x - self.p2[0].x,
                y: self.p2[1].y - self.p2[0].y,
                z: self.p2[1].z - self.p2[0].z,
            };
            let dl2 = dl.x * dl.x + dl.y * dl.y + dl.z * dl.z;
            if dl2 > 0.0 {
                let df = 1e12 * dl2;
                let rs = 1.0 / df.sqrt();
                self.p2[1].x = self.p2[0].x + rs * dl.x;
                self.p2[1].y = self.p2[0].y + rs * dl.y;
                self.p2[1].z = self.p2[0].z + rs * dl.z;
                lsum += df.ln();
                nl += 1;
                l = std::f64::consts::LOG2_E / 2.0 * lsum / nl as f64 / self.step2;
            }
            self.count2 += 1;
        }

        // Anything that did not explode has a finite attractor. A negative
        // exponent means it found a fixed point or a limit cycle instead.
        self.lyap2 = l;
        self.size2 = (max.x - min.x).max(max.y - min.y).max(max.z - min.z);
        self.mid2 = DVector {
            x: (max.x + min.x) / 2.0,
            y: (max.y + min.y) / 2.0,
            z: (max.z + min.z) / 2.0,
        };

        if maxv2.sqrt() > self.size2 * 0.2 {
            // Flowing too fast: reduce the step size. This eliminates the
            // high-speed limit cycles that show a positive exponent purely
            // through integration error.
            self.step2 /= 2.0;
        }
        true
    }

    /// True if the line was wholly behind the plane, having clipped it to the
    /// plane otherwise. `n` is the plane's normal and `d` its distance from
    /// the origin.
    fn clip(nx: f64, ny: f64, nz: f64, d: f64, s: &mut DVector, e: &mut DVector) -> bool {
        let front1 = nx * s.x + ny * s.y + nz * s.z >= -d;
        let front2 = nx * e.x + ny * e.y + nz * e.z >= -d;
        if !front1 && !front2 {
            return true;
        }
        if front1 && front2 {
            return false;
        }
        let w = DVector {
            x: e.x - s.x,
            y: e.y - s.y,
            z: e.z - s.z,
        };
        let t = (-d - nx * s.x - ny * s.y - nz * s.z) / (nx * w.x + ny * w.y + nz * w.z);
        let p = DVector {
            x: s.x + w.x * t,
            y: s.y + w.y * t,
            z: s.z + w.z * t,
        };
        if front2 {
            *s = p
        } else {
            *e = p
        }
        false
    }
}

impl Screenhack for Flow {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (w, h) = (self.mi.width, self.mi.height);
        let npixels = self.mi.npixels();
        let ncols = (npixels - 1).max(1);
        let mut swarm = 0;

        if self.searchp {
            if self.count2 == 0 {
                self.step2 = INITIALSTEP;
                // The actual range is irrelevant: the parameter scale sets the
                // flow's speed but not its structure.
                for i in 0..N_PARS {
                    self.par2[i].x = self.gauss.rand(1.0);
                    self.par2[i].y = self.gauss.rand(1.0);
                    self.par2[i].z = self.gauss.rand(1.0);
                }
            }
            if !self.discover() {
                self.count2 = 0; // The flow exploded.
            } else if self.lyap2 < 0.0 {
                self.count2 = 0; // An attractor, but not a strange one.
            } else if self.count2 > 1_000_000 {
                self.count2 = 0;
                self.install();
                // Zoom out, if allowed, for a look at the new attractor.
                if self.chaseto == Chaseto::Bee && self.rotatep {
                    self.chaseto = Chaseto::Orbit;
                    self.chasetime = 100;
                }
                // Reset the initial conditions so the particle density does
                // not carry a misleading artefact over.
                self.restart_flow();
            }
        }

        for c in &mut self.csegs {
            c.clear();
        }

        // The circling viewpoint.
        self.circle[1] = self.circle[0];
        let t = self.count as f64;
        self.circle[0] = DVector {
            x: self.size * 2.0 * (t / 100.0).sin() * (-0.6 + 0.4 * (t / 500.0).cos()) + self.mid.x,
            y: self.size * 2.0 * (t / 100.0).cos() * (0.6 + 0.4 * (t / 500.0).cos()) + self.mid.y,
            z: self.size * 2.0 * (t / 421.0).sin() + self.mid.z,
        };

        // A timed chase, rather than a bistable oscillator.
        if self.rotatep && self.ridep {
            if self.chaseto == Chaseto::Bee && nrand(1000) == 0 {
                self.chaseto = Chaseto::Orbit;
                self.chasetime = 100;
            } else if nrand(4000) == 0 {
                self.chaseto = Chaseto::Bee;
                self.chasetime = 100;
            }
        }

        // The orientation matrix, from the camera's three points.
        let mut m = [[0.0f64; 3]; 3];
        {
            // Chasetime guarantees the camera reaches its target in a finite
            // number of steps.
            if self.chasetime > 1 {
                self.chasetime -= 1;
            }
            let ct = self.chasetime as f64;

            if self.chaseto == Chaseto::Bee {
                let (b00, b10, b01) = (self.bee(0, 0), self.bee(1, 0), self.bee(0, 1));
                for (cam, target) in self.cam.iter_mut().zip([b00, b10, b01]) {
                    cam.x += (target.x - cam.x) / ct;
                    cam.y += (target.y - cam.y) / ct;
                    cam.z += (target.z - cam.z) / ct;
                }
            } else {
                // The head follows the orbiter, the tail sits diametrically
                // opposite the middle of the box from it, and the wing trails
                // where the orbiter was.
                let targets = [
                    self.circle[0],
                    DVector {
                        x: 2.0 * self.circle[0].x - self.mid.x,
                        y: 2.0 * self.circle[0].y - self.mid.y,
                        z: 2.0 * self.circle[0].z - self.mid.z,
                    },
                    self.circle[1],
                ];
                for (cam, target) in self.cam.iter_mut().zip(targets) {
                    cam.x += (target.x - cam.x) / ct;
                    cam.y += (target.y - cam.y) / ct;
                    cam.z += (target.z - cam.z) / ct;
                }
            }

            self.centre = self.cam[1]; // The viewpoint is the camera's tail.

            let x = [
                self.cam[0].x - self.cam[1].x,
                self.cam[0].y - self.cam[1].y,
                self.cam[0].z - self.cam[1].z,
            ];
            let p = [
                self.cam[2].x - self.cam[1].x,
                self.cam[2].y - self.cam[1].y,
                self.cam[2].z - self.cam[1].z,
            ];

            // So long as X and P do not collide, X, (X x P) x X and X x P are
            // three mutually orthogonal axes; normalised, they are the matrix.
            let (mut x2, mut xp) = (0.0, 0.0);
            for i in 0..3 {
                x2 += x[i] * x[i];
                xp += x[i] * p[i];
                m[0][i] = x[i];
            }
            for i in 0..3 {
                m[1][i] = x2 * p[i] - xp * x[i]; // (X . X) P - (X . P) X
            }
            m[2][0] = x[1] * p[2] - x[2] * p[1];
            m[2][1] = -x[0] * p[2] + x[2] * p[0];
            m[2][2] = x[0] * p[1] - x[1] * p[0];

            for row in &mut m {
                let a = (row[0] * row[0] + row[1] * row[1] + row[2] * row[2]).sqrt();
                if a > 0.0 {
                    for v in row.iter_mut() {
                        *v /= a;
                    }
                }
            }

            if self.chaseto == Chaseto::Bee && self.beecount > 1 {
                // Move the wing bee to wherever the up-vector needs it.
                let b0 = self.bee(0, 0);
                let step = self.step;
                *self.bee_mut(0, 1) = DVector {
                    x: b0.x + m[1][0] * step,
                    y: b0.y + m[1][1] * step,
                    z: b0.z + m[1][2] * step,
                };
            }
        }

        // The bounding box.
        if self.boxp {
            for (b, line) in LINES.iter().enumerate() {
                let corner = |k: usize| {
                    (
                        BOX[k][0] * self.size / 2.0 + self.mid.x - self.centre.x,
                        BOX[k][1] * self.size / 2.0 + self.mid.y - self.centre.y,
                        BOX[k][2] * self.size / 2.0 + self.mid.z - self.centre.z,
                    )
                };
                let view = |(x, y, z): (f64, f64, f64)| DVector {
                    x: m[0][0] * x + m[0][1] * y + m[0][2] * z,
                    y: m[1][0] * x + m[1][1] * y + m[1][2] * z,
                    z: m[2][0] * x + m[2][1] * y + m[2][2] * z + EYEHEIGHT * self.size,
                };
                let mut a1 = view(corner(line[0]));
                let mut a2 = view(corner(line[1]));

                // Clip in three dimensions before projecting: a flat clip
                // afterwards could not handle a line crossing x = 0.
                let aspect = 2.0 * w as f64 / h as f64;
                if Self::clip(1.0, 0.0, 0.0, -1.0, &mut a1, &mut a2)
                    || Self::clip(1.0, 2.0, 0.0, 0.0, &mut a1, &mut a2)
                    || Self::clip(1.0, -2.0, 0.0, 0.0, &mut a1, &mut a2)
                    || Self::clip(1.0, 0.0, aspect, 0.0, &mut a1, &mut a2)
                    || Self::clip(1.0, 0.0, -aspect, 0.0, &mut a1, &mut a2)
                {
                    continue;
                }

                let col = b % ncols as usize;
                self.csegs[col].push(XSegment {
                    x1: (w as f64 / 2.0 + w as f64 * a1.y / a1.x) as i32,
                    y1: (h as f64 / 2.0 + w as f64 * a1.z / a1.x) as i32,
                    x2: (w as f64 / 2.0 + w as f64 * a2.y / a2.x) as i32,
                    y2: (h as f64 / 2.0 + w as f64 * a2.z / a2.x) as i32,
                });
            }
        }

        // The bees.
        for b in 0..self.beecount {
            let here = self.bee(0, b);
            if here.x.abs() > LOST_IN_SPACE
                || here.y.abs() > LOST_IN_SPACE
                || here.z.abs() > LOST_IN_SPACE
            {
                if self.chaseto == Chaseto::Bee && b == 0 && self.beecount > 1 {
                    // The camera bee is lost. Rerunning init could throw away a
                    // hard-won new attractor, so move it very close to another
                    // bee instead, which is likely to be near the attractor and
                    // will not form a false artefact.
                    let newb = 1 + nrand(self.beecount as i32 - 1) as usize;
                    let mut v = self.bee(0, newb);
                    v.x += 0.001;
                    *self.bee_mut(0, 0) = v;
                }
                continue;
            }

            // Age the tail. This has to be fast: bees times tail can be large.
            let base = b * self.taillen;
            let n = self.taillen;
            self.p[base..base + n].copy_within(0..n - 1, 1);
            let (ode, par, step) = (self.ode, self.par, self.step);
            iterate(self.bee_mut(0, b), ode, &par, step);

            // The wing bee is not quite in the flow, so it is not drawn.
            if self.chaseto == Chaseto::Bee && b == 1 {
                continue;
            }

            let col = b % ncols as usize;
            let mut begin = 0usize;
            let end = self.taillen.min(self.count.max(0) as usize);
            let mut pending: Option<(i32, i32)> = None;

            for i in 0..end {
                let bp = self.bee(i, b);
                let yscale = if self.yperiod < 0.0 {
                    self.size / self.yperiod
                } else {
                    1.0
                };
                let (x, y, z) = (
                    bp.x - self.centre.x,
                    bp.y * yscale - self.centre.y,
                    bp.z - self.centre.z,
                );
                let xm = m[0][0] * x + m[0][1] * y + m[0][2] * z;
                let ym = m[1][0] * x + m[1][1] * y + m[1][2] * z;
                let zm = m[2][0] * x + m[2][1] * y + m[2][2] * z + EYEHEIGHT * self.size;

                swarm += 1;
                if self.yperiod > 0.0 && bp.y > self.yperiod {
                    let ny = bp.y - self.yperiod;
                    // Hide the tail, to prevent streaks along y. Streaks in x
                    // and z are welcome: they outline the Poincare slice.
                    for j in i..end {
                        self.bee_mut(j, b).y = ny;
                    }
                    break;
                }

                if xm <= 0.0 {
                    begin = i + 1; // Off screen: start a new trail.
                    pending = None;
                    continue;
                }
                let absx = (w as f64 / 2.0 + w as f64 * ym / xm) as i16 as i32;
                let absy = (h as f64 / 2.0 + w as f64 * zm / xm) as i16 as i32;
                if absx <= 0 || absx >= w || absy <= 0 || absy >= h {
                    begin = i + 1;
                    pending = None;
                    continue;
                }
                if i > begin
                    && let Some((px, py)) = pending
                {
                    self.csegs[col].push(XSegment {
                        x1: px,
                        y1: py,
                        x2: absx,
                        y2: absy,
                    });
                }
                if i < end - 1 {
                    pending = Some((absx, absy));
                }
            }
        }

        // Draw. The double-buffered path upstream clears and redraws, which is
        // what a framebuffer blitted whole every frame amounts to.
        self.mi.gc.set_foreground(self.mi.black);
        d.win().fill_rectangle(&self.mi.gc, 0, 0, w, h);

        if npixels > 2 {
            for col in 0..ncols as usize {
                if self.csegs[col].is_empty() {
                    continue;
                }
                self.mi.gc.set_foreground(self.mi.pixel(col + 1));
                d.win().draw_segments(&self.mi.gc, &self.csegs[col]);
            }
        } else {
            self.mi.gc.set_foreground(self.mi.white);
            d.win().draw_segments(&self.mi.gc, &self.csegs[0]);
        }

        if self.count > 1 && swarm == 0 {
            self.restart(d); // All gone.
        }
        self.count += 1;
        if self.count > self.mi.cycles as i64 {
            // Time is up: if nothing new has turned up by now, pick another
            // standard flow.
            self.restart(d);
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
    "*count: 3000",
    "*size: -10",
    "*cycles: 10000",
    "*ncolors: 200",
    "*rotate: True",
    "*ride: True",
    "*box: True",
    "*periodic: True",
    "*search: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Count", 10.0, 5000.0, 10.0, 0, "3000"),
    Opt::slider("cycles", "Timeout", 0.0, 800_000.0, 1000.0, 0, "10000"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
    Opt::slider("size", "Length of trails", -20.0, -2.0, 1.0, 0, "-10").inverted(),
    Opt::boolean("rotate", "Rotating around attractor", "True"),
    Opt::boolean("ride", "Ride in the flow", "True"),
    Opt::boolean("box", "Draw bounding box", "True"),
    Opt::boolean("periodic", "Periodic attractors", "True"),
    Opt::boolean("search", "Search for new attractors", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "flow",
    label: "Flow",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tim Auckland",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=RJjbRV0FC_A"),
        blurb: "Strange attractors formed of flows in a 3D differential equation phase space.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
