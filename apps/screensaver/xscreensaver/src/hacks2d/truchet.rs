//! Port of `hacks/truchet.c`.
//!
//! ```text
//! truchet --- curved and straight tilings
//! Copyright (c) 1998 Adrian Likins <adrian@gimp.org>
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
//! Truchet tiles: a grid where every cell gets one of two orientations of the
//! same motif, either a pair of quarter-circles or a pair of diagonals. The
//! motif joins up across cell edges either way, so a coin toss per cell draws
//! one continuous meandering pattern. Every frame picks a fresh colour, tile
//! size and line width and lays down another one.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, random, random_below,
};

/// The widest a tile may be relative to its height, and the other way round.
const MAXRATIO: i32 = 2;

struct State {
    agc: Gc,
    bgc: Gc,
    /// The tile size, which changes every frame.
    width: i32,
    height: i32,
    /// The window size, which is what the tiling has to cover.
    win_width: i32,
    win_height: i32,
    frame: Pixmap,
    overlap: i32,

    maxlinewidth: i32,
    minlinewidth: i32,
    minwidth: i32,
    minheight: i32,
    max_width: i32,
    max_height: i32,
    delay: u32,
    count: i32,
    anim_delay: u32,
    anim_step_size: i32,

    curves: bool,
    square: bool,
    angles: bool,
    erase: bool,
    erase_count: i32,
    scroll: bool,
    scrolling: i32,
}

impl State {
    /// Two diagonals per cell, cutting opposite pairs of edge midpoints.
    fn draw_angles(&mut self) {
        let (w, h) = (self.width, self.height);
        let mut cy = 0;
        while self.win_height + self.overlap > cy * h {
            let mut cx = 0;
            while self.win_width + self.overlap > cx * w {
                let (x, y) = (cx * w, cy * h);
                if random() % 2 == 1 {
                    self.frame
                        .draw_line(&self.agc, x + w / 2, y, x + w, y + h / 2);
                    self.frame
                        .draw_line(&self.agc, x, y + h / 2, x + w / 2, y + h);
                } else {
                    self.frame.draw_line(&self.agc, x + w / 2, y, x, y + h / 2);
                    self.frame
                        .draw_line(&self.agc, x + w, y + h / 2, x + w / 2, y + h);
                }
                cx += 1;
            }
            cy += 1;
        }
    }

    /// Two quarter-circles per cell, centred on opposite corners.
    fn draw_truchet(&mut self) {
        let (w, h) = (self.width, self.height);
        let mut cy = 0;
        while self.win_height + self.overlap > cy * h {
            let mut cx = 0;
            while self.win_width + self.overlap > cx * w {
                let (x, y) = (cx * w, cy * h);
                if random() % 2 == 1 {
                    self.frame
                        .draw_arc(&self.agc, x - w / 2, y - h / 2, w, h, 0, -5760);
                    self.frame
                        .draw_arc(&self.agc, x + w / 2, y + h / 2, w, h, 11520, -5760);
                } else {
                    self.frame
                        .draw_arc(&self.agc, x + w / 2, y - h / 2, w, h, 17280, -5760);
                    self.frame
                        .draw_arc(&self.agc, x - w / 2, y + h / 2, w, h, 0, 5760);
                }
                cx += 1;
            }
            cy += 1;
        }
    }

    /// Show a different window onto the oversized frame each tick, so the
    /// pattern appears to drift. The path is a diamond: four legs, each one
    /// `scroll` pixels long, chosen off the countdown.
    fn scroll_area(&mut self, d: &mut Dpy) {
        let offset = self.overlap / 2;
        let scroll = self.overlap / 4;
        let legs = (scroll / self.anim_step_size).max(1);

        let direction = self.scrolling / legs;
        let progress = (self.scrolling % legs) * self.anim_step_size;

        let (mut sx, mut sy) = if direction & 1 == 1 {
            (progress - scroll, progress)
        } else {
            (-progress, progress - scroll)
        };
        if direction & 2 == 2 {
            sx = -sx;
            sy = -sy;
        }

        let (w, h) = (self.win_width, self.win_height);
        d.win()
            .copy_area(&self.agc, &self.frame, sx + offset, sy + offset, w, h, 0, 0);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let background = d.res.pixel("background");
    let white = d.res.pixel("foreground");

    let mut st = State {
        agc: Gc::new(white, white),
        bgc: Gc::new(background, background),
        width: 60,
        height: 60,
        win_width: d.width(),
        win_height: d.height(),
        frame: Pixmap::new(1, 1),
        overlap: d.res.int("scroll-overlap").max(0),

        maxlinewidth: d.res.int("maxLineWidth").max(1),
        minlinewidth: d.res.int("minLineWidth").max(1),
        minwidth: d.res.int("minWidth").max(1),
        minheight: d.res.int("minHeight").max(1),
        max_width: d.res.int("max-Width").max(1),
        max_height: d.res.int("max-Height").max(1),
        delay: d.res.int("delay").max(0) as u32,
        count: 0,
        anim_delay: d.res.int("anim-delay").max(0) as u32,
        anim_step_size: d.res.int("anim-step-size").max(1),

        curves: d.res.bool("curves"),
        square: d.res.bool("square"),
        angles: d.res.bool("angles"),
        erase: d.res.bool("erase"),
        erase_count: d.res.int("eraseCount").max(1),
        scroll: d.res.bool("scroll"),
        scrolling: 0,
    };

    // The author's own favourite command lines, one picked at random.
    if d.res.bool("randomize") {
        match random_below(12) {
            0 => {}
            1 => st.curves = false,
            2 => {
                st.curves = false;
                st.square = true;
                st.erase = false;
            }
            3 => {
                st.square = true;
                st.erase = false;
                st.erase_count = 5;
            }
            4 => st.scroll = true,
            5 => {
                st.scroll = true;
                st.erase = false;
                st.anim_step_size = 9;
            }
            6 => {
                st.angles = false;
                st.minwidth = 36;
                st.max_width = 36;
            }
            7 => {
                st.curves = false;
                st.minwidth = 12;
                st.max_width = 12;
            }
            8 => {
                st.curves = false;
                st.erase = false;
                st.minwidth = 36;
                st.max_width = 36;
            }
            9 => {
                st.erase = false;
                st.minwidth = 256;
                st.max_width = 512;
                st.minlinewidth = 96;
            }
            10 => {
                st.angles = false;
                st.minwidth = 64;
                st.max_width = 128;
                st.maxlinewidth = 4;
            }
            _ => {
                st.curves = false;
                st.minwidth = 64;
                st.max_width = 128;
                st.maxlinewidth = 4;
            }
        }
    }

    let (w, h) = (d.width(), d.height());
    d.win().fill_rectangle(&st.bgc, 0, 0, w, h);

    st.frame = Pixmap::new(w + st.overlap, h + st.overlap);
    st.frame
        .fill_rectangle(&st.bgc, 0, 0, w + st.overlap, h + st.overlap);

    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.scrolling > 0 {
            self.scrolling -= 1;
            self.scroll_area(d);
            return self.anim_delay * 1000;
        }

        // Borrowed from munch: a fresh colour every frame, picked in the
        // 16-bit components an XColor carries.
        let chan = || ((random() % 65535) >> 8) as u8;
        self.agc
            .set_foreground(crate::runtime::color::rgb(chan(), chan(), chan()));

        let mut linewidth = random_below(self.maxlinewidth);
        if linewidth < self.minlinewidth {
            linewidth = self.minlinewidth;
        }
        // An odd line width seems to work a little better, says the author.
        if linewidth % 2 == 1 {
            linewidth += 1;
        }

        self.width = random_below(self.max_width);
        self.height = random_below(self.max_height);

        if self.width == 0 || self.height == 0 {
            self.height = self.max_height;
            self.width = self.max_width;
        }
        if self.height < self.minheight {
            self.height = self.minheight;
        }
        if self.width < self.minwidth {
            self.width = self.minwidth;
        }
        if self.square {
            self.height = self.width;
        }
        if self.width / self.height > MAXRATIO {
            self.height = self.width;
        }
        if self.height / self.width > MAXRATIO {
            self.width = self.height;
        }

        if linewidth == 0 || linewidth < self.minlinewidth {
            linewidth = self.minlinewidth;
        }
        if linewidth > 0 && linewidth >= self.height / 5 {
            linewidth = self.height / 5;
        }
        self.agc.set_line_width(linewidth);

        if self.erase || self.count >= self.erase_count {
            self.frame.fill_rectangle(
                &self.bgc,
                0,
                0,
                self.win_width + self.overlap,
                self.win_height + self.overlap,
            );
            self.count = 0;
        }

        if !self.scroll {
            self.overlap = 0;
        }

        if self.curves && self.angles {
            if random() % 2 == 1 {
                self.draw_truchet();
            } else {
                self.draw_angles();
            }
        } else if self.curves {
            self.draw_truchet();
        } else if self.angles {
            self.draw_angles();
        }

        self.count += 1;

        if self.scroll {
            self.scrolling = (self.overlap / 4 / self.anim_step_size) * 4;
            return 0;
        }

        let (w, h) = (self.win_width, self.win_height);
        d.win().copy_area(&self.agc, &self.frame, 0, 0, w, h, 0, 0);

        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.win_width = width;
        self.win_height = height;
        // Upstream leaves the frame at its old size here, which on X only ever
        // meant a slightly clipped pattern. A canvas is resized far more often
        // than an X window was, so grow it instead of showing a torn edge.
        self.frame = Pixmap::new(width + self.overlap, height + self.overlap);
        self.frame
            .fill_rectangle(&self.bgc, 0, 0, width + self.overlap, height + self.overlap);
        d.win().fill_rectangle(&self.bgc, 0, 0, width, height);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*minWidth: 40",
    "*minHeight: 40",
    "*max-Width: 150",
    "*max-Height: 150",
    "*maxLineWidth: 25",
    "*minLineWidth: 2",
    "*erase: True",
    "*eraseCount: 25",
    "*square: True",
    "*delay: 400000",
    "*curves: True",
    "*angles: True",
    "*scroll: False",
    "*scroll-overlap: 400",
    "*anim-delay: 100",
    "*anim-step-size: 3",
    "*randomize: true",
];

const OPTS: &[Opt] = &[Opt::slider(
    "delay",
    "Frame rate",
    0.0,
    1_000_000.0,
    10_000.0,
    0,
    "400000",
)
.inverted()];

pub static DEF: SaverDef = SaverDef {
    slug: "truchet",
    label: "Truchet",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Adrian Likins",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=hoJ23JSsUD8"),
        blurb: "Line- and arc-based truchet patterns that tile the screen.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
