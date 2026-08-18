//! Port of `hacks/cynosure.c`.
//!
//! ```text
//! cynosure --- draw some rectangles
//!
//! 01-aug-96: written in Java by ozymandias G desiderata <ogd@organic.com>
//! 25-dec-97: ported to C and XScreenSaver by Jamie Zawinski <jwz@jwz.org>
//!
//! Original version:
//!   http://www.organic.com/staff/ogd/java/cynosure.html
//!
//! Original comments and copyright:
//!
//!   Cynosure.java
//!   A Java implementation of Stephen Linhart's Cynosure screen-saver as a
//!   drop-in class.
//!
//!   ozymandias G desiderata <ogd@organic.com>
//!   Thu Aug  1 1996
//!
//!   COPYRIGHT NOTICE
//!
//!   Copyright 1996 ozymandias G desiderata. Title, ownership rights, and
//!   intellectual property rights in and to this software remain with
//!   ozymandias G desiderata. This software may be copied, modified,
//!   or used as long as this copyright is retained. Use this code at your
//!   own risk.
//! ```
//!
//! Dropshadowed rectangles, laid down a gridful at a time. Each row of the
//! grid gets one colour, and that colour usually drifts a little way from the
//! last row's, so a layer reads as a family of shades; one time in a hundred it
//! jumps somewhere else entirely. Layers pile up until the screen is repainted
//! in a fresh background colour and it starts again.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{hsv_to_rgb, make_smooth_colormap, rgb_to_hsv};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, XEvent, random,
    screenhack_event_helper,
};

/// The smallest size for an individual cell.
const MIN_CELL_SIZE: i32 = 16;
/// The narrowest a rectangle can be.
const MIN_RECT_SIZE: i32 = 6;
/// One call in this many to `gen_new_color` returns something unrelated to the
/// current palette.
const THRESHOLD: u32 = 100;

struct Cynosure {
    colors: Vec<XColor>,
    /// The same colours darkened, for the dropshadows.
    shadows: Vec<XColor>,
    ncolors: usize,
    fg_gc: Gc,
    bg_gc: Gc,
    shadow_gc: Gc,
    cur_color: usize,
    /// Colour progression.
    cur_base: usize,
    shadow_width: i32,
    /// Offset of the dropshadow.
    elevation: i32,
    /// Time until the base colour is changed, and how long is left.
    sway: i32,
    time_left: i32,
    /// Amount of colour variance.
    tweak: i32,
    grid_size: i32,
    iterations: i32,
    i: i32,
    delay: u32,
    width: i32,
    height: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut shadow_width = d.res.int("shadowWidth");
    let mut elevation = d.res.int("elevation");
    let mut lw = 1;
    if d.width() > 2560 || d.height() > 2560 {
        // Retina displays.
        shadow_width *= 2;
        elevation *= 2;
        lw *= 2;
    }

    let ncolors = d.res.int("colors").max(2) as usize;
    let colors = make_smooth_colormap(ncolors);
    // Darkened copies, for the shadows.
    let shadows = colors
        .iter()
        .map(|c| {
            let (h, s, v) = rgb_to_hsv(c.red, c.green, c.blue);
            let (r, g, b) = hsv_to_rgb(h, s, v * 0.4);
            XColor::from_rgb16(r, g, b)
        })
        .collect();

    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");
    let mut fg_gc = Gc::new(fg, bg);
    fg_gc.set_line_width(lw);
    let mut bg_gc = Gc::new(bg, bg);
    bg_gc.set_line_width(lw);
    let mut shadow_gc = Gc::new(fg, bg);
    shadow_gc.set_line_width(lw);

    Box::new(Cynosure {
        ncolors: colors.len(),
        colors,
        shadows,
        fg_gc,
        bg_gc,
        shadow_gc,
        cur_color: 0,
        cur_base: 0,
        shadow_width,
        elevation,
        sway: d.res.int("sway").max(1),
        time_left: 0,
        tweak: d.res.int("tweak").max(1),
        grid_size: d.res.int("gridSize").max(2),
        iterations: d.res.int("iterations"),
        i: 0,
        delay: d.res.int("delay").max(0) as u32,
        width: d.width(),
        height: d.height(),
    })
}

/// Generate a value near `base`, skewed by up to `tweak` either way, folded
/// back to positive and capped.
fn c_tweak(base: i32, tweak: i32) -> i32 {
    let ran = (random() % (2 * tweak).max(1) as u32) as i32;
    let n = base + (ran - tweak);
    let n = if n < 0 { -n } else { n };
    n.min(255)
}

impl Cynosure {
    /// A colour within `tweak` of an existing one, wrapped into the map.
    fn gen_constrained_color(&self, base: usize) -> usize {
        let mut i = 1 + (random() % self.tweak as u32) as i32;
        if random() & 1 == 1 {
            i = -i;
        }
        let n = self.ncolors as i32;
        let mut i = (base as i32 + i) % n;
        while i < 0 {
            i += n;
        }
        i as usize
    }

    /// Mutate the colour gradually, and now and then jump somewhere else.
    fn gen_new_color(&mut self) -> usize {
        // After enough calls, whatever was most recently generated becomes the
        // new base.
        if self.time_left == 0 {
            self.time_left = c_tweak(self.sway, self.sway / 3);
            self.cur_color = self.cur_base;
        } else {
            self.time_left -= 1;
        }

        if random().is_multiple_of(THRESHOLD) {
            (random() % self.ncolors as u32) as usize
        } else {
            self.cur_base = self.gen_constrained_color(self.cur_color);
            self.cur_base
        }
    }

    /// Lay down one gridful of rectangles: a row per colour, each rectangle
    /// randomly sized and placed inside its own cell.
    fn paint(&mut self, d: &mut Dpy) {
        let (width, height) = (self.width, self.height);

        let mut cells_wide = c_tweak(self.grid_size, self.grid_size / 2).max(1);
        let mut cells_high = c_tweak(self.grid_size, self.grid_size / 2).max(1);
        let mut cell_width = width / cells_wide;
        let mut cell_height = height / cells_high;

        // Each cell has to be above a certain minimum size.
        if cell_width < MIN_CELL_SIZE {
            cell_width = MIN_CELL_SIZE;
            cells_wide = width / cell_width;
        }
        if cell_height < MIN_CELL_SIZE {
            cell_height = MIN_CELL_SIZE;
            // Upstream measures the row count off the width here.
            cells_high = width / cell_width;
        }
        if cell_width <= self.shadow_width || cell_height <= self.shadow_width {
            return;
        }

        for i in 0..cells_high {
            let c = self.gen_new_color();
            let fg = self.colors[c].pixel;
            self.fg_gc.set_foreground(fg);
            let sh = self.shadows[c].pixel;
            self.shadow_gc.set_foreground(sh);

            for j in 0..cells_wide {
                let mut cur_height = (random() % (cell_height - self.shadow_width) as u32) as i32;
                if cur_height < MIN_RECT_SIZE {
                    cur_height = MIN_RECT_SIZE;
                }
                let mut cur_width = (random() % (cell_width - self.shadow_width) as u32) as i32;
                if cur_width < MIN_RECT_SIZE {
                    cur_width = MIN_RECT_SIZE;
                }

                let room_y = ((cell_height - cur_height) - self.shadow_width).max(1);
                let room_x = ((cell_width - cur_width) - self.shadow_width).max(1);
                let cur_y = (i * cell_height) + (random() % room_y as u32) as i32;
                let cur_x = (j * cell_width) + (random() % room_x as u32) as i32;

                if self.elevation > 0 {
                    d.win().fill_rectangle(
                        &self.shadow_gc,
                        cur_x + self.elevation,
                        cur_y + self.elevation,
                        cur_width,
                        cur_height,
                    );
                }
                if self.shadow_width > 0 {
                    d.win().fill_rectangle(
                        &self.bg_gc,
                        cur_x + self.shadow_width,
                        cur_y + self.shadow_width,
                        cur_width,
                        cur_height,
                    );
                }
                d.win()
                    .fill_rectangle(&self.fg_gc, cur_x, cur_y, cur_width, cur_height);
                d.win()
                    .draw_rectangle(&self.bg_gc, cur_x, cur_y, cur_width, cur_height);
            }
        }
    }
}

impl Screenhack for Cynosure {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.i += 1;
        if self.iterations > 0 && self.i >= self.iterations {
            self.i = 0;
            // Repaint in a fresh background colour and start piling up again.
            let c = self.colors[(random() % self.ncolors as u32) as usize].pixel;
            self.bg_gc.set_foreground(c);
            let (w, h) = (self.width, self.height);
            d.win().fill_rectangle(&self.bg_gc, 0, 0, w, h);
            let bg = self.bg_gc.background;
            self.bg_gc.set_foreground(bg);
        }
        self.paint(d);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.i = self.iterations;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*delay: 500000",
    "*colors: 128",
    "*iterations: 100",
    "*shadowWidth: 2",
    "*elevation: 5",
    "*sway: 30",
    "*tweak: 20",
    "*gridSize: 12",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "delay",
        "Frame rate",
        0.0,
        1_000_000.0,
        10000.0,
        0,
        "500000",
    )
    .inverted(),
    Opt::slider("colors", "Number of colors", 2.0, 255.0, 1.0, 0, "128"),
    Opt::slider("iterations", "Duration", 2.0, 200.0, 1.0, 0, "100"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cynosure",
    label: "Cynosure",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ozymandias G. Desiderata, Jamie Zawinski, and Stephen Linhart",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=If7FOc8UnYs"),
        blurb: "Random dropshadowed rectangles pop onto the screen in lockstep.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
