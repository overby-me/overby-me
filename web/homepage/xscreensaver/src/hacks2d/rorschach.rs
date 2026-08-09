//! Port of `hacks/rorschach.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! 19971004: Johannes Keukelaar <johannes@nada.kth.se>: Use helix screen
//!           eraser.
//! ```
//!
//! A random walk from the middle of the screen, mirrored about one or both
//! axes; the reflection is what makes it read as an inkblot. When the walk runs
//! out of iterations it lingers, then wipes the screen with one of the shared
//! erasers and starts again in a new colour.

use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, SaverDef, Screenhack, XColor, XEvent, XRectangle,
    color::hsv_to_rgb, random_below, screenhack_event_helper,
};

/// Points plotted per `draw` call. Upstream picks this so one call is a
/// sensible unit of work rather than the whole inkblot.
const ITER_CHUNK: i32 = 300;

struct Rorschach {
    draw_gc: Gc,
    default_fg_pixel: Pixel,
    iterations: i32,
    offset: i32,
    xsym: bool,
    ysym: bool,
    /// Seconds to linger once an inkblot is finished.
    sleep_time: i32,
    xlim: i32,
    ylim: i32,
    scale: i32,
    color: XColor,
    current_x: i32,
    current_y: i32,
    remaining_iterations: i32,
    eraser: Option<Eraser>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    // Retina displays: a one-pixel walk is invisible on a very large window.
    let scale = if d.width() > 2560 || d.height() > 2560 {
        3
    } else {
        1
    };
    Box::new(Rorschach {
        draw_gc: Gc::new(fg, d.res.pixel("background")),
        default_fg_pixel: fg,
        iterations: d.res.int("iterations").max(10),
        offset: {
            let o = d.res.int("offset");
            if o <= 0 { 3 } else { o }
        },
        xsym: d.res.bool("xsymmetry"),
        ysym: d.res.bool("ysymmetry"),
        sleep_time: d.res.int("delay"),
        xlim: d.width(),
        ylim: d.height(),
        scale,
        color: XColor::default(),
        current_x: 0,
        current_y: 0,
        remaining_iterations: -1,
        eraser: None,
    })
}

impl Rorschach {
    fn draw_start(&mut self, d: &mut Dpy) {
        self.xlim = d.width();
        self.ylim = d.height();

        if !d.mono_p {
            let (r, g, b) = hsv_to_rgb(random_below(360), 1.0, 1.0);
            self.color = XColor::from_rgb16(r, g, b);
            self.color.alloc();
            self.draw_gc.set_foreground(self.color.pixel);
        } else {
            self.draw_gc.set_foreground(self.default_fg_pixel);
        }

        self.current_x = self.xlim / 2;
        self.current_y = self.ylim / 2;
        self.remaining_iterations = self.iterations;
    }

    fn draw_step(&mut self, d: &mut Dpy) {
        let mut points: Vec<XRectangle> = Vec::with_capacity(4 * ITER_CHUNK as usize);
        let mut x = self.current_x;
        let mut y = self.current_y;

        let this_iterations = ITER_CHUNK.min(self.remaining_iterations);
        let s = self.scale;
        for _ in 0..this_iterations {
            x += random_below(1 + (self.offset << 1)) - self.offset;
            y += random_below(1 + (self.offset << 1)) - self.offset;
            points.push(XRectangle {
                x,
                y,
                width: s,
                height: s,
            });
            if self.xsym {
                points.push(XRectangle {
                    x: self.xlim - x,
                    y,
                    width: s,
                    height: s,
                });
            }
            if self.ysym {
                points.push(XRectangle {
                    x,
                    y: self.ylim - y,
                    width: s,
                    height: s,
                });
            }
            if self.xsym && self.ysym {
                points.push(XRectangle {
                    x: self.xlim - x,
                    y: self.ylim - y,
                    width: s,
                    height: s,
                });
            }
        }
        d.win().fill_rectangles(&self.draw_gc, &points);

        self.remaining_iterations -= this_iterations;
        if self.remaining_iterations < 0 {
            self.remaining_iterations = 0;
        }
        self.current_x = x;
        self.current_y = y;
    }
}

impl Screenhack for Rorschach {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut delay: u32 = 20000;

        if self.eraser.is_some() {
            self.eraser = erase_window(d, self.eraser.take());
            return delay;
        }

        if self.remaining_iterations > 0 {
            self.draw_step(d);
            if self.remaining_iterations == 0 {
                delay = (self.sleep_time.max(0) as u32).saturating_mul(1_000_000);
            }
        } else {
            if self.remaining_iterations == 0 {
                self.eraser = erase_window(d, None);
            }
            self.draw_start(d);
        }
        delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.xlim = width;
        self.ylim = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.remaining_iterations = 0;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background:	black",
    ".foreground:	white",
    "*fpsSolid:	true",
    "*xsymmetry:	true",
    "*ysymmetry:	false",
    "*iterations:	4000",
    "*offset:	7",
    "*delay:	5",
    "*eraseSeconds:	1",
];

const OPTS: &[Opt] = &[
    Opt::slider("iterations", "Iterations", 0.0, 10000.0, 100.0, 0, "4000"),
    Opt::slider("offset", "Offset", 0.0, 50.0, 1.0, 0, "7"),
    Opt::boolean("xsymmetry", "With X symmetry", "true"),
    Opt::boolean("ysymmetry", "With Y symmetry", "false"),
    Opt::slider("delay", "Linger", 1.0, 60.0, 1.0, 0, "5"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "rorschach",
    label: "Rorschach",
    new: init,
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=G1OLn4Mdk5Y"),
        blurb: "Inkblot patterns via a reflected random walk.",
    },
};
