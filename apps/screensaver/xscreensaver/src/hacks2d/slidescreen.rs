//! Port of `hacks/slidescreen.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2018 Jamie Zawinski <jwz@jwz.org>
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
//! A fifteen puzzle played on a picture. The image is ruled into cells with a
//! gutter between them, one cell irises open to make the hole, and from then on
//! a run of tiles slides into it a few pixels at a time, alternating between
//! horizontal and vertical moves. Nothing is stored: the tiles are the window
//! itself, moved about with overlapping blits.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::parse_color;
use crate::runtime::{
    About, Dpy, Gc, ImageLoad, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XEvent, XPoint,
    random_below, screenhack_event_helper,
};

const DOWN: usize = 0;
const LEFT: usize = 1;
const UP: usize = 2;
const RIGHT: usize = 3;

const VERTICAL: i32 = 0;
const HORIZONTAL: i32 = 1;

struct State {
    grid_size: i32,
    pix_inc: i32,
    border: i32,
    hole_x: i32,
    hole_y: i32,
    bitmap_w: i32,
    bitmap_h: i32,
    xoff: i32,
    yoff: i32,
    grid_w: i32,
    grid_h: i32,
    delay: u32,
    delay2: u32,
    duration: f64,
    gc: Gc,
    /// Upstream reads these two crossed over: `fg` is parsed from the
    /// background resource and `bg` from the foreground one. So `fg` is the
    /// black of the gutters and the hole, and `bg` is the grey of the thin
    /// outline inside each cell.
    fg: Pixel,
    bg: Pixel,
    max_width: i32,
    max_height: i32,
    early_i: i32,

    draw_rnd: i32,
    draw_i: i32,
    draw_x: i32,
    draw_y: i32,
    /// Where the sliding run started, which the trailing gap is filled from.
    draw_x0: i32,
    draw_y0: i32,
    draw_dx: i32,
    draw_dy: i32,
    draw_dir: usize,
    draw_w: i32,
    draw_h: i32,
    draw_size: i32,
    draw_inc: i32,
    draw_last: i32,
    draw_initted: bool,

    start_time: f64,
    img_loader: Option<ImageLoad>,
    loading: bool,
}

impl State {
    fn draw_grid(&mut self, d: &mut Dpy) {
        let (w, h) = (d.width(), d.height());
        let mut border = self.border;
        self.bitmap_w = w;
        self.bitmap_h = h;

        if w < 50 || h < 50 {
            // Tiny window.
            let s = w.min(h);
            border = 1;
            self.grid_size = (s / 2).max(16);
            self.bitmap_w = self.bitmap_w.max(self.grid_size * 2);
            self.bitmap_h = self.bitmap_h.max(self.grid_size * 2);
        }
        if w > 2560 || h > 2560 {
            border *= 2; // Retina displays.
        }

        self.grid_w = self.bitmap_w / self.grid_size;
        self.grid_h = self.bitmap_h / self.grid_size;
        self.hole_x = random_below(self.grid_w.max(1));
        self.hole_y = random_below(self.grid_h.max(1));
        self.xoff = (self.bitmap_w - self.grid_w * self.grid_size) / 2;
        self.yoff = (self.bitmap_h - self.grid_h * self.grid_size) / 2;

        self.early_i = -10;
        self.draw_last = -1;

        if border != 0 {
            let half = border / 2;
            let half2 = if border & 1 != 0 { half + 1 } else { half };
            self.gc.set_foreground(self.bg);
            let mut i = 0;
            while i < self.bitmap_w {
                let mut j = 0;
                while j < self.bitmap_h {
                    d.win().draw_rectangle(
                        &self.gc,
                        self.xoff + i + half2,
                        self.yoff + j + half2,
                        self.grid_size - border - 1,
                        self.grid_size - border - 1,
                    );
                    j += self.grid_size;
                }
                i += self.grid_size;
            }

            self.gc.set_foreground(self.fg);
            let mut i = 0;
            while i <= self.bitmap_w {
                d.win().fill_rectangle(
                    &self.gc,
                    self.xoff + i - half,
                    self.yoff,
                    border,
                    self.bitmap_h,
                );
                i += self.grid_size;
            }
            let mut i = 0;
            while i <= self.bitmap_h {
                d.win().fill_rectangle(
                    &self.gc,
                    self.xoff,
                    self.yoff + i - half,
                    self.bitmap_w,
                    border,
                );
                i += self.grid_size;
            }
        }

        if self.xoff != 0 {
            let (xo, bh, bw) = (self.xoff, self.bitmap_h, self.bitmap_w);
            d.win().fill_rectangle(&self.gc, 0, 0, xo, bh);
            d.win().fill_rectangle(&self.gc, bw - xo, 0, xo, bh);
        }
        if self.yoff != 0 {
            let (yo, bh, bw) = (self.yoff, self.bitmap_h, self.bitmap_w);
            d.win().fill_rectangle(&self.gc, 0, 0, bw, yo);
            d.win().fill_rectangle(&self.gc, 0, bh - yo, bw, yo);
        }
    }

    /// The hole opening: four triangles eating into the corners of one cell
    /// until the whole cell is gone.
    fn draw_early(&mut self, d: &mut Dpy) -> bool {
        if self.early_i < 0 {
            self.early_i += 1;
            return true;
        }

        let x0 = self.xoff + self.grid_size * self.hole_x;
        let y0 = self.yoff + self.grid_size * self.hole_y;
        let g = self.grid_size;
        let e = self.early_i;
        let tri = |d: &mut Dpy, gc: &Gc, a: (i32, i32), b: (i32, i32), c: (i32, i32)| {
            let pts = [
                XPoint { x: a.0, y: a.1 },
                XPoint { x: b.0, y: b.1 },
                XPoint { x: c.0, y: c.1 },
            ];
            d.win().fill_polygon(gc, &pts);
        };
        tri(d, &self.gc, (x0, y0), (x0 + g, y0), (x0, y0 + e));
        tri(d, &self.gc, (x0, y0), (x0, y0 + g), (x0 + e, y0 + g));
        tri(
            d,
            &self.gc,
            (x0 + g, y0 + g),
            (x0, y0 + g),
            (x0 + g, y0 + g - e),
        );
        tri(
            d,
            &self.gc,
            (x0 + g, y0 + g),
            (x0 + g, y0),
            (x0 + g - e, y0),
        );

        self.early_i += self.pix_inc;
        if self.early_i < self.grid_size {
            return true;
        }

        d.win().fill_rectangle(&self.gc, x0, y0, g, g);
        false
    }

    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    fn image_arrived(&mut self, d: &mut Dpy) {
        self.loading = false;
        self.start_time = d.time;
        self.draw_grid(d);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (w, h) = (d.width(), d.height());

    let mut grid_size = d.res.int("gridSize");
    if w > 2560 || h > 2560 {
        grid_size *= 2; // Retina displays.
    }
    // Do not let the grid be smaller than 5x5.
    while grid_size > w / 5 {
        grid_size /= 2;
    }
    while grid_size > h / 5 {
        grid_size /= 2;
    }
    grid_size = grid_size.max(1);

    // Crossed over, as upstream reads them.
    let fg = parse_color(d.res.string("background")).unwrap_or(0xFF00_0000);
    let bg = parse_color(d.res.string("foreground")).unwrap_or(0xFFBE_BEBE);

    let mut st = State {
        grid_size,
        pix_inc: d.res.int("pixelIncrement").max(1),
        border: d.res.int("internalBorderWidth").max(0),
        hole_x: 0,
        hole_y: 0,
        bitmap_w: w,
        bitmap_h: h,
        xoff: 0,
        yoff: 0,
        grid_w: 1,
        grid_h: 1,
        delay: d.res.int("delay").max(0) as u32,
        delay2: d.res.int("delay2").max(0) as u32,
        duration: d.res.int("duration").max(1) as f64,
        gc: Gc::new(fg, bg),
        fg,
        bg,
        max_width: w,
        max_height: h,
        early_i: -10,
        draw_rnd: 0,
        draw_i: 0,
        draw_x: 0,
        draw_y: 0,
        draw_x0: 0,
        draw_y0: 0,
        draw_dx: 0,
        draw_dy: 0,
        draw_dir: DOWN,
        draw_w: 1,
        draw_h: 1,
        draw_size: 1,
        draw_inc: 1,
        draw_last: -1,
        draw_initted: false,
        start_time: 0.0,
        img_loader: None,
        loading: false,
    };
    st.start_load(d);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // This code is a total kludge, but who cares, it works.
        let mut this_delay = self.delay;

        if self.loading {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return self.delay;
        }

        if self.start_time + self.duration < d.time {
            self.start_load(d);
            self.draw_initted = false;
            return self.delay;
        }

        if !self.draw_initted {
            return if self.draw_early(d) {
                self.delay
            } else {
                self.draw_initted = true;
                self.delay2
            };
        }

        if self.draw_i == 0 {
            if self.draw_last == -1 {
                self.draw_last = random_below(2);
            }

            // Alternate between horizontal and vertical slides. draw_dir is
            // the direction the hole moves, not the tiles.
            if self.draw_last == VERTICAL {
                // When there is only one column upstream leaves draw_rnd at
                // whatever the last move left in it.
                let probe = if self.grid_w > 1 {
                    self.draw_rnd = random_below(self.grid_w - 1);
                    self.draw_rnd
                } else {
                    0
                };
                if probe < self.hole_x {
                    self.draw_dx = -1;
                    self.draw_dir = LEFT;
                    self.hole_x -= self.draw_rnd;
                } else {
                    self.draw_dx = 1;
                    self.draw_dir = RIGHT;
                    self.draw_rnd -= self.hole_x;
                }
                self.draw_dy = 0;
                self.draw_size = self.draw_rnd + 1;
                self.draw_w = self.draw_size;
                self.draw_h = 1;
                self.draw_last = HORIZONTAL;
            } else {
                let probe = if self.grid_h > 1 {
                    self.draw_rnd = random_below(self.grid_h - 1);
                    self.draw_rnd
                } else {
                    0
                };
                if probe < self.hole_y {
                    self.draw_dy = -1;
                    self.draw_dir = UP;
                    self.hole_y -= self.draw_rnd;
                } else {
                    self.draw_dy = 1;
                    self.draw_dir = DOWN;
                    self.draw_rnd -= self.hole_y;
                }
                self.draw_dx = 0;
                self.draw_size = self.draw_rnd + 1;
                self.draw_h = self.draw_size;
                self.draw_w = 1;
                self.draw_last = VERTICAL;
            }

            self.draw_x = self.xoff + (self.hole_x + self.draw_dx) * self.grid_size;
            self.draw_x0 = self.draw_x;
            self.draw_y = self.yoff + (self.hole_y + self.draw_dy) * self.grid_size;
            self.draw_y0 = self.draw_y;
            self.draw_inc = self.pix_inc;
        }

        if self.draw_inc + self.draw_i > self.grid_size {
            self.draw_inc = self.grid_size - self.draw_i;
        }
        let mut tox = self.draw_x - self.draw_dx * self.draw_inc;
        let mut toy = self.draw_y - self.draw_dy * self.draw_inc;

        let fx = self.draw_x.clamp(0, self.max_width);
        let fy = self.draw_y.clamp(0, self.max_height);
        tox = tox.clamp(0, self.max_width);
        toy = toy.clamp(0, self.max_height);

        let (cw, ch) = (self.grid_size * self.draw_w, self.grid_size * self.draw_h);
        d.win().copy_area_self(&self.gc, fx, fy, cw, ch, tox, toy);

        self.draw_x -= self.draw_dx * self.draw_inc;
        self.draw_y -= self.draw_dy * self.draw_inc;
        match self.draw_dir {
            DOWN => d.win().fill_rectangle(
                &self.gc,
                self.draw_x0,
                self.draw_y + ch,
                cw,
                self.draw_y0 - self.draw_y,
            ),
            LEFT => d.win().fill_rectangle(
                &self.gc,
                self.draw_x0,
                self.draw_y0,
                self.draw_x - self.draw_x0,
                ch,
            ),
            UP => d.win().fill_rectangle(
                &self.gc,
                self.draw_x0,
                self.draw_y0,
                cw,
                self.draw_y - self.draw_y0,
            ),
            _ => d.win().fill_rectangle(
                &self.gc,
                self.draw_x + cw,
                self.draw_y0,
                self.draw_x0 - self.draw_x,
                ch,
            ),
        }

        self.draw_i += self.draw_inc;
        if self.draw_i >= self.grid_size {
            self.draw_i = 0;
            match self.draw_dir {
                DOWN => self.hole_y += self.draw_size,
                LEFT => self.hole_x -= 1,
                UP => self.hole_y -= 1,
                _ => self.hole_x += self.draw_size,
            }
            this_delay = self.delay2;
        }

        this_delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.max_width = width;
        self.max_height = height;
        if !self.loading {
            self.draw_initted = false;
            self.start_load(d);
        }
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
    "*fpsSolid: true",
    ".background: Black",
    ".foreground: #BEBEBE",
    "*gridSize: 70",
    "*pixelIncrement: 10",
    "*internalBorderWidth: 4",
    "*delay: 50000",
    "*delay2: 1000000",
    "*duration: 120",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "50000").inverted(),
    Opt::slider("delay2", "Pause", 0.0, 2_000_000.0, 50_000.0, 0, "1000000"),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::slider("pixelIncrement", "Slide speed", 1.0, 30.0, 1.0, 0, "10"),
    Opt::slider("gridSize", "Cell size", 12.0, 500.0, 1.0, 0, "70"),
    Opt::spin("internalBorderWidth", "Gutter size", 0.0, 50.0, "4"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "slidescreen",
    label: "Slide Screen",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1994",
        video: Some("https://www.youtube.com/watch?v=uKNE4xCdlno"),
        blurb: "A fifteen puzzle variant, dividing the image into a grid and shuffling.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
