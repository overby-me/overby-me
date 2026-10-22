//! Port of `hacks/fadeplot.c`.
//!
//! ```text
//! Copyright (c) 1997 by Bas van Gaalen and Charles Vidal
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
//! A Lissajous-like figure sampled from a table of `sin(x)*|sin(x)|` rather
//! than a plain sine, which is what gives it the flattened lobes. Each frame
//! erases the previous points and plots a new set at a slightly advanced phase,
//! so the whole figure appears to sweep. The phase speeds and step sizes are
//! nudged on a timer, so it never quite settles.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XRectangle};

/// Upstream's `MINSTEPS`.
const MIN_STEPS: i32 = 1;

struct FadePlot {
    mi: ModeInfo,
    speed_x: i32,
    speed_y: i32,
    step_x: i32,
    step_y: i32,
    st_x: i32,
    st_y: i32,
    factor_x: i32,
    factor_y: i32,
    temps: i32,
    maxpts: usize,
    nbstep: i32,
    min: i32,
    pix: usize,
    angles: i32,
    /// `sin(t) * |sin(t)|`, scaled: the shape of the figure lives here.
    stab: Vec<i32>,
    pts: Vec<XRectangle>,
    scale: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // BRIGHT_COLORS and UNIFORM_COLORS are both defined above the
    // xlockmore.h include; uniform wins in xlockmore's switch.
    let mi = ModeInfo::new(d, ColorScheme::Uniform);
    let mut st = FadePlot {
        mi,
        speed_x: 8,
        speed_y: 10,
        step_x: 1,
        step_y: 1,
        st_x: 0,
        st_y: 0,
        factor_x: 1,
        factor_y: 1,
        temps: 0,
        maxpts: 1,
        nbstep: MIN_STEPS,
        min: 1,
        pix: 0,
        angles: 0,
        stab: Vec::new(),
        pts: Vec::new(),
        scale: 1,
    };
    st.restart(d);
    Box::new(st)
}

impl FadePlot {
    fn init_sintab(&mut self) {
        self.angles = nrand(950) + 250;
        self.stab = (0..self.angles)
            .map(|i| {
                let x = (std::f64::consts::TAU * i as f64 / self.angles as f64).sin();
                (x * x.abs() * self.min as f64) as i32 + self.min
            })
            .collect();
    }

    fn restart(&mut self, d: &mut Dpy) {
        self.mi.width = d.width();
        self.mi.height = d.height();
        self.min = (self.mi.width.min(self.mi.height) / 2).max(1);

        self.speed_x = 8;
        self.speed_y = 10;
        self.step_x = 1;
        self.step_y = 1;
        self.temps = 0;
        self.factor_x = (self.mi.width / (2 * self.min)).max(1);
        self.factor_y = (self.mi.height / (2 * self.min)).max(1);

        // Retina displays.
        self.scale = 1;
        if self.mi.width > 2560 || self.mi.height > 2560 {
            self.scale *= 3;
            self.step_x *= self.scale;
            self.step_y *= self.scale;
        }

        self.nbstep = self.mi.count;
        if self.nbstep < -MIN_STEPS {
            self.nbstep = nrand(-self.nbstep - MIN_STEPS + 1) + MIN_STEPS;
        } else if self.nbstep < MIN_STEPS {
            self.nbstep = MIN_STEPS;
        }

        self.maxpts = (self.mi.cycles / self.scale).max(1) as usize;
        self.pts = vec![XRectangle::default(); self.maxpts];

        if self.mi.npixels() > 2 {
            self.pix = nrand(self.mi.npixels()) as usize;
        }

        self.init_sintab();
        d.clear_window();
    }
}

impl Screenhack for FadePlot {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // Erase the previous frame's points.
        let black = self.mi.black;
        self.mi.gc.set_foreground(black);
        let old = self.pts.clone();
        d.win().fill_rectangles(&self.mi.gc, &old);

        if self.mi.npixels() > 2 {
            let c = self.mi.pixel(self.pix);
            self.mi.gc.set_foreground(c);
            self.pix += 1;
            if self.pix >= self.mi.npixels() as usize {
                self.pix = 0;
            }
        } else {
            let w = self.mi.white;
            self.mi.gc.set_foreground(w);
        }

        let per_step = self.maxpts / self.nbstep.max(1) as usize;
        let mut temp = 0;
        for j in 0..self.nbstep {
            for i in 0..per_step as i32 {
                if temp >= self.pts.len() {
                    break;
                }
                let sx = (self.st_x + self.speed_x * j + i * self.step_x).rem_euclid(self.angles)
                    as usize;
                let sy = (self.st_y + self.speed_y * j + i * self.step_y).rem_euclid(self.angles)
                    as usize;
                self.pts[temp] = XRectangle {
                    x: self.stab[sx] * self.factor_x + self.mi.width / 2 - self.min,
                    y: self.stab[sy] * self.factor_y + self.mi.height / 2 - self.min,
                    width: self.scale,
                    height: self.scale,
                };
                temp += 1;
            }
        }
        let drawn = self.pts[..temp].to_vec();
        d.win().fill_rectangles(&self.mi.gc, &drawn);

        self.st_x = (self.st_x + self.speed_x) % self.angles;
        self.st_y = (self.st_y + self.speed_y) % self.angles;
        self.temps += 1;

        // Every half turn, nudge the speeds and steps so it never settles.
        if self.temps % (self.angles / 2).max(1) == 0 {
            self.temps = self.temps % self.angles * 5;
            if self.temps % self.angles.max(1) == 0 {
                self.speed_y = (self.speed_y + 1) % 30 + 1;
            }
            if self.temps % (self.angles * 2).max(1) == 0 {
                self.speed_x %= 20;
            }
            if self.temps % (self.angles * 3).max(1) == 0 {
                self.step_y = (self.step_y + 1) % 2 + 1;
            }
            d.clear_window();
        }

        self.mi.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.min = (width.min(height) / 2).max(1);
        self.factor_x = (width / (2 * self.min)).max(1);
        self.factor_y = (height / (2 * self.min)).max(1);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 30000",
    "*count: 10",
    "*cycles: 1500",
    "*ncolors: 64",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("count", "Thickness", 0.0, 30.0, 1.0, 0, "10"),
    Opt::slider("cycles", "Cycles", 0.0, 10000.0, 100.0, 0, "1500"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "fadeplot",
    label: "Fade Plot",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Bas van Gaalen and Charles Vidal",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=Cev034v37JM"),
        blurb: "A sweeping plot of sine squared.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
