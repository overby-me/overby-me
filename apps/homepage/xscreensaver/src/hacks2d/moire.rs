//! Port of `hacks/moire.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1997, 1998, 2006 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Concept snarfed from Michael D. Bayne in
//! http://samskivert.com/internet/deep/1997/04/16/body.html
//! ```
//!
//! Colours each pixel by `(x² + y²) / factor` indexed into a ramp, from an
//! origin somewhere off screen. Concentric rings at two slightly different
//! scales interfere, and the moiré is the interference.
//!
//! Upstream paints through a shared-memory `XImage` a band at a time for
//! speed; here the window already *is* a pixel buffer, so the band structure
//! survives only because it is also what paces the animation.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{make_color_ramp, rgb_to_hsv};
use crate::runtime::{
    About, Dpy, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XColor, random, random_below,
};

/// Rows painted per `draw`. Upstream's `chunk_size`.
const CHUNK_SIZE: i32 = 20;

struct Moire {
    delay: i32,
    offset: i32,
    colors: Vec<XColor>,
    fg_pixel: Pixel,
    bg_pixel: Pixel,
    random_colors: bool,
    ncolors: usize,

    draw_y: i32,
    draw_xo: i32,
    draw_yo: i32,
    draw_factor: i32,
    width: i32,
    height: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut st = Moire {
        delay: d.res.int("delay"),
        offset: d.res.int("offset").max(2),
        colors: Vec::new(),
        fg_pixel: d.res.pixel("foreground"),
        bg_pixel: d.res.pixel("background"),
        random_colors: d.res.bool("random"),
        ncolors: d.res.int("ncolors").max(2) as usize,
        draw_y: 0,
        draw_xo: 0,
        draw_yo: 0,
        draw_factor: 1,
        width: d.width(),
        height: d.height(),
    };
    st.pick_colors(d);
    Box::new(st)
}

impl Moire {
    /// `moire_init_1`: a closed ramp between two colours, either the
    /// foreground and background resources or a fresh random pair.
    fn pick_colors(&mut self, d: &Dpy) {
        if d.mono_p {
            // Compensate for the lack of shading.
            self.offset *= 20;
            self.colors.clear();
            return;
        }

        let (fg, bg) = if self.random_colors {
            (
                XColor::from_rgb16(random() as u16, random() as u16, random() as u16),
                XColor::from_rgb16(random() as u16, random() as u16, random() as u16),
            )
        } else {
            (
                XColor::from_pixel(self.fg_pixel),
                XColor::from_pixel(self.bg_pixel),
            )
        };

        let (fgh, fgs, fgv) = rgb_to_hsv(fg.red, fg.green, fg.blue);
        let (bgh, bgs, bgv) = rgb_to_hsv(bg.red, bg.green, bg.blue);
        self.colors = make_color_ramp(fgh, fgs, fgv, bgh, bgs, bgv, self.ncolors, true);
    }
}

impl Screenhack for Moire {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.draw_y == 0 {
            self.pick_colors(d);
            self.width = d.width();
            self.height = d.height();
            self.draw_xo = random_below(self.width) - self.width / 2;
            self.draw_yo = random_below(self.height) - self.height / 2;
            self.draw_factor = random_below(self.offset) + 1;
        }

        let factor = self.draw_factor.max(1) as f64;
        for ii in 0..CHUNK_SIZE {
            let y = self.draw_y + ii;
            if y >= self.height {
                break;
            }
            let yy = (y + self.draw_yo) as f64;
            for x in 0..self.width {
                let xx = (x + self.draw_xo) as f64;
                let i = ((xx * xx) + (yy * yy)) / factor;
                let pixel = if self.colors.is_empty() {
                    if (i as i64) & 1 == 1 {
                        self.fg_pixel
                    } else {
                        self.bg_pixel
                    }
                } else {
                    self.colors[(i as i64).rem_euclid(self.colors.len() as i64) as usize].pixel
                };
                d.win().put_pixel(x, y, pixel);
            }
        }

        self.draw_y += CHUNK_SIZE;
        if self.draw_y >= self.height {
            self.draw_y = 0;
            // A finished picture lingers before the next one starts.
            return (self.delay.max(0) as u32).saturating_mul(1_000_000);
        }
        (self.delay.max(0) as u32).saturating_mul(10_000)
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.draw_y = 0;
    }
}

const DEFAULTS: &[&str] = &[
    ".background:		blue",
    ".foreground:		red",
    "*fpsSolid:		true",
    "*random:		true",
    "*delay:		5",
    "*ncolors:		64",
    "*offset:		50",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Duration", 1.0, 60.0, 1.0, 0, "5"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
    Opt::slider("offset", "Offset", 1.0, 200.0, 1.0, 0, "50"),
    Opt::boolean("random", "Random colors", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "moire",
    label: "Moiré",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski and Michael Bayne",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=S50zFVcUe4s"),
        blurb: "Interference patterns between two sets of concentric rings.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
