//! Port of `hacks/cwaves.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 2007 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! A sum of sine waves along the horizontal axis, with the total at each column
//! used to index a smooth colormap and painted as a full-height stripe. The
//! waves drift at different rates, so the bands slide over one another and the
//! interference is the whole effect.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, XEvent, frand,
    screenhack_event_helper,
};

/// `BELLRAND(n)`: three samples averaged, so the result clusters in the middle
/// rather than spreading evenly.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

struct Wave {
    scale: f64,
    offset: f64,
    delta: f64,
}

struct Cwaves {
    gc: Gc,
    delay: u32,
    /// Column width. Upstream calls the resource `waveScale` because Xft has
    /// already taken `scale`.
    scale: i32,
    ncolors: usize,
    colors: Vec<XColor>,
    waves: Vec<Wave>,
    width: i32,
    height: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let scale = d.res.int("waveScale").max(1);
    let ncolors = d.res.int("ncolors").max(4) as usize;
    let nwaves = d.res.int("nwaves").max(1) as usize;

    Box::new(Cwaves {
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        delay: d.res.int("delay").max(0) as u32,
        scale,
        ncolors,
        colors: make_smooth_colormap(ncolors),
        waves: (0..nwaves)
            .map(|_| Wave {
                scale: frand(0.03) + 0.005,
                offset: frand(std::f64::consts::PI),
                delta: (bellrand(2.0) - 1.0) / 15.0,
            })
            .collect(),
        width: d.width(),
        height: d.height(),
    })
}

impl Screenhack for Cwaves {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for w in self.waves.iter_mut() {
            w.offset += w.delta;
        }

        let nwaves = self.waves.len() as f64;
        let mut x = 0;
        while x < self.width {
            let mut v = 0.0;
            for w in &self.waves {
                v += ((x as f64 * w.scale) - w.offset).cos();
            }
            v /= nwaves;

            // The sum is in -1..1, so this lands in the map. Upstream aborts if
            // it does not; clamping is the same thing without the crash.
            let j = ((self.ncolors as f64 * (v / 2.0 + 0.5)) as usize).min(self.ncolors - 1);
            self.gc.set_foreground(self.colors[j].pixel);
            let (scale, height) = (self.scale, self.height);
            d.win().fill_rectangle(&self.gc, x, 0, scale, height);
            x += self.scale;
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.colors = make_smooth_colormap(self.ncolors);
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background:		   black",
    ".foreground:		   white",
    "*ncolors:		   600",
    "*nwaves:		   15",
    "*waveScale:		   2",
    "*debug:		   False",
    "*delay:		   20000",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("nwaves", "Complexity", 1.0, 100.0, 1.0, 0, "15"),
    Opt::slider("ncolors", "Color transitions", 2.0, 1000.0, 10.0, 0, "600"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cwaves",
    label: "C Waves",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2007",
        video: Some("https://www.youtube.com/watch?v=yOuJqiDUrpY"),
        blurb: "Interference bands from a sum of sine waves.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
