//! Port of `hacks/deco.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1997, 1998, 2002 Jamie Zawinski <jwz@jwz.org>
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
//! Splits the screen in half, then splits the halves, and so on until the
//! pieces are too small or the recursion has gone deep enough; each leaf gets
//! a colour and an outline. Golden-ratio mode always divides the longer side
//! at phi rather than in half, and Mondrian mode restricts the palette to
//! white, red, blue and yellow with heavy black rules.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{make_random_colormap, make_smooth_colormap};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, random, random_below,
};

/// The golden ratio: dividing A+B so that A:B equals (A+B):A is supposed to be
/// visually pleasing.
const PHI: f64 = 1.618_03;
const PHI1: f64 = 1.0 / PHI;
const PHI2: f64 = 1.0 - PHI1;

struct Deco {
    colors: Vec<XColor>,
    max_depth: i32,
    min_height: i32,
    min_width: i32,
    line_width: i32,
    golden_ratio: bool,
    mondrian: bool,

    delay: i32,
    width: i32,
    height: i32,
    fgc: Gc,
    bgc: Gc,
    current_color: usize,
    mono: bool,
}

/// The Mondrian palette: mostly white, with one each of red, blue and yellow.
/// Copied from upstream's `make_mondrian_colormap`.
fn mondrian_colormap() -> Vec<XColor> {
    let white = XColor::from_rgb16(0xE800, 0xE800, 0xE800);
    vec![
        white,
        white,
        white,
        white,
        XColor::from_rgb16(0xCFFF, 0, 0),      // red
        XColor::from_rgb16(0x2000, 0, 0xCFFF), // blue
        XColor::from_rgb16(0xDFFF, 0xCFFF, 0), // yellow
        white,
    ]
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut line_width = d.res.int("lineWidth");
    if d.width() > 2560 || d.height() > 2560 {
        line_width *= 3; // Retina displays
    }

    let ncolors = d.res.int("ncolors");
    let mono = d.mono_p || ncolors <= 2;
    let mondrian = d.res.bool("mondrian");
    let smooth_colors = d.res.bool("smoothColors");

    // Not mono: the fill and outline GCs swap, so the outline is drawn in the
    // background colour and the fill walks the colormap.
    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");
    let (fgc, bgc) = if mono {
        (Gc::new(fg, bg), Gc::new(bg, fg))
    } else {
        (Gc::new(bg, fg), Gc::new(fg, bg))
    };

    let colors = if mondrian {
        mondrian_colormap()
    } else if smooth_colors {
        make_smooth_colormap(ncolors.max(1) as usize)
    } else {
        make_random_colormap(ncolors.max(1) as usize, false)
    };

    let mut st = Deco {
        colors,
        max_depth: d.res.int("maxDepth").clamp(1, 1000),
        min_width: d.res.int("minWidth").max(2),
        min_height: d.res.int("minHeight").max(2),
        line_width,
        golden_ratio: d.res.bool("goldenRatio"),
        mondrian,
        delay: d.res.int("delay"),
        width: d.width(),
        height: d.height(),
        fgc,
        bgc,
        current_color: 0,
        mono,
    };
    st.fgc.set_line_width(st.line_width);
    if st.mondrian {
        st.mondrian_set_sizes();
    }
    Box::new(st)
}

impl Deco {
    /// Mondrian overrides the size and rule-width settings, scaled to the
    /// window's shorter dimension.
    fn mondrian_set_sizes(&mut self) {
        let n = if self.width > self.height {
            self.width
        } else {
            self.height
        };
        self.line_width = n / 50;
        self.min_height = n / 8;
        self.min_width = n / 8;
    }

    fn deco(&mut self, d: &mut Dpy, x: i32, y: i32, w: i32, h: i32, depth: i32) {
        if random_below(self.max_depth) < depth || w < self.min_width || h < self.min_height {
            if !self.mono && !self.colors.is_empty() {
                self.current_color += 1;
                if self.current_color >= self.colors.len() {
                    self.current_color = 0;
                }
                let c = self.colors[self.current_color].pixel;
                self.bgc.set_foreground(c);
            }
            d.win().fill_rectangle(&self.bgc, x, y, w, h);
            d.win().draw_rectangle(&self.fgc, x, y, w, h);
            return;
        }

        // Golden-ratio and Mondrian modes always cut the longer side.
        let side_by_side = if self.golden_ratio || self.mondrian {
            w > h
        } else {
            random() & 1 == 1
        };

        if side_by_side {
            let wnew = if self.golden_ratio {
                (w as f64 * if random() & 1 == 1 { PHI1 } else { PHI2 }) as i32
            } else {
                w / 2
            };
            self.deco(d, x, y, wnew, h, depth + 1);
            self.deco(d, x + wnew, y, w - wnew, h, depth + 1);
        } else {
            let hnew = if self.golden_ratio {
                (h as f64 * if random() & 1 == 1 { PHI1 } else { PHI2 }) as i32
            } else {
                h / 2
            };
            self.deco(d, x, y, w, hnew, depth + 1);
            self.deco(d, x, y + hnew, w, h - hnew, depth + 1);
        }
    }
}

impl Screenhack for Deco {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (w, h) = (self.width, self.height);
        d.win().fill_rectangle(&self.bgc, 0, 0, w, h);
        if self.mondrian {
            self.mondrian_set_sizes();
            self.fgc.set_line_width(self.line_width);
        }
        self.deco(d, 0, 0, w, h, 0);
        (self.delay.max(0) as u32).saturating_mul(1_000_000)
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }
}

const DEFAULTS: &[&str] = &[
    ".background:		black",
    ".foreground:		white",
    "*maxDepth:		12",
    "*minWidth:		20",
    "*minHeight:		20",
    "*lineWidth:          1",
    "*delay:		5",
    "*ncolors:		64",
    "*goldenRatio:        False",
    "*smoothColors:       False",
    "*mondrian:           False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Duration", 1.0, 60.0, 1.0, 0, "5"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
    Opt::slider("minWidth", "Minimum width", 1.0, 100.0, 1.0, 0, "20"),
    Opt::slider("minHeight", "Minimum height", 1.0, 100.0, 1.0, 0, "20"),
    Opt::slider("maxDepth", "Maximum depth", 1.0, 40.0, 1.0, 0, "12"),
    Opt::boolean("smoothColors", "Smooth colors", "false"),
    Opt::boolean("goldenRatio", "Golden ratio", "false"),
    Opt::boolean("mondrian", "Mondrian", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "deco",
    label: "Deco",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski and Michael Bayne",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=kfdDTv07Nhw"),
        blurb: "Subdivides the screen into rectangles, like a bad 1980s lobby.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
