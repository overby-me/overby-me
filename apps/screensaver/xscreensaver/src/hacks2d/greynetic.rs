//! Port of `hacks/greynetic.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2008 Jamie Zawinski <jwz@jwz.org>
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
//! Random rectangles in random colours, forever. Upstream has two variants
//! behind `#ifdef DO_STIPPLE`, which is off by default, and a jwxyz-only branch
//! that gives each rectangle a random alpha; this follows the plain X11 build,
//! so the rectangles are opaque and unstippled.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XColor, random,
    random_below,
};

/// How many distinct colours to allocate before starting to reuse them.
/// Upstream's array is 512 entries; on a TrueColor visual the limit no longer
/// does anything useful, but it is what gives the palette its slow drift.
const MAX_PIXELS: usize = 512;

struct Greynetic {
    gc: Gc,
    delay: u32,
    fg: Pixel,
    bg: Pixel,
    pixels: Vec<Pixel>,
    xlim: i32,
    ylim: i32,
    grey_p: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");
    Box::new(Greynetic {
        gc: Gc::new(fg, bg),
        delay: d.res.int("delay").max(0) as u32,
        fg,
        bg,
        pixels: Vec::new(),
        xlim: d.width(),
        ylim: d.height(),
        grey_p: d.res.bool("grey"),
    })
}

impl Screenhack for Greynetic {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (mut w, mut h) = (0, 0);
        // Minimize area, but don't try too hard.
        for _ in 0..10 {
            w = 50 + random_below(self.xlim - 50);
            h = 50 + random_below(self.ylim - 50);
            if w + h < self.xlim && w + h < self.ylim {
                break;
            }
        }
        let x = random_below(self.xlim - w);
        let y = random_below(self.ylim - h);

        let foreground = if d.mono_p {
            if random() & 1 == 1 { self.fg } else { self.bg }
        } else if self.pixels.len() >= MAX_PIXELS {
            self.pixels[random_below(self.pixels.len() as i32) as usize]
        } else {
            let mut c = XColor::from_rgb16(random() as u16, random() as u16, random() as u16);
            if self.grey_p {
                c.green = c.red;
                c.blue = c.red;
            }
            c.alloc();
            self.pixels.push(c.pixel);
            c.pixel
        };

        self.gc.set_foreground(foreground);
        d.win().fill_rectangle(&self.gc, x, y, w, h);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.xlim = width;
        self.ylim = height;
    }
}

const DEFAULTS: &[&str] = &[
    ".background:	black",
    ".foreground:	white",
    "*fpsSolid:	true",
    "*delay:	10000",
    "*grey:	false",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 250_000.0, 1000.0, 0, "10000").inverted(),
    Opt::boolean("grey", "Grey", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "greynetic",
    label: "Greynetic",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=lVEi089s1_c"),
        blurb: "Colored, stippled and transparent rectangles.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
///
/// Naming `init` here rather than storing it in [`DEF`] is what lets the
/// splitter see this hack's code as reachable from this chunk and nowhere else.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
