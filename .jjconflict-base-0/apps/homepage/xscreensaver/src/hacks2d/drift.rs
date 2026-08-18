//! Port of `hacks/drift.c`.
//!
//! ```text
//! drift --- drifting recursive fractal cosmic flames
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
//! 01-Nov-2000: Allocation checks
//! 10-May-1997: Jamie Zawinski <jwz@jwz.org> compatible with xscreensaver
//! 01-Jan-1997: Moved new flame to drift.  Compile time options now run time.
//! 01-Jun-1995: Updated by Scott Draves.
//! 27-Jun-1991: vary number of functions used.
//! 24-Jun-1991: fixed portability problem with integer mod (%).
//! 06-Jun-1991: Written, received from Scott Draves <spot@cs.cmu.edu>
//! ```
//!
//! The ancestor of every flame fractal since: a handful of affine maps, each
//! followed by one of seven distortions (sinusoidal, complex, bent, swirl,
//! horseshoe, drape, or none at all), applied over and over to a wandering
//! point. The maps themselves drift a little every frame, so the figure
//! breathes rather than sitting still, and the colour of each point comes
//! from which maps it has recently been through.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XPoint};

/// Mono batch size, colour batch size.
const MAXBATCH1: usize = 200;
const MAXBATCH2: usize = 20;
/// Discard this many initial iterations.
const FUSE: i32 = 10;
const NMAJORVARS: i32 = 7;
const MAXLEV: usize = 10;

struct State {
    mi: ModeInfo,
    /// Shape of the current flame: a bunch of non-homogeneous transforms.
    nxforms: usize,
    f: [[[f64; MAXLEV]; 3]; 2],
    variation: [i32; 10],
    /// How each coefficient is drifting.
    df: [[[f64; MAXLEV]; 3]; 2],

    /// 0 is slow and single, 1 is fast and many.
    mode: i32,
    nfractals: i32,
    major_variation: i32,
    /// Points per fractal.
    fractal_len: i64,
    color: bool,
    /// More than one colour per fractal, computed by adding a dimension.
    rainbow: bool,

    width: i32,
    height: i32,

    /// Iterate this many before drawing anything.
    fuse: i32,
    total_points: i64,
    npoints: usize,
    pts: Vec<XPoint>,
    pixcol: Pixel,
    /// When drawing in colour there is a buffer per colour.
    ncpoints: Vec<usize>,
    cpts: Vec<XPoint>,

    x: f64,
    y: f64,
    c: f64,
    liss_time: i32,
    grow: bool,
    liss: bool,

    /// One random word yields two values.
    lasthalf: u16,
    /// And another is spent a few bits at a time.
    saved_random_bits: u32,
    nbits: i32,

    erase_countdown: i32,
}

impl State {
    fn halfrandom(&mut self, mv: i32) -> i32 {
        if mv <= 0 {
            return 0;
        }
        let r: u32 = if self.lasthalf != 0 {
            let r = self.lasthalf as u32;
            self.lasthalf = 0;
            r
        } else {
            let r = lrand();
            self.lasthalf = (r >> 16) as u16;
            r
        };
        (r % mv as u32) as i32
    }

    /// A few bits at a time out of one random word, which is where the
    /// choice of transform comes from.
    fn frandom(&mut self, n: usize) -> usize {
        if self.nbits < 3 {
            self.saved_random_bits = lrand();
            self.nbits = 31;
        }
        match n {
            2 => {
                let r = (self.saved_random_bits & 1) as usize;
                self.saved_random_bits >>= 1;
                self.nbits -= 1;
                r
            }
            3 => {
                let r = (self.saved_random_bits & 3) as usize;
                self.saved_random_bits >>= 2;
                self.nbits -= 2;
                if r == 3 { self.frandom(3) } else { r }
            }
            4 => {
                let r = (self.saved_random_bits & 3) as usize;
                self.saved_random_bits >>= 2;
                self.nbits -= 2;
                r
            }
            5 => {
                let r = (self.saved_random_bits & 7) as usize;
                self.saved_random_bits >>= 3;
                self.nbits -= 3;
                if r > 4 { self.frandom(5) } else { r }
            }
            _ => 0,
        }
    }

    fn distrib_a(&mut self) -> i64 {
        (self.halfrandom(7000) + 9000) as i64
    }

    fn distrib_b(&mut self) -> i64 {
        let a = self.frandom(3) as i64 + 1;
        let b = self.frandom(3) as i64 + 1;
        a * b * 120000
    }

    fn initmode(&mut self, d: &mut Dpy, mode: i32) {
        const VARIATION_LEN: i32 = 14;
        self.mode = mode;

        // 0, 0, 1, 1, 2, 2, 3, 4, 4, 5, 5, 6, 6, 6
        let v = self.halfrandom(VARIATION_LEN);
        self.major_variation = if (VARIATION_LEN >> 1..VARIATION_LEN - 1).contains(&v) {
            (v + 1) >> 1
        } else {
            v >> 1
        };

        if self.grow {
            self.rainbow = false;
            if mode != 0 {
                if !self.color || self.halfrandom(8) != 0 {
                    self.nfractals = self.halfrandom(30) + 5;
                    self.fractal_len = self.distrib_a();
                } else {
                    self.nfractals = self.halfrandom(5) + 5;
                    self.fractal_len = self.distrib_b();
                }
            } else {
                self.rainbow = self.color;
                self.nfractals = 1;
                self.fractal_len = self.distrib_b();
            }
        } else {
            self.nfractals = 1;
            self.rainbow = self.color;
            self.fractal_len = 2000000;
        }
        self.fractal_len = self.fractal_len * self.mi.count as i64 / 20;
        d.clear_window();
    }

    fn pick_df_coefs(&mut self) {
        for i in 0..self.nxforms {
            let mut r = 1e-6;
            for j in 0..2 {
                for k in 0..3 {
                    self.df[j][k][i] = self.halfrandom(1000) as f64 / 500.0 - 1.0;
                    r += self.df[j][k][i] * self.df[j][k][i];
                }
            }
            let r = (3 + self.halfrandom(5)) as f64 * 0.01 / r.sqrt();
            for j in 0..2 {
                for k in 0..3 {
                    self.df[j][k][i] *= r;
                }
            }
        }
    }

    fn initfractal(&mut self) {
        const XFORM_LEN: i32 = 9;
        self.fuse = FUSE;
        self.total_points = 0;

        let npix = self.mi.npixels().max(1) as usize;
        if self.ncpoints.len() < npix {
            self.ncpoints = vec![0; npix];
            self.cpts = vec![XPoint::default(); MAXBATCH2 * npix];
        }

        if self.rainbow {
            self.ncpoints.iter_mut().for_each(|n| *n = 0);
        } else {
            self.npoints = 0;
        }

        // 2, 2, 2, 3, 3, 3, 4, 4, 5
        let n = self.halfrandom(XFORM_LEN);
        self.nxforms = ((n >= XFORM_LEN - 1) as i32 + n / 3 + 2) as usize;

        self.c = 0.0;
        self.x = 0.0;
        self.y = 0.0;
        if self.liss && self.halfrandom(10) == 0 {
            self.liss_time = 0;
        }
        if !self.grow {
            self.pick_df_coefs();
        }
        for i in 0..self.nxforms {
            self.variation[i] = if self.major_variation == NMAJORVARS {
                self.halfrandom(NMAJORVARS)
            } else {
                self.major_variation
            };
            for j in 0..2 {
                for k in 0..3 {
                    self.f[j][k][i] = if self.liss {
                        (self.liss_time as f64 * self.df[j][k][i]).sin()
                    } else {
                        self.halfrandom(1000) as f64 / 500.0 - 1.0
                    };
                }
            }
        }
        self.pixcol = if self.color {
            let i = self.halfrandom(self.mi.npixels()) as usize;
            self.mi.pixel(i)
        } else {
            self.mi.white
        };
    }

    fn iter(&mut self) {
        let i = self.frandom(self.nxforms);
        let nc = if i != 0 {
            (self.c + 1.0) / 2.0
        } else {
            self.c / 2.0
        };

        let mut nx = self.f[0][0][i] * self.x + self.f[0][1][i] * self.y + self.f[0][2][i];
        let mut ny = self.f[1][0][i] * self.x + self.f[1][1][i] * self.y + self.f[1][2][i];

        match self.variation[i] {
            1 => {
                // Sinusoidal.
                nx = nx.sin();
                ny = ny.sin();
            }
            2 => {
                // Complex.
                let r2 = nx * nx + ny * ny + 1e-6;
                nx /= r2;
                ny /= r2;
            }
            3 => {
                // Bent.
                if nx < 0.0 {
                    nx *= 2.0;
                }
                if ny < 0.0 {
                    ny /= 2.0;
                }
            }
            4 => {
                // Swirl. Note that nx is computed from the new ny, not the
                // old one; that is upstream's ordering and part of the shape.
                let r = nx * nx + ny * ny;
                let c1 = r.sin();
                let c2 = r.cos();
                let t = nx;
                if !(-1e4..=1e4).contains(&nx) || !(-1e4..=1e4).contains(&ny) {
                    ny = 1e4;
                } else {
                    ny = c2 * t + c1 * ny;
                }
                nx = c1 * nx - c2 * ny;
            }
            5 => {
                // Horseshoe.
                let r = if nx == 0.0 && ny == 0.0 {
                    0.0
                } else {
                    nx.atan2(ny)
                };
                let c1 = r.sin();
                let c2 = r.cos();
                let t = nx;
                nx = c1 * nx - c2 * ny;
                ny = c2 * t + c1 * ny;
            }
            6 => {
                // Drape.
                let t = if nx == 0.0 && ny == 0.0 {
                    0.0
                } else {
                    nx.atan2(ny) / std::f64::consts::PI
                };
                if !(-1e4..=1e4).contains(&nx) || !(-1e4..=1e4).contains(&ny) {
                    ny = 1e4;
                } else {
                    ny = (nx * nx + ny * ny).sqrt() - 1.0;
                }
                nx = t;
            }
            _ => {}
        }

        // If it has run away, start again. No need to check ny; it will
        // propagate.
        if !(-1e4..=1e4).contains(&nx) || nx.is_nan() {
            nx = self.halfrandom(1000) as f64 / 500.0 - 1.0;
            ny = self.halfrandom(1000) as f64 / 500.0 - 1.0;
            self.fuse = FUSE;
        }
        self.x = nx;
        self.y = ny;
        self.c = nc;
    }

    fn draw_point(&mut self, d: &mut Dpy) {
        if self.fuse > 0 {
            self.fuse -= 1;
            return;
        }
        let (x, y) = (self.x, self.y);
        if !(x > -1.0 && x < 1.0 && y > -1.0 && y < 1.0) {
            return;
        }

        let fixed_x = ((self.width / 2) as f64 * (x + 1.0)) as i32;
        let fixed_y = ((self.height / 2) as f64 * (y + 1.0)) as i32;

        if !self.rainbow {
            self.pts[self.npoints] = XPoint {
                x: fixed_x,
                y: fixed_y,
            };
            self.npoints += 1;
            if self.npoints == MAXBATCH1 {
                self.mi.gc.set_foreground(self.pixcol);
                d.win().draw_points(&self.mi.gc, &self.pts[..self.npoints]);
                self.npoints = 0;
            }
        } else {
            let npix = self.mi.npixels().max(1) as usize;
            let c = ((self.c * npix as f64) as isize).clamp(0, npix as isize - 1) as usize;
            let n = self.ncpoints[c];
            self.cpts[c * MAXBATCH2 + n] = XPoint {
                x: fixed_x,
                y: fixed_y,
            };
            self.ncpoints[c] += 1;
            if self.ncpoints[c] == MAXBATCH2 {
                self.mi.gc.set_foreground(self.mi.pixel(c));
                let s = c * MAXBATCH2;
                d.win()
                    .draw_points(&self.mi.gc, &self.cpts[s..s + MAXBATCH2]);
                self.ncpoints[c] = 0;
            }
        }
    }

    fn draw_flush(&mut self, d: &mut Dpy) {
        if self.rainbow {
            let npix = self.mi.npixels().max(1) as usize;
            for i in 0..npix {
                if self.ncpoints[i] != 0 {
                    self.mi.gc.set_foreground(self.mi.pixel(i));
                    let s = i * MAXBATCH2;
                    let n = self.ncpoints[i];
                    d.win().draw_points(&self.mi.gc, &self.cpts[s..s + n]);
                    self.ncpoints[i] = 0;
                }
            }
        } else {
            if self.npoints != 0 {
                self.mi.gc.set_foreground(self.pixcol);
            }
            let n = self.npoints;
            d.win().draw_points(&self.mi.gc, &self.pts[..n]);
            self.npoints = 0;
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let (width, height) = (mi.width, mi.height);
    let color = mi.npixels() > 2;

    let mut st = State {
        mi,
        nxforms: 2,
        f: [[[0.0; MAXLEV]; 3]; 2],
        variation: [0; 10],
        df: [[[0.0; MAXLEV]; 3]; 2],
        mode: 1,
        nfractals: 1,
        major_variation: 0,
        fractal_len: 0,
        color,
        rainbow: false,
        width,
        height,
        fuse: FUSE,
        total_points: 0,
        npoints: 0,
        pts: vec![XPoint::default(); MAXBATCH1],
        pixcol: 0,
        ncpoints: Vec::new(),
        cpts: Vec::new(),
        x: 0.0,
        y: 0.0,
        c: 0.0,
        liss_time: 0,
        grow: false,
        liss: false,
        lasthalf: 0,
        saved_random_bits: 0,
        nbits: 0,
        erase_countdown: 0,
    };

    // Upstream runs fullrandom under xscreensaver, so the two knobs it
    // declares are always decided by the coin rather than by resources.
    if nrand(3) == 0 {
        st.grow = true;
    } else {
        st.grow = false;
        st.liss = lrand() & 1 == 1;
    }

    st.initmode(d, 1);
    st.initfractal();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.erase_countdown != 0 {
            self.erase_countdown -= 1;
            if self.erase_countdown == 0 {
                let m = self.frandom(2) as i32;
                self.initmode(d, m);
                self.initfractal();
            }
            return self.mi.delay;
        }

        let mut timer = 3000;
        while timer > 0 {
            self.iter();
            self.draw_point(d);
            self.total_points += 1;
            if self.total_points > self.fractal_len {
                self.draw_flush(d);
                self.nfractals -= 1;
                if self.nfractals == 0 {
                    self.erase_countdown = 4 * 1_000_000
                        / if self.mi.delay == 0 {
                            1
                        } else {
                            self.mi.delay as i32
                        };
                    return self.mi.delay;
                }
                self.initfractal();
            }
            timer -= 1;
        }

        if !self.grow {
            self.draw_flush(d);
            if self.liss {
                self.liss_time += 1;
            }
            for i in 0..self.nxforms {
                for j in 0..2 {
                    for k in 0..3 {
                        if self.liss {
                            self.f[j][k][i] = (self.liss_time as f64 * self.df[j][k][i]).sin();
                        } else {
                            self.f[j][k][i] += self.df[j][k][i];
                            let t = self.f[j][k][i];
                            if !(-1.0..=1.0).contains(&t) {
                                self.df[j][k][i] *= -1.0;
                            }
                        }
                    }
                }
            }
        }

        self.mi.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.width = width;
        self.height = height;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 10000",
    "*count: 30",
    "*ncolors: 200",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Duration", 1.0, 200.0, 1.0, 0, "30"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "drift",
    label: "Drift",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Scott Draves",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=cppZgCh6U7I"),
        blurb: "Drifting recursive fractal cosmic flames.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
