//! Port of `hacks/xspirograph.c`.
//!
//! ```text
//! The Spiral Generator, Copyright (c) 2000
//! by Rohit Singh <rohit_singh@hotmail.com>
//!
//! Contains code from / To be used with:
//! xscreensaver, Copyright (c) 1992, 1995, 1996, 1997
//! Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notices appear in all copies and that both that
//! copyright notices and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Modified (Dec 2001) by Matthew Strait <straitm@mathcs.carleton.edu>
//! Added -subdelay and -alwaysfinish
//! Prevented redrawing over existing lines
//! ```
//!
//! The pen-in-nested-plastic-gears toy. A point a fixed distance from the
//! centre of a disc rolling inside a hollow rim traces the curve, and a
//! deliberate one-step error in the disc's angle is what turns a closed figure
//! into a drifting one. Layers alternate the sign of the inner radius, so each
//! pair of colours winds opposite ways, and the run ends when the pen returns
//! exactly to where it started.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::hsv_to_rgb;
use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XColor, frand, random,
};

/// How long a wipe is given per frame while one is running.
const ERASE_DELAY: u32 = 20000;

/// Line segments drawn per frame.
const STEPS_PER_FRAME: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CurState {
    NewLayer,
    Draw,
    Erase1,
    Erase2,
}

struct XSpirograph {
    gc: Gc,
    /// Seconds the finished figure lingers, and the wait after a wipe.
    long_delay: u32,
    sub_sleep_time: u32,
    num_layers: i32,
    default_fg: Pixel,
    always_finish: bool,
    mono: bool,
    width: i32,
    height: i32,
    theta: i32,
    /// The first point of this layer, kept at `float` precision because the
    /// run ends on an exact match against it.
    firstx: f32,
    firsty: f32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    counter: i32,
    distance: i32,
    radius1: i32,
    radius2: i32,
    drawstate: CurState,
    eraser: Option<Eraser>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    let mut gc = Gc::new(fg, d.res.pixel("background"));
    gc.set_line_width(if d.width() > 2560 || d.height() > 2560 {
        3 // Retina displays
    } else {
        1
    });

    Box::new(XSpirograph {
        gc,
        long_delay: d.res.int("delay").max(0) as u32,
        sub_sleep_time: d.res.int("subdelay").max(0) as u32,
        num_layers: d.res.int("layers"),
        default_fg: fg,
        always_finish: d.res.bool("alwaysfinish"),
        mono: d.mono_p,
        width: d.width(),
        height: d.height(),
        theta: 1,
        firstx: 0.0,
        firsty: 0.0,
        x1: 0,
        y1: 0,
        x2: 0,
        y2: 0,
        counter: 0,
        distance: 0,
        radius1: 0,
        radius2: 0,
        drawstate: CurState::NewLayer,
        eraser: None,
    })
}

impl XSpirograph {
    /// One step of the pen. Returns true when the figure has closed, or when
    /// it has run long enough and nobody asked for it to finish.
    fn go(&mut self, d: &mut Dpy, radius1: i32, radius2: i32, dist: i32) -> bool {
        // The disc's angle is an integer division, so it lags the true angle by
        // a fraction of a step: that error is the whole point.
        let delta = 1;
        let xmid = self.width / 2;
        let ymid = self.height / 2;
        // Upstream would divide by zero here when the random divisor lands
        // exactly on minus the offset.
        let r2 = if radius2 == 0 { 1 } else { radius2 };

        if self.theta == 1 {
            self.x1 = xmid + radius1 - radius2 + dist;
            self.y1 = ymid;
        }

        let turn = (self.theta as f64 * std::f64::consts::PI) / 180.0;
        let pen = (((radius1 * self.theta) - delta) / r2) as f64 * std::f64::consts::PI / 180.0;

        let tmpx = (xmid as f64
            + ((radius1 - radius2) as f64 * turn.cos())
            + (dist as f64 * pen.cos())) as f32;
        let tmpy = (ymid as f64
            + ((radius1 - radius2) as f64 * turn.sin())
            + (dist as f64 * pen.sin())) as f32;

        self.x2 = tmpx as i32;
        self.y2 = tmpy as i32;

        if self.theta == 1 {
            self.firstx = tmpx;
            self.firsty = tmpy;
        }

        if self.theta != 1 {
            let (x1, y1, x2, y2) = (self.x1, self.y1, self.x2, self.y2);
            d.win().draw_line(&self.gc, x1, y1, x2, y2);
        }

        self.x1 = self.x2;
        self.y1 = self.y2;

        // Back where it started: nothing new will be drawn from here.
        if tmpx == self.firstx && tmpy == self.firsty && self.theta != 1 {
            self.firstx = 0.0;
            self.firsty = 0.0;
            self.theta = 1;
            return true;
        }

        if !self.always_finish && self.theta > (360 * 100) {
            self.firstx = 0.0;
            self.firsty = 0.0;
            self.theta = 1;
            return true;
        }

        self.theta += 1;
        false
    }

    fn pick_new(&mut self) {
        let radius = self.width.min(self.height) / 2;
        let divisor = (frand(3.0) + 1.0) * (((random() & 1) as f64 * 2.0) - 1.0);
        self.radius1 = radius;
        self.radius2 = (radius as f64 / divisor) as i32 + 5;
        self.distance = 100 + (random() % 200) as i32;
        self.theta = 1;
    }

    fn new_colors(&mut self) {
        if self.mono {
            let fg = self.default_fg;
            self.gc.set_foreground(fg);
        } else {
            let (r, g, b) = hsv_to_rgb((random() % 360) as i32, frand(1.0), frand(0.5) + 0.5);
            self.gc.set_foreground(XColor::from_rgb16(r, g, b).pixel);
        }
    }
}

impl Screenhack for XSpirograph {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let flip_p = self.counter & 1 == 1;

        match self.drawstate {
            CurState::Erase1 => {
                // Pause before starting the wipe.
                self.drawstate = CurState::Erase2;
                if self.long_delay == 0 { 0 } else { 5_000_000 }
            }

            CurState::Erase2 => {
                self.eraser = erase_window(d, self.eraser.take());
                if self.eraser.is_some() {
                    return ERASE_DELAY;
                }
                self.drawstate = CurState::NewLayer;
                // Leave the screen black for a moment.
                if self.long_delay == 0 { 0 } else { 1_000_000 }
            }

            CurState::Draw => {
                let (r1, r2, dist) = (
                    self.radius1,
                    if flip_p { self.radius2 } else { -self.radius2 },
                    self.distance,
                );
                for _ in 0..STEPS_PER_FRAME {
                    if self.go(d, r1, r2, dist) {
                        self.drawstate = CurState::NewLayer;
                        break;
                    }
                }
                self.sub_sleep_time
            }

            CurState::NewLayer => {
                self.counter += 1;
                if self.counter > (2 * self.num_layers) {
                    self.counter = 0;
                    self.drawstate = CurState::Erase1;
                } else {
                    // Every other layer reuses the last one's gears with the
                    // inner radius flipped, so the pair winds both ways.
                    if !flip_p {
                        self.pick_new();
                    }
                    self.new_colors();
                    self.drawstate = CurState::Draw;
                }
                0
            }
        }
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*delay: 5",
    "*subdelay: 20000",
    "*layers: 2",
    "*alwaysfinish: false",
];

const OPTS: &[Opt] = &[
    Opt::slider("subdelay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("delay", "Linger", 1.0, 60.0, 1.0, 0, "5"),
    Opt::spin("layers", "Layers", 1.0, 10.0, "2"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "xspirograph",
    label: "XSpirograph",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Rohit Singh",
        year: "2000",
        video: Some("https://www.youtube.com/watch?v=XWCeQqzNavY"),
        blurb: "That pen-in-nested-plastic-gears toy from your childhood.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
