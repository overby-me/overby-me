//! Port of `hacks/imsmap.c`.
//!
//! ```text
//! imsmap, Copyright (c) 1992-2013 Juergen Nickelsen and Jamie Zawinski.
//! Derived from code by Markus Schirmer, TU Berlin.
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Revision History:
//! 24-aug-92: jwz: hacked.
//! 17-May-97: jwz: hacked more.
//! ```
//!
//! Cloud-like fractal patterns by repeated subdivision: each pass halves the
//! grid spacing and fills in the new midpoints from the average of their
//! neighbours plus a random offset that halves along with the spacing. Cells
//! are drawn as they are computed, so the map coarsens in and then sharpens.
//! One run in five is "extra krinkly", where a height that runs off the end of
//! the colour map wraps round instead of clamping, which puts hard seams
//! through the clouds.
//!
//! Upstream's mono path, which dithers the map with Floyd-Steinberg, is left
//! out: it needs a colour count of two or fewer, and a canvas is always
//! TrueColor.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XColor, XEvent,
    random_below, screenhack_event_helper,
};

const NSTEPS: i32 = 7;
const COUNT: i32 = 1 << NSTEPS;

struct Imsmap {
    ncolors: usize,
    colors: Vec<XColor>,
    extra_krinkly: bool,
    delay: u32,
    delay2: u32,
    /// The height field, at one signed byte a cell as upstream stores it.
    cell: Vec<i8>,
    xmax: i32,
    ymax: i32,
    iteration: i32,
    iterations: i32,
    cx: i32,
    xstep: i32,
    ystep: i32,
    xnext_step: i32,
    ynext_step: i32,
    last_pixel: Pixel,
    last_valid: bool,
    /// Whether to mirror and transpose the map on its way to the screen.
    flip_x: bool,
    flip_xy: bool,
    gc: Gc,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut st = Imsmap {
        ncolors: 0,
        colors: Vec::new(),
        extra_krinkly: false,
        delay: 0,
        delay2: 0,
        cell: Vec::new(),
        xmax: 0,
        ymax: 0,
        iteration: 0,
        iterations: 7,
        cx: 0,
        xstep: COUNT,
        ystep: COUNT,
        xnext_step: 0,
        ynext_step: 0,
        last_pixel: 0,
        last_valid: false,
        flip_x: false,
        flip_xy: false,
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
    };
    st.init_map(d);
    Box::new(st)
}

impl Imsmap {
    fn cell(&self, c: i32, r: i32) -> i32 {
        self.cell[(c + r * self.xmax) as usize] as i32
    }

    fn set_cell(&mut self, c: i32, r: i32, v: i32) {
        let at = (c + r * self.xmax) as usize;
        self.cell[at] = v as i8;
    }

    /// `HEIGHT_TO_PIXEL`: fold a height into the colour map, either by clamping
    /// or, when krinkly, by wrapping.
    fn height_to_pixel(&self, height: i32) -> usize {
        let n = self.ncolors as i32;
        if height < 0 {
            if self.extra_krinkly {
                (n - 1 - ((-height) % n)) as usize
            } else {
                0
            }
        } else if height >= n {
            if self.extra_krinkly {
                (height % n) as usize
            } else {
                (n - 1) as usize
            }
        } else {
            height as usize
        }
    }

    fn set(&mut self, l: i32, c: i32, size: i32, height: i32) -> Pixel {
        let rang = 1 << (NSTEPS - size);
        let height = height + random_below(rang) - rang / 2;
        let height = self.height_to_pixel(height);
        self.set_cell(l, c, height as i32);
        self.colors[height].pixel
    }

    fn draw_cell(&mut self, d: &mut Dpy, x: i32, y: i32, pixel: Pixel, grid_size: i32) {
        let mut x = if self.flip_x { self.xmax - x } else { x };
        let mut y = y;
        if self.flip_xy {
            std::mem::swap(&mut x, &mut y);
        }

        if !(self.last_valid && pixel == self.last_pixel) {
            self.gc.set_foreground(pixel);
        }
        self.last_valid = true;
        self.last_pixel = pixel;

        if grid_size == 1 {
            d.win().draw_point(&self.gc, x, y);
        } else {
            d.win().fill_rectangle(&self.gc, x, y, grid_size, grid_size);
        }
    }

    fn init_map(&mut self, d: &mut Dpy) {
        self.flip_x = random_below(2) == 1;
        self.flip_xy = random_below(2) == 1;

        self.ncolors = d.res.int("ncolors").clamp(3, 255) as usize;
        self.delay = d.res.int("delay").max(0) as u32;
        self.delay2 = d.res.int("delay2").max(0) as u32;
        self.iterations = d.res.int("iterations").clamp(0, 7);

        self.extra_krinkly = random_below(5) == 0;
        self.colors = make_smooth_colormap(self.ncolors);

        let c = self.colors[1].pixel;
        self.gc.set_foreground(c);
        let (w, h) = (d.width(), d.height());
        d.win().fill_rectangle(&self.gc, 0, 0, w, h);
        self.last_valid = false;

        if self.flip_xy {
            self.xmax = h;
            self.ymax = w;
        } else {
            self.xmax = w;
            self.ymax = h;
        }
        self.cell = vec![0; (self.xmax * self.ymax) as usize];

        self.xstep = COUNT;
        self.ystep = COUNT;
        self.iteration = 0;
        self.cx = 0;
    }
}

impl Screenhack for Imsmap {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut this_delay = self.delay2;
        // Do this many columns at a time without pausing. The finer the grid,
        // the more of it fits in one frame.
        let col_chunk = self.iteration * 2 + 1;

        if self.iteration > self.iterations {
            self.init_map(d);
        }

        if self.cx == 0 {
            self.xnext_step = self.xstep / 2;
            self.ynext_step = self.ystep / 2;
        }

        for _ in 0..col_chunk {
            let x = self.cx;

            let mut x1 = x + self.xnext_step;
            if x1 < 0 {
                x1 = self.xmax - 1;
            } else if x1 >= self.xmax {
                x1 = 0;
            }

            let mut x2 = x + self.xstep;
            if x2 < 0 {
                x2 = self.xmax - 1;
            } else if x2 >= self.xmax {
                x2 = 0;
            }

            let mut y = 0;
            while y < self.ymax {
                let mut y1 = y + self.ynext_step;
                if y1 < 0 {
                    y1 = self.ymax - 1;
                } else if y1 >= self.ymax {
                    y1 = 0;
                }

                let mut y2 = y + self.ystep;
                if y2 < 0 {
                    y2 = self.ymax - 1;
                } else if y2 >= self.ymax {
                    y2 = 0;
                }

                // The four corners this pass is subdividing.
                let q = [
                    self.colors[self.height_to_pixel(self.cell(x, y))].pixel,
                    self.colors[self.height_to_pixel(self.cell(x, y2))].pixel,
                    self.colors[self.height_to_pixel(self.cell(x2, y))].pixel,
                    self.colors[self.height_to_pixel(self.cell(x2, y2))].pixel,
                ];
                let same = |p: Pixel| q.iter().all(|c| *c == p);

                let it = self.iteration;
                let grid = self.ynext_step;

                let h = (self.cell(x, y) + self.cell(x, y2) + 1) / 2;
                let pixel = self.set(x, y1, it, h);
                if !same(pixel) {
                    self.draw_cell(d, x, y1, pixel, grid);
                }

                let h = (self.cell(x, y) + self.cell(x2, y) + 1) / 2;
                let pixel = self.set(x1, y, it, h);
                if !same(pixel) {
                    self.draw_cell(d, x1, y, pixel, grid);
                }

                let h =
                    (self.cell(x, y) + self.cell(x, y2) + self.cell(x2, y) + self.cell(x2, y2) + 2)
                        / 4;
                let pixel = self.set(x1, y1, it, h);
                if !same(pixel) {
                    self.draw_cell(d, x1, y1, pixel, grid);
                }

                y += self.ystep;
            }

            self.cx += self.xstep;
            if self.cx >= self.xmax {
                break;
            }
        }

        if self.cx >= self.xmax {
            self.cx = 0;
            self.xstep = self.xnext_step;
            self.ystep = self.ynext_step;
            self.iteration += 1;

            // The finished map lingers before the next one starts.
            if self.iteration > self.iterations {
                this_delay = self.delay.saturating_mul(1_000_000);
            }
        }

        this_delay
    }

    fn reshape(&mut self, d: &mut Dpy, _width: i32, _height: i32) {
        self.init_map(d);
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.init_map(d);
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: #000066",
    ".foreground: #FF00FF",
    "*fpsSolid: true",
    "*ncolors: 50",
    "*iterations: 7",
    "*delay: 5",
    "*delay2: 20000",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay2", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("delay", "Linger", 1.0, 60.0, 1.0, 0, "5"),
    Opt::slider("iterations", "Density", 1.0, 7.0, 1.0, 0, "7"),
    Opt::slider("ncolors", "Number of colors", 3.0, 255.0, 1.0, 0, "50"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "imsmap",
    label: "IMS Map",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Juergen Nickelsen and Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=FP8YJzFkdoQ"),
        blurb: "Recursive cloud-like fractal patterns.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
