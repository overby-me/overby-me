//! Port of `hacks/galaxy.c`.
//!
//! ```text
//! Originally done by Uli Siegmund <uli@wombat.okapi.sub.org> on Amiga
//!   for EGS in Cluster
//! Port from Cluster/EGS to C/Intuition by Harald Backert
//! Port to X11 and incorporation into xlockmore by Hubert Feyrer
//!   <hubert.feyrer@rz.uni-regensburg.de>
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
//! 26-Aug-00: robert.nagtegaal@phil.uu.nl and roland@tschai.demon.nl:
//!            various improvements
//! 10-May-97: jwz@jwz.org: turned into a standalone program.
//! 18-Apr-97: Memory leak fixed by Tom Schmidt <tschmidt@micron.com>
//! 07-Apr-97: Modified by Dave Mitchell <davem@magnet.com>
//! 23-Oct-94: Modified by David Bagley <bagleyd@bigfoot.com>
//!  random star sizes
//!  colors change depending on velocity
//! 10-Oct-94: Add colors by Hubert Feyer
//! 30-Sep-94: Initial port by Hubert Feyer
//! 09-Mar-94: VMS can generate a random number 0.0 which results in a
//!            division by zero, corrected by Jouk Jansen
//!            <joukj@crys.chem.uva.nl>
//! ```
//!
//! Colliding galaxies. Each is a disc of a couple of thousand stars on
//! circular orbits, and the stars feel the gravity of every galaxy's centre
//! but not of each other, so a couple of thousand bodies cost only a couple of
//! thousand force calculations a frame. The galaxy centres do attract one
//! another, which is what pulls the discs into tidal tails as they pass.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XRectangle, frand};

const MIN_GALAXIES: i32 = 2;
const MAX_STARS: i32 = 3000;
const MAX_IDELTAT: f64 = 50.0;
const EPSILON: f64 = 0.00000001;
const SQRT_EPSILON: f64 = 0.0001;
const DELTAT: f64 = MAX_IDELTAT * 0.0001;

const GALAXY_RANGE_SIZE: f64 = 0.1;
const GALAXY_MIN_SIZE: f64 = 0.15;
const QCONS: f64 = 0.001;

/// Colours per galaxy.
const COLORBASE: i32 = 16;

#[derive(Clone, Copy, Default)]
struct Star {
    pos: [f64; 3],
    vel: [f64; 3],
}

struct GalaxyBody {
    mass: i32,
    stars: Vec<Star>,
    oldpoints: Vec<XRectangle>,
    newpoints: Vec<XRectangle>,
    pos: [f64; 3],
    vel: [f64; 3],
    galcol: i32,
}

struct Galaxy {
    mi: ModeInfo,
    /// The frame each galaxy's disc is laid out in.
    mat: [[f64; 3]; 3],
    scale: f64,
    midx: i32,
    midy: i32,
    size: f64,
    galaxies: Vec<GalaxyBody>,
    f_hititerations: i32,
    step: i32,
    pscale: i32,
    spin: bool,
    /// Rotation of the eye around the centre of the universe.
    rot_y: f64,
    rot_x: f64,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // UNIFORM_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Uniform);
    let mut pscale = 1;
    let mut scale = (mi.width + mi.height) as f64 / 8.0;
    if mi.width > 2560 || mi.height > 2560 {
        // Retina displays.
        pscale *= 2;
        scale /= pscale as f64;
    }

    let mut st = Galaxy {
        f_hititerations: mi.cycles,
        scale,
        midx: mi.width / 2,
        midy: mi.height / 2,
        pscale,
        spin: d.res.bool("spin"),
        mat: [[0.0; 3]; 3],
        size: 0.0,
        galaxies: Vec::new(),
        step: 0,
        rot_y: 0.0,
        rot_x: 0.0,
        mi,
    };
    st.startover(d);
    Box::new(st)
}

impl Galaxy {
    fn startover(&mut self, d: &mut Dpy) {
        self.step = 0;
        self.rot_y = 0.0;
        self.rot_x = 0.0;

        let mut ngalaxies = self.mi.count;
        if ngalaxies < -MIN_GALAXIES {
            ngalaxies = nrand(-ngalaxies - MIN_GALAXIES + 1) + MIN_GALAXIES;
        } else if ngalaxies < MIN_GALAXIES {
            ngalaxies = MIN_GALAXIES;
        }

        self.galaxies = Vec::with_capacity(ngalaxies as usize);
        for _ in 0..ngalaxies {
            // No all-green galaxies, though one may still have green stars.
            let mut galcol = nrand(COLORBASE - 2);
            if galcol > 1 {
                galcol += 2;
            }

            let nstars = (nrand(MAX_STARS / 2) + MAX_STARS / 2) as usize;

            let w1 = 2.0 * std::f64::consts::PI * frand(1.0);
            let w2 = 2.0 * std::f64::consts::PI * frand(1.0);
            let (sinw1, cosw1) = (w1.sin(), w1.cos());
            let (sinw2, cosw2) = (w2.sin(), w2.cos());

            self.mat = [
                [cosw2, -sinw1 * sinw2, cosw1 * sinw2],
                [0.0, cosw1, sinw1],
                [-sinw2, -sinw1 * cosw2, cosw1 * cosw2],
            ];

            let vel = [
                frand(1.0) * 2.0 - 1.0,
                frand(1.0) * 2.0 - 1.0,
                frand(1.0) * 2.0 - 1.0,
            ];
            let hit = self.f_hititerations as f64;
            let pos = [
                -vel[0] * DELTAT * hit + frand(1.0) - 0.5,
                -vel[1] * DELTAT * hit + frand(1.0) - 0.5,
                -vel[2] * DELTAT * hit + frand(1.0) - 0.5,
            ];
            let mass = (frand(1.0) * 1000.0) as i32 + 1;
            self.size = GALAXY_RANGE_SIZE * frand(1.0) + GALAXY_MIN_SIZE;

            let mut stars = Vec::with_capacity(nstars);
            for _ in 0..nstars {
                let w = 2.0 * std::f64::consts::PI * frand(1.0);
                let (sinw, cosw) = (w.sin(), w.cos());
                let dist = frand(1.0) * self.size;
                // The disc is thin, and thinner further out.
                let mut h = frand(1.0) * (-2.0 * (dist / self.size)).exp() / 5.0 * self.size;
                if frand(1.0) < 0.5 {
                    h = -h;
                }

                let mut s = Star::default();
                for (k, sp) in s.pos.iter_mut().enumerate() {
                    *sp = self.mat[0][k] * dist * cosw
                        + self.mat[1][k] * dist * sinw
                        + self.mat[2][k] * h
                        + pos[k];
                }

                // Circular orbit speed at this radius.
                let v = (mass as f64 * QCONS / (dist * dist + h * h).sqrt()).sqrt();
                for (k, sv) in s.vel.iter_mut().enumerate() {
                    *sv =
                        (-self.mat[0][k] * v * sinw + self.mat[1][k] * v * cosw + vel[k]) * DELTAT;
                }
                stars.push(s);
            }

            let blank = XRectangle {
                x: 0,
                y: 0,
                width: self.pscale,
                height: self.pscale,
            };
            self.galaxies.push(GalaxyBody {
                mass,
                oldpoints: vec![blank; nstars],
                newpoints: vec![blank; nstars],
                stars,
                pos,
                vel,
                galcol,
            });
        }

        d.clear_window();
    }
}

impl Screenhack for Galaxy {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.spin {
            self.rot_y += 0.01;
            self.rot_x += 0.004;
        }

        let (cox, six) = (self.rot_y.cos(), self.rot_y.sin());
        let (cor, sir) = (self.rot_x.cos(), self.rot_x.sin());
        let eps = 1.0 / (EPSILON * SQRT_EPSILON * DELTAT * DELTAT * QCONS);

        let ngalaxies = self.galaxies.len();

        for i in 0..ngalaxies {
            {
                // Read live: the galaxies before this one have already moved
                // this frame, as they have upstream.
                let centres: Vec<([f64; 3], i32)> =
                    self.galaxies.iter().map(|g| (g.pos, g.mass)).collect();
                let scale = self.scale * self.pscale as f64;
                let (midx, midy, pscale) = (self.midx, self.midy, self.pscale);
                let gt = &mut self.galaxies[i];
                for (j, s) in gt.stars.iter_mut().enumerate() {
                    let mut v = s.vel;
                    for (cpos, cmass) in &centres {
                        let d0 = cpos[0] - s.pos[0];
                        let d1 = cpos[1] - s.pos[1];
                        let d2 = cpos[2] - s.pos[2];
                        let dd = d0 * d0 + d1 * d1 + d2 * d2;
                        let f = if dd > EPSILON {
                            *cmass as f64 / (dd * dd.sqrt()) * DELTAT * DELTAT * QCONS
                        } else {
                            *cmass as f64 / (eps * eps.sqrt())
                        };
                        v[0] += d0 * f;
                        v[1] += d1 * f;
                        v[2] += d2 * f;
                    }
                    s.vel = v;
                    for (p, vv) in s.pos.iter_mut().zip(v.iter()) {
                        *p += *vv;
                    }

                    gt.newpoints[j] = XRectangle {
                        x: (((cox * s.pos[0]) - (six * s.pos[2])) * scale) as i32 + midx,
                        y: (((cor * s.pos[1]) - (sir * ((six * s.pos[0]) + (cox * s.pos[2]))))
                            * scale) as i32
                            + midy,
                        width: pscale,
                        height: pscale,
                    };
                }
            }

            // The galaxy centres attract one another, which is what draws the
            // tidal tails out.
            for k in i + 1..ngalaxies {
                let d0 = self.galaxies[k].pos[0] - self.galaxies[i].pos[0];
                let d1 = self.galaxies[k].pos[1] - self.galaxies[i].pos[1];
                let d2 = self.galaxies[k].pos[2] - self.galaxies[i].pos[2];
                let dd = d0 * d0 + d1 * d1 + d2 * d2;
                let f = if dd > EPSILON {
                    1.0 / (dd * dd.sqrt()) * DELTAT * QCONS
                } else {
                    1.0 / (EPSILON * SQRT_EPSILON) * DELTAT * QCONS
                };
                let (d0, d1, d2) = (d0 * f, d1 * f, d2 * f);
                let (mi_, mk) = (self.galaxies[i].mass as f64, self.galaxies[k].mass as f64);
                self.galaxies[i].vel[0] += d0 * mk;
                self.galaxies[i].vel[1] += d1 * mk;
                self.galaxies[i].vel[2] += d2 * mk;
                self.galaxies[k].vel[0] -= d0 * mi_;
                self.galaxies[k].vel[1] -= d1 * mi_;
                self.galaxies[k].vel[2] -= d2 * mi_;
            }

            for k in 0..3 {
                let v = self.galaxies[i].vel[k];
                self.galaxies[i].pos[k] += v * DELTAT;
            }

            // Erase the last frame's stars, then draw this one's.
            let black = self.mi.black;
            self.mi.gc.set_foreground(black);
            d.win()
                .fill_rectangles(&self.mi.gc, &self.galaxies[i].oldpoints);

            let step = self.mi.npixels() / COLORBASE;
            let c = self.mi.pixel((step * self.galaxies[i].galcol) as usize);
            self.mi.gc.set_foreground(c);
            d.win()
                .fill_rectangles(&self.mi.gc, &self.galaxies[i].newpoints);

            let g = &mut self.galaxies[i];
            std::mem::swap(&mut g.oldpoints, &mut g.newpoints);
        }

        self.step += 1;
        if self.step > self.f_hititerations * 4 {
            self.startover(d);
        }

        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.scale = (width + height) as f64 / 8.0 / self.pscale as f64;
        self.midx = width / 2;
        self.midy = height / 2;
        self.startover(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 20000",
    "*count: -5",
    "*cycles: 250",
    "*ncolors: 64",
    "*spin: True",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::spin("count", "Count", -20.0, 20.0, "-5"),
    Opt::slider("cycles", "Duration", 10.0, 1000.0, 10.0, 0, "250"),
    Opt::slider("ncolors", "Number of colors", 10.0, 255.0, 1.0, 0, "64"),
    Opt::boolean("spin", "Rotate viewpoint", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "galaxy",
    label: "Galaxy",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Uli Siegmund, Harald Backert, and Hubert Feyrer",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=xBprAm9w-Fo"),
        blurb: "Spinning galaxies collide.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
