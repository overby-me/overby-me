//! Port of `hacks/zoom.c`.
//!
//! ```text
//!  Copyright (C) 2000 James Macnicol
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
//! Fatbits. The screen is a grid of tiles, and each tile shows a small square
//! cut from a picture, at a position that steps a little further along for
//! every tile. Because the step is smaller than the tile, the picture is
//! magnified and each tile behaves like its own lens, so the whole grid slides
//! and distorts as the sample point wanders on a pair of very slow sines.
//!
//! With lenses turned off each tile is a single pixel of the picture blown up
//! to tile size, which is the plainer magnifying-glass version of the same
//! idea.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Gc, ImageLoad, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent,
    random, screenhack_event_helper,
};

const MINX: f64 = 0.0;
const MINY: f64 = 0.0;
/// This should be way slower than the spotlight hack was.
const X_PERIOD: f64 = 45000.0 * 3.0;
const Y_PERIOD: f64 = 36000.0 * 3.0;

struct State {
    sizex: i32,
    sizey: i32,
    delay: u32,
    duration: f64,
    pixwidth: i32,
    pixheight: i32,
    pixspacex: i32,
    pixspacey: i32,
    lensoffsetx: i32,
    lensoffsety: i32,
    lenses: bool,

    window_gc: Gc,
    /// The picture, kept off screen.
    pm: Pixmap,

    tlx: i32,
    tly: i32,
    s: i32,

    /// Do not run multiple screens in lock-step.
    sinusoid_offset: f64,

    start_time: f64,
    img_loader: Option<ImageLoad>,
    loading: bool,
}

impl State {
    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    /// The channel draws into the window, so lift the picture off it and clear
    /// the window: upstream loads straight into an off-screen pixmap.
    fn image_arrived(&mut self, d: &mut Dpy) {
        self.pm = d.win_ref().sub_image(0, 0, self.sizex, self.sizey);
        self.loading = false;
        self.start_time = d.time;
        d.clear_window();
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let bg = d.res.pixel("background");
    let (sizex, sizey) = (d.width(), d.height());

    let mut pixwidth = d.res.int("pixwidth").max(1);
    let mut pixheight = d.res.int("pixheight").max(1);
    let mut pixspacex = d.res.int("pixspacex").max(0);
    let mut pixspacey = d.res.int("pixspacey").max(0);

    if sizex < 50 || sizey < 50 {
        // Tiny window.
        pixwidth = 10;
        pixheight = 10;
    }

    let lenses = d.res.bool("lenses");
    // Upstream clamps both offsets against pixwidth, including the vertical
    // one, so a tall thin tile cannot step further down than it is wide.
    let lensoffsetx = d.res.int("lensoffsetx").clamp(0, pixwidth);
    let lensoffsety = d.res.int("lensoffsety").clamp(0, pixwidth);

    if sizex > 2560 || sizey > 2560 {
        // Retina displays.
        pixwidth *= 3;
        pixheight *= 3;
        pixspacex *= 3;
        pixspacey *= 3;
    }

    let nblocksx = (sizex as f64 / (pixwidth + pixspacex) as f64).ceil() as i32;
    let nblocksy = (sizey as f64 / (pixheight + pixspacey) as f64).ceil() as i32;
    let s = if lenses {
        ((nblocksx - 1) * lensoffsetx + pixwidth).max((nblocksy - 1) * lensoffsety + pixheight) * 2
    } else {
        nblocksx.max(nblocksy) * 2
    };

    let mut st = State {
        sizex,
        sizey,
        delay: d.res.int("delay").max(1) as u32,
        duration: d.res.int("duration").max(1) as f64,
        pixwidth,
        pixheight,
        pixspacex,
        pixspacey,
        lensoffsetx,
        lensoffsety,
        lenses,
        window_gc: Gc::new(bg, bg),
        pm: Pixmap::new(sizex, sizey),
        tlx: 0,
        tly: 0,
        s,
        sinusoid_offset: (random() % 10_000_000) as f64,
        start_time: 0.0,
        img_loader: None,
        loading: false,
    };
    let (w, h) = (sizex, sizey);
    d.win().fill_rectangle(&st.window_gc, 0, 0, w, h);
    st.start_load(d);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.loading {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return self.delay;
        }

        if self.start_time + self.duration < d.time {
            self.start_load(d);
            return self.delay;
        }

        let now = d.time * 1000.0 + self.sinusoid_offset;

        // Find new x, y.
        self.tlx = (((1.0 + (now / X_PERIOD * 2.0 * std::f64::consts::PI).sin()) / 2.0)
            * (self.sizex - self.s / 2) as f64
            + MINX) as i32;
        self.tly = (((1.0 + (now / Y_PERIOD * 2.0 * std::f64::consts::PI).sin()) / 2.0)
            * (self.sizey - self.s / 2) as f64
            + MINY) as i32;

        let (stepx, stepy) = (
            self.pixwidth + self.pixspacex,
            self.pixheight + self.pixspacey,
        );
        if self.lenses {
            let mut i = 0;
            let mut x = 0;
            while x < self.sizex {
                let mut j = 0;
                let mut y = 0;
                while y < self.sizey {
                    d.win().copy_area(
                        &self.window_gc,
                        &self.pm,
                        self.tlx + i * self.lensoffsetx,
                        self.tly + j * self.lensoffsety,
                        self.pixwidth,
                        self.pixheight,
                        x,
                        y,
                    );
                    y += stepy;
                    j += 1;
                }
                x += stepx;
                i += 1;
            }
        } else {
            let mut i = 0;
            let mut x = 0;
            while x < self.sizex {
                let mut j = 0;
                let mut y = 0;
                while y < self.sizey {
                    let p = self.pm.get_pixel(self.tlx + i, self.tly + j);
                    self.window_gc.set_foreground(p);
                    d.win().fill_rectangle(
                        &self.window_gc,
                        i * stepx,
                        j * stepy,
                        self.pixwidth,
                        self.pixheight,
                    );
                    y += stepy;
                    j += 1;
                }
                x += stepx;
                i += 1;
            }
        }

        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape handler, so the picture stays at the old
        // size. A canvas is resized far more often than an X window was, so
        // fetch a fresh one at the new size.
        self.sizex = width;
        self.sizey = height;
        self.pm = Pixmap::new(width, height);
        d.clear_window();
        self.start_load(d);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.start_time = f64::NEG_INFINITY;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    "*dontClearRoot: True",
    ".foreground: white",
    ".background: #111111",
    "*fpsSolid: true",
    "*lenses: true",
    "*delay: 10000",
    "*duration: 120",
    "*pixwidth: 40",
    "*pixheight: 40",
    "*pixspacex: 2",
    "*pixspacey: 2",
    "*lensoffsetx: 5",
    "*lensoffsety: 5",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::spin("pixwidth", "X mag", 2.0, 100.0, "40"),
    Opt::spin("pixheight", "Y mag", 2.0, 100.0, "40"),
    Opt::spin("pixspacex", "X border", 0.0, 10.0, "2"),
    Opt::spin("pixspacey", "Y border", 0.0, 10.0, "2"),
    Opt::spin("lensoffsetx", "X lens", 1.0, 100.0, "5"),
    Opt::spin("lensoffsety", "Y lens", 1.0, 100.0, "5"),
    Opt::boolean("lenses", "Lenses", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "zoom",
    label: "Zoom",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "James Macnicol",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=LeQa9inGEKc"),
        blurb: "Fatbits! Zooms in on a part of an image and scrolls, distorting each pixel with its own lens.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
