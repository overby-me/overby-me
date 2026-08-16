//! Port of `hacks/grav.c`.
//!
//! ```text
//! Copyright (c) 1997 by Greg Bowering <greg@cs.adelaide.edu.au>
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
//! Planets orbiting a star, in three dimensions projected onto the screen: a
//! planet's radius comes from its Z, so it swells as it swings toward you. With
//! decay on, the accelerations are clamped and the velocities damped, so the
//! orbits spiral inward instead of flinging everything away.
//!
//! Each planet is erased by redrawing it in black before it moves, which is why
//! the trail dots have to be laid down in between.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{
    About, Dpy, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, random_below,
};

/// Gravitational constant, and the rest of upstream's tuning.
const GRAV: f64 = -0.02;
const DIST: f64 = 16.0;
const COLLIDE: f64 = 0.0001;
const ALMOST: f64 = 15.99;
const HALF: f64 = 0.5;
const VR: f64 = 0.04;
const DAMP: f64 = 0.999_999;
/// Maximum acceleration, with damping.
const MAX_A: f64 = 0.1;

const X: usize = 0;
const Y: usize = 1;
const Z: usize = 2;

struct Planet {
    p: [f64; 3],
    v: [f64; 3],
    a: [f64; 3],
    xi: i32,
    yi: i32,
    ri: i32,
    color: Pixel,
}

struct Grav {
    mi: ModeInfo,
    /// Star radius, which wanders up and down as it goes.
    sr: i32,
    nplanets: usize,
    starcolor: Pixel,
    planets: Vec<Planet>,
    decay: bool,
    trail: bool,
    drawn: bool,
}

/// `FLOATRAND(min, max)`.
fn floatrand(min: f64, max: f64) -> f64 {
    min + (crate::runtime::random() as f64 / u32::MAX as f64) * (max - min)
}

impl Grav {
    fn intrinsic_radius(&self) -> f64 {
        self.mi.height as f64 / 5.0
    }

    fn star_radius(&self) -> i32 {
        (self.mi.height as f64 / (2.0 * DIST)) as i32
    }

    fn radius_of(&self, z: f64) -> i32 {
        (self.intrinsic_radius() / (z + DIST)) as i32
    }

    fn init_planet(&self, planet: &mut Planet) {
        planet.color = if self.mi.npixels() > 2 {
            self.mi.pixel(nrand(self.mi.npixels()) as usize)
        } else {
            self.mi.white
        };

        let r = HALF * ALMOST;
        planet.p = [floatrand(-r, r), floatrand(-r, r), floatrand(-r, r)];

        if planet.p[Z] > -ALMOST {
            planet.xi = (self.mi.width as f64 * (HALF + planet.p[X] / (planet.p[Z] + DIST))) as i32;
            planet.yi =
                (self.mi.height as f64 * (HALF + planet.p[Y] / (planet.p[Z] + DIST))) as i32;
        } else {
            planet.xi = -1;
            planet.yi = -1;
        }
        planet.ri = self.radius_of(planet.p[Z]);

        planet.v = [floatrand(-VR, VR), floatrand(-VR, VR), floatrand(-VR, VR)];
        planet.a = [0.0; 3];
    }

    /// `Planet(x, y)`: a filled disc, clipped to the window as upstream does
    /// before drawing rather than after.
    fn draw_disc(&self, d: &mut Dpy, x: i32, y: i32, ri: i32, color: Pixel) {
        if x < 0 || y < 0 || x > self.mi.width || y > self.mi.height {
            return;
        }
        let mut gc = self.mi.gc.clone();
        gc.set_foreground(color);
        d.win()
            .fill_arc(&gc, x - ri / 2, y - ri / 2, ri, ri, 0, FULL_CIRCLE);
    }

    fn draw_star(&self, d: &mut Dpy, color: Pixel) {
        let mut gc = self.mi.gc.clone();
        gc.set_foreground(color);
        d.win().draw_arc(
            &gc,
            self.mi.width / 2 - self.sr / 2,
            self.mi.height / 2 - self.sr / 2,
            self.sr,
            self.sr,
            0,
            FULL_CIRCLE,
        );
    }

    fn restart(&mut self, d: &mut Dpy) {
        self.mi.width = d.width();
        self.mi.height = d.height();
        self.sr = self.star_radius();

        let mut nplanets = self.mi.count;
        if nplanets < 0 {
            // Add one so it is not too boring.
            nplanets = nrand(-nplanets) + 1;
        }
        self.nplanets = nplanets.max(1) as usize;

        d.clear_window();

        self.starcolor = if self.mi.npixels() > 2 {
            self.mi.pixel(nrand(self.mi.npixels()) as usize)
        } else {
            self.mi.white
        };

        self.planets = (0..self.nplanets)
            .map(|_| {
                let mut p = Planet {
                    p: [0.0; 3],
                    v: [0.0; 3],
                    a: [0.0; 3],
                    xi: -1,
                    yi: -1,
                    ri: 0,
                    color: self.mi.white,
                };
                self.init_planet(&mut p);
                p
            })
            .collect();
        self.drawn = false;
    }

    fn draw_planet(&mut self, d: &mut Dpy, i: usize) {
        let (black, width, height) = (self.mi.black, self.mi.width, self.mi.height);
        let decay = self.decay;

        let mut dist = {
            let p = &self.planets[i].p;
            p[X] * p[X] + p[Y] * p[Y] + p[Z] * p[Z]
        };
        if dist < COLLIDE {
            dist = COLLIDE;
        }
        dist = dist.sqrt();
        dist = dist * dist * dist;

        {
            let planet = &mut self.planets[i];
            for c in X..=Z {
                planet.a[c] = planet.p[c] * GRAV / dist;
                if decay {
                    planet.a[c] = planet.a[c].clamp(-MAX_A, MAX_A);
                    planet.v[c] += planet.a[c];
                    planet.v[c] *= DAMP;
                } else {
                    planet.v[c] += planet.a[c];
                }
                planet.p[c] += planet.v[c];
            }
        }

        // Where it was, before the projection is recomputed.
        let (old_x, old_y, old_ri) = {
            let p = &self.planets[i];
            (p.xi, p.yi, p.ri)
        };

        {
            let planet = &mut self.planets[i];
            if planet.p[Z] > -ALMOST {
                planet.xi = (width as f64 * (HALF + planet.p[X] / (planet.p[Z] + DIST))) as i32;
                planet.yi = (height as f64 * (HALF + planet.p[Y] / (planet.p[Z] + DIST))) as i32;
            } else {
                planet.xi = -1;
                planet.yi = -1;
            }
        }

        // Mask out where it was.
        self.draw_disc(d, old_x, old_y, old_ri, black);

        if self.trail {
            let mut r = 1;
            if width > 2560 || height > 2560 {
                r *= 3; // Retina displays
            }
            let color = self.planets[i].color;
            let mut gc = self.mi.gc.clone();
            gc.set_foreground(color);
            d.win()
                .fill_arc(&gc, old_x - r / 2, old_y - r / 2, r, r, 0, FULL_CIRCLE);
        }

        let z = self.planets[i].p[Z];
        let ri = self.radius_of(z);
        self.planets[i].ri = ri;
        let (x, y, color) = {
            let p = &self.planets[i];
            (p.xi, p.yi, p.color)
        };
        self.draw_disc(d, x, y, ri, color);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // BRIGHT_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Bright);
    let mut st = Grav {
        starcolor: mi.white,
        mi,
        sr: 0,
        nplanets: 0,
        planets: Vec::new(),
        decay: d.res.bool("decay"),
        trail: d.res.bool("trail"),
        drawn: false,
    };
    st.restart(d);
    Box::new(st)
}

impl Screenhack for Grav {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if !self.drawn {
            for i in 0..self.planets.len() {
                let (x, y, ri, color) = {
                    let p = &self.planets[i];
                    (p.xi, p.yi, p.ri, p.color)
                };
                self.draw_disc(d, x, y, ri, color);
            }
            self.draw_star(d, self.starcolor);
        }
        self.drawn = true;

        // Mask the centrepoint, resize it, redraw it.
        let black = self.mi.black;
        self.draw_star(d, black);
        match random_below(4) {
            0 if self.sr < self.star_radius() => self.sr += 1,
            1 if self.sr > 2 => self.sr -= 1,
            _ => {}
        }
        let starcolor = self.starcolor;
        self.draw_star(d, starcolor);

        for i in 0..self.planets.len() {
            self.draw_planet(d, i);
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        d.clear_window();
        self.drawn = false;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 10000",
    "*count: 12",
    "*ncolors: 64",
    "*fpsSolid: true",
    "*decay: True",
    "*trail: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Number of objects", 1.0, 40.0, 1.0, 0, "12"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
    Opt::boolean("decay", "Orbital decay", "true"),
    Opt::boolean("trail", "Object trails", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "grav",
    label: "Grav",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Greg Bowering",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=spQRFDmDMeg"),
        blurb: "Planets orbiting a star, in three dimensions.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
