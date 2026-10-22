//! Port of `hacks/flame.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1993-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! This file was ported from xlock for use in xscreensaver (and standalone)
//! by jwz on 18-Oct-93.  (And again, 11-May-97.)  Original copyright reads:
//!
//!   static char sccsid[] = "@(#)flame.c 1.4 91/09/27 XLOCK";
//!
//! flame.c - recursive fractal cosmic flames.
//!
//! Copyright (c) 1991 by Patrick J. Naughton.
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
//! 01-Jun-95: This should look more like the original with some updates by
//!            Scott Draves.
//! 27-Jun-91: vary number of functions used.
//! 24-Jun-91: fixed portability problem with integer mod (%).
//! 06-Jun-91: Written. (received from Scott Draves, spot@cs.cmu.edu).
//! ```
//!
//! Iterated function systems, the ancestor of every "fractal flame" image
//! since. Two to four affine maps are rolled at random, some of them followed
//! by one of ten nonlinear warps, and the tree of all their compositions is
//! walked depth first, plotting a dot wherever it lands inside the unit square.
//! Each frame is one such system; every so often the screen is cleared and the
//! warp changes.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, XEvent, XRectangle,
    random, screenhack_event_helper,
};

/// How many dots are batched before they are drawn.
const POINT_BUFFER_SIZE: usize = 10;
/// The most affine maps a system can have.
const MAXLEV: usize = 4;
/// How many nonlinear warps there are to choose from.
const MAXKINDS: u32 = 10;

struct Flame {
    /// Three non-homogeneous transforms, six coefficients each.
    f: [[[f64; MAXLEV]; 3]; 2],
    max_total: i32,
    max_levels: i32,
    cur_level: i32,
    variation: u32,
    snum: usize,
    anum: usize,
    num_points: usize,
    total_points: i32,
    pixcol: i32,
    ncolors: i32,
    colors: Vec<XColor>,
    points: [XRectangle; POINT_BUFFER_SIZE],
    gc: Gc,
    delay: u32,
    delay2: u32,
    width: i32,
    height: i32,
    /// Upstream's cached second half of a random number.
    lasthalf: u32,
    flame_alt: bool,
    do_reset: bool,
    scale: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut max_points = d.res.int("iterations");
    if max_points <= 0 {
        max_points = 100;
    }
    let mut max_total = d.res.int("points");
    if max_total <= 0 {
        max_total = 10000;
    }

    let mut scale = 1;
    if d.width() > 2560 || d.height() > 2560 {
        scale *= 2; // Retina displays
    }

    let mut ncolors = d.res.int("colors");
    if ncolors <= 0 {
        ncolors = 128;
    }
    let colors = make_smooth_colormap(ncolors as usize);
    let ncolors = colors.len() as i32;

    let mut st = Flame {
        f: [[[0.0; MAXLEV]; 3]; 2],
        max_total,
        max_levels: max_points,
        cur_level: 0,
        variation: random() % MAXKINDS,
        snum: 2,
        anum: 0,
        num_points: 0,
        total_points: 0,
        pixcol: 0,
        ncolors,
        colors,
        points: [XRectangle::default(); POINT_BUFFER_SIZE],
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        delay: d.res.int("delay").max(0) as u32,
        delay2: d.res.int("delay2").max(0) as u32,
        width: d.width(),
        height: d.height(),
        lasthalf: 0,
        flame_alt: false,
        do_reset: false,
        scale,
    };
    st.pixcol = st.halfrandom(st.ncolors.max(1));
    let c = st.colors[st.pixcol.clamp(0, st.ncolors - 1) as usize].pixel;
    st.gc.set_foreground(c);
    Box::new(st)
}

impl Flame {
    /// Upstream gets two values out of each random number, which halves the
    /// cost and is part of why the figures look the way they do.
    fn halfrandom(&mut self, mv: i32) -> i32 {
        let r = if self.lasthalf != 0 {
            let r = self.lasthalf;
            self.lasthalf = 0;
            r
        } else {
            let r = random() & 0x7fff_ffff;
            self.lasthalf = r >> 16;
            r
        };
        (r % mv.max(1) as u32) as i32
    }

    /// Apply this system's nonlinear warp to a point.
    fn warp(&self, nx: f64, ny: f64) -> (f64, f64) {
        match self.variation {
            0 => (nx.sin(), ny.sin()), // sinusoidal
            1 => {
                // complex
                let r2 = nx * nx + ny * ny + 1e-6;
                (nx / r2, ny / r2)
            }
            2 => {
                // bent
                let nx = if nx < 0.0 { nx * 2.0 } else { nx };
                let ny = if ny < 0.0 { ny / 2.0 } else { ny };
                (nx, ny)
            }
            3 => {
                // swirl
                let r = nx * nx + ny * ny;
                let c1 = r.sin();
                let c2 = r.cos();
                let t = nx;
                let ny = if !(-1e4..=1e4).contains(&nx) || !(-1e4..=1e4).contains(&ny) {
                    1e4
                } else {
                    c2 * t + c1 * ny
                };
                (c1 * nx - c2 * ny, ny)
            }
            4 => {
                // horseshoe
                let r = if nx == 0.0 && ny == 0.0 {
                    0.0
                } else {
                    nx.atan2(ny)
                };
                let c1 = r.sin();
                let c2 = r.cos();
                let t = nx;
                (c1 * nx - c2 * ny, c2 * t + c1 * ny)
            }
            5 => {
                // drape
                let t = if nx == 0.0 && ny == 0.0 {
                    0.0
                } else {
                    nx.atan2(ny) / std::f64::consts::PI
                };
                let ny = if !(-1e4..=1e4).contains(&nx) || !(-1e4..=1e4).contains(&ny) {
                    1e4
                } else {
                    (nx * nx + ny * ny).sqrt() - 1.0
                };
                (t, ny)
            }
            6 => {
                // broken
                let nx = if nx > 1.0 {
                    nx - 1.0
                } else if nx < -1.0 {
                    nx + 1.0
                } else {
                    nx
                };
                let ny = if ny > 1.0 {
                    ny - 1.0
                } else if ny < -1.0 {
                    ny + 1.0
                } else {
                    ny
                };
                (nx, ny)
            }
            7 => {
                // spherical
                let r = 0.5 + (nx * nx + ny * ny + 1e-6).sqrt();
                (nx / r, ny / r)
            }
            8 => (
                nx.atan() / std::f64::consts::FRAC_PI_2,
                ny.atan() / std::f64::consts::FRAC_PI_2,
            ),
            9 => {
                // complex sine
                let (u, v) = (nx, ny);
                let ev = v.exp();
                let emv = (-v).exp();
                ((ev + emv) * u.sin() / 2.0, (ev - emv) * u.cos() / 2.0)
            }
            _ => (nx.sin(), ny.sin()),
        }
    }

    /// Walk the tree of compositions depth first. Returns false once the dot
    /// budget for this system has run out.
    fn recurse(&mut self, d: &mut Dpy, mut x: f64, y: f64, l: i32) -> bool {
        if l == self.max_levels {
            self.total_points += 1;
            // How long each fractal runs.
            if self.total_points > self.max_total {
                return false;
            }

            if x > -1.0 && x < 1.0 && y > -1.0 && y < 1.0 {
                self.points[self.num_points] = XRectangle {
                    x: ((self.width / 2) as f64 * (x + 1.0)) as i32,
                    y: ((self.height / 2) as f64 * (y + 1.0)) as i32,
                    width: self.scale,
                    height: self.scale,
                };
                self.num_points += 1;
                if self.num_points >= POINT_BUFFER_SIZE {
                    d.win()
                        .fill_rectangles(&self.gc, &self.points[..self.num_points]);
                    self.num_points = 0;
                }
            }
            return true;
        }

        for i in 0..self.snum {
            // Scale back when values get very large. Spot sez: "I think this
            // happens on HPUX.  I think it's non-IEEE to generate an exception
            // instead of a silent NaN."
            // The change sticks for the rest of the loop, as it does
            // upstream.
            if x.abs() > 1.0e5 || y.abs() > 1.0e5 {
                x /= y;
            }

            let nx = self.f[0][0][i] * x + self.f[0][1][i] * y + self.f[0][2][i];
            let ny = self.f[1][0][i] * x + self.f[1][1][i] * y + self.f[1][2][i];
            let (nx, ny) = if i < self.anum {
                self.warp(nx, ny)
            } else {
                (nx, ny)
            };

            if !self.recurse(d, nx, ny, l + 1) {
                return false;
            }
        }
        true
    }
}

impl Screenhack for Flame {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut this_delay = self.delay;

        if self.do_reset {
            self.do_reset = false;
            d.clear_window();
        }

        let level = self.cur_level;
        self.cur_level += 1;
        if level % self.max_levels.max(1) == 0 {
            self.do_reset = true;
            this_delay = self.delay2;
            self.flame_alt = !self.flame_alt;
            self.variation = random() % MAXKINDS;
        } else if self.ncolors > 2 {
            let c = self.colors[self.pixcol.clamp(0, self.ncolors - 1) as usize].pixel;
            self.gc.set_foreground(c);
            self.pixcol -= 1;
            if self.pixcol < 0 {
                self.pixcol = self.ncolors - 1;
            }
        }

        // Number of functions, and how many of them are of alternate form.
        self.snum = 2 + (self.cur_level as usize % (MAXLEV - 1));
        self.anum = if self.flame_alt {
            0
        } else {
            self.halfrandom(self.snum as i32) as usize + 2
        };

        // Six coefficients per function.
        for k in 0..self.snum {
            for i in 0..2 {
                for j in 0..3 {
                    self.f[i][j][k] = (random() & 1023) as f64 / 512.0 - 1.0;
                }
            }
        }

        self.num_points = 0;
        self.total_points = 0;
        self.recurse(d, 0.0, 0.0, 0);
        d.win()
            .fill_rectangles(&self.gc, &self.points[..self.num_points]);

        this_delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.do_reset = true;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*colors: 64",
    "*iterations: 25",
    "*delay: 50000",
    "*delay2: 2000000",
    "*points: 10000",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "50000").inverted(),
    Opt::slider(
        "delay2",
        "Linger",
        1000.0,
        10_000_000.0,
        100_000.0,
        0,
        "2000000",
    ),
    Opt::slider("iterations", "Number of fractals", 1.0, 250.0, 1.0, 0, "25"),
    Opt::slider("points", "Complexity", 100.0, 80000.0, 100.0, 0, "10000"),
    Opt::slider("colors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "flame",
    label: "Flame",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Scott Draves",
        year: "1993",
        video: Some("https://www.youtube.com/watch?v=6Pu8JKNT_Jk"),
        blurb: "Iterative fractals.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
