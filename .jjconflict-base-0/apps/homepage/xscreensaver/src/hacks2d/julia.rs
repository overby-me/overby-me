//! Port of `hacks/julia.c`.
//!
//! ```text
//! Copyright (c) 1995 Sean McCullough <bankshot@mailhost.nmt.edu>.
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
//! 10-Jun-06: j.grahl@ucl.ac.uk: tweaked functions for parameter of Julia set
//! 28-May-97: jwz@jwz.org: added interactive frobbing with the mouse.
//! 10-May-97: jwz@jwz.org: turned into a standalone program.
//! 02-Dec-95: snagged boilerplate from hop.c
//!           used ifs {w0 = sqrt(x-c), w1 = -sqrt(x-c)} with random iteration
//!           to plot the julia set, and sinusoidially varied parameter for set
//!           and plotted parameter with a circle.
//! ```
//!
//! A Julia set drawn by inverse iteration: the two square roots of z minus c
//! are followed as a binary tree, and every node of that tree is a point of the
//! set. The parameter c wanders on a pair of incommensurate sinusoids, so the
//! set melts continuously from one shape to another, and a small circle marks
//! where c currently is. Dragging with the mouse takes c over instead.
//!
//! Upstream stipples the marker circle so it reads as a ring of dots; here it
//! is a plain outline, since the framebuffer has no stipple.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, XRectangle,
};

struct Julia {
    mi: ModeInfo,
    centerx: i32,
    centery: i32,
    /// The Julia parameter.
    cr: f64,
    ci: f64,
    depth: i32,
    inc: i32,
    circsize: i32,
    erase: bool,
    scale: i32,
    pix: usize,
    itree: usize,
    /// A ring of past frames, so the oldest can be erased exactly.
    buffer: usize,
    nbuffers: usize,
    point_buffer: Vec<Vec<XRectangle>>,
    marker: Gc,
    button_down: bool,
    mouse_x: i32,
    mouse_y: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // UNIFORM_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Uniform);
    let mut st = Julia {
        centerx: 0,
        centery: 0,
        cr: 0.0,
        ci: 0.0,
        depth: 1,
        inc: 0,
        circsize: 8,
        erase: false,
        scale: 1,
        pix: 0,
        itree: 0,
        buffer: 0,
        nbuffers: 2,
        point_buffer: Vec::new(),
        marker: Gc::new(mi.white, mi.black),
        mi,
        button_down: false,
        mouse_x: 0,
        mouse_y: 0,
    };
    st.restart(d);
    Box::new(st)
}

impl Julia {
    fn numpoints(&self) -> usize {
        ((2usize << self.depth) - 1).max(1)
    }

    fn restart(&mut self, d: &mut Dpy) {
        self.centerx = self.mi.width / 2;
        self.centery = self.mi.height / 2;

        self.depth = self.mi.count.clamp(1, 10);

        self.scale = 1;
        if self.mi.width > 2560 || self.mi.height > 2560 {
            self.scale *= 3; // Retina displays
        }

        self.circsize = 8.max((self.centerx.min(self.centery) / 96) * 2 + 1);

        if self.mi.npixels() > 2 {
            self.pix = nrand(self.mi.npixels()) as usize;
        }
        self.inc = ((lrand() & 1) as i32 * 2 - 1) * nrand(200);

        self.nbuffers = (self.mi.cycles + 1).max(1) as usize;
        let n = self.numpoints();
        self.point_buffer = (0..self.nbuffers)
            .map(|_| vec![XRectangle::default(); n])
            .collect();

        self.buffer = 0;
        self.erase = false;
        d.clear_window();
    }

    /// The parameter's path: two sinusoids per axis whose periods do not
    /// divide one another, so the shape never quite repeats.
    fn incr(&mut self) {
        if self.button_down {
            self.cr = ((self.mouse_x + 2 - self.centerx) as f64) * 2.0 / self.centerx.max(1) as f64;
            self.ci = ((self.mouse_y + 2 - self.centery) as f64) * 2.0 / self.centery.max(1) as f64;
        } else {
            use std::f64::consts::PI;
            let i = self.inc as f64;
            self.cr = 1.5 * ((PI * (i / 290.0)).sin() * (i * PI / 210.0).sin());
            self.ci = 1.5 * ((PI * (i / 310.0)).cos() * (i * PI / 190.0).cos());
            self.cr += 0.5 * (PI * i / 395.0).cos();
            self.ci += 0.5 * (PI * i / 410.0).sin();
        }
    }

    /// Walk both square-root branches as a binary tree, plotting every node.
    fn apply(&mut self, xr: f64, xi: f64, d: i32) {
        if self.itree < self.point_buffer[self.buffer].len() {
            let at = self.itree;
            self.point_buffer[self.buffer][at] = XRectangle {
                x: (0.5 * xr * self.centerx as f64) as i32 + self.centerx,
                y: (0.5 * xi * self.centery as f64) as i32 + self.centery,
                width: self.scale,
                height: self.scale,
            };
        }
        self.itree += 1;

        if d > 0 {
            let xi = xi - self.ci;
            let xr = xr - self.cr;

            // Avoid atan2's DOMAIN error message.
            let theta = if xi == 0.0 && xr == 0.0 {
                0.0
            } else {
                xi.atan2(xr) / 2.0
            };
            // Three times faster than a fourth-root by powf.
            let r = (xi * xi + xr * xr).sqrt().sqrt();

            let xr = r * theta.cos();
            let xi = r * theta.sin();

            self.apply(xr, xi, d - 1);
            self.apply(-xr, -xi, d - 1);
        }
    }
}

impl Screenhack for Julia {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let old_x = (self.centerx as f64 * self.cr / 2.0) as i32 + self.centerx - 2;
        let old_y = (self.centery as f64 * self.ci / 2.0) as i32 + self.centery - 2;
        self.incr();
        let new_x = (self.centerx as f64 * self.cr / 2.0) as i32 + self.centerx - 2;
        let new_y = (self.centery as f64 * self.ci / 2.0) as i32 + self.centery - 2;

        let black = self.mi.black;
        self.mi.gc.set_foreground(black);
        let cs = self.circsize;
        d.win().fill_arc(
            &self.mi.gc,
            old_x - cs / 2 - 2,
            old_y - cs / 2 - 2,
            cs + 4,
            cs + 4,
            0,
            FULL_CIRCLE,
        );

        // The marker showing where the parameter currently sits.
        let white = self.mi.white;
        self.marker.set_foreground(white);
        d.win().draw_arc(
            &self.marker,
            new_x - cs / 2,
            new_y - cs / 2,
            cs,
            cs,
            0,
            FULL_CIRCLE,
        );

        if self.erase {
            let buf = std::mem::take(&mut self.point_buffer[self.buffer]);
            d.win().fill_rectangles(&self.mi.gc, &buf);
            self.point_buffer[self.buffer] = buf;
        }

        self.inc += 1;
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

        // Sixty-four warm-up steps put the walk on the attractor before the
        // tree is built over it. Upstream reuses one random number for
        // thirty-two of them, a bit at a time.
        let (mut xr, mut xi) = (0.0f64, 0.0f64);
        let mut rnd = 0u32;
        let warm = 64.min(self.point_buffer[self.buffer].len());
        for slot in 0..64usize {
            let k = 63 - slot;
            if k % 32 == 0 {
                rnd = lrand();
            }

            xi -= self.ci;
            xr -= self.cr;

            let theta = if xi == 0.0 && xr == 0.0 {
                0.0
            } else {
                xi.atan2(xr) / 2.0
            };
            let r = (xi * xi + xr * xr).sqrt().sqrt();

            xr = r * theta.cos();
            xi = r * theta.sin();

            if (rnd >> (k % 32)) & 1 == 1 {
                xi = -xi;
                xr = -xr;
            }

            if slot < warm {
                let at = self.buffer;
                self.point_buffer[at][slot] = XRectangle {
                    x: self.centerx + ((self.centerx >> 1) as f64 * xr) as i32,
                    y: self.centery + ((self.centery >> 1) as f64 * xi) as i32,
                    width: self.scale,
                    height: self.scale,
                };
            }
        }

        self.itree = 0;
        let depth = self.depth;
        self.apply(xr, xi, depth);

        let buf = std::mem::take(&mut self.point_buffer[self.buffer]);
        d.win().fill_rectangles(&self.mi.gc, &buf);
        self.point_buffer[self.buffer] = buf;

        self.buffer += 1;
        if self.buffer > self.nbuffers - 1 {
            self.buffer -= self.nbuffers;
            self.erase = true;
        }

        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.restart(d);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        match event {
            XEvent::ButtonPress { x, y, button: 1 } => {
                self.button_down = true;
                self.mouse_x = *x;
                self.mouse_y = *y;
                true
            }
            XEvent::ButtonRelease { button: 1, .. } => {
                self.button_down = false;
                true
            }
            XEvent::MotionNotify { x, y } if self.button_down => {
                self.mouse_x = *x;
                self.mouse_y = *y;
                true
            }
            _ => false,
        }
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*count: 1000",
    "*cycles: 20",
    "*delay: 10000",
    "*ncolors: 200",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Count", 10.0, 20000.0, 10.0, 0, "1000"),
    Opt::slider("cycles", "Iterations", 1.0, 100.0, 1.0, 0, "20"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "julia",
    label: "Julia",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Sean McCullough",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=cA4Rgq-rmy8"),
        blurb: "A continuously varying Julia set.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
