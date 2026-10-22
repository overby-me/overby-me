//! Port of `hacks/coral.c`.
//!
//! ```text
//! Copyright (c) 1997 by Frederick G.M. Roeber
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
//! Diffusion-limited aggregation. Scatter a crowd of random walkers and a
//! handful of sticky seeds; a walker that steps onto sticky ground sticks
//! there and makes its neighbours sticky too. What grows is a coral, and the
//! colour walks the map as the walkers are used up, so the growth rings are
//! visible.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_uniform_colormap;
use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, XEvent, XPoint,
    XRectangle, random, random_below, screenhack_event_helper,
};

/// Upstream's `NCOLORSMAX`.
const NCOLORS_MAX: usize = 200;

/// Rectangles buffered before they are drawn. Upstream's `max_points`.
const MAX_POINTS: usize = 200;

struct Coral {
    draw_gc: Gc,
    default_fg_pixel: crate::runtime::Pixel,
    colors: Vec<XColor>,
    colorindex: usize,
    /// Walkers used up between colour steps, so the map is walked exactly once
    /// as the coral fills in.
    colorsloth: usize,

    walkers: Vec<XPoint>,
    width: i32,
    height: i32,
    delay: i32,
    delay2: i32,
    pointbuf: Vec<XRectangle>,
    scale: i32,

    /// Sticky ground, one bit per pixel.
    board: Vec<u32>,
    widthb: i32,

    done: bool,
    reset: bool,
    eraser: Option<Eraser>,
    /// Two bits of randomness at a time, as upstream does to conserve calls.
    rand_bits: u32,
    rand_left: u32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    let mut st = Coral {
        draw_gc: Gc::new(fg, d.res.pixel("background")),
        default_fg_pixel: fg,
        colors: Vec::new(),
        colorindex: 0,
        colorsloth: 0,
        walkers: Vec::new(),
        width: 0,
        height: 0,
        delay: d.res.int("delay"),
        delay2: d.res.int("delay2"),
        pointbuf: Vec::with_capacity(MAX_POINTS),
        scale: 1,
        board: Vec::new(),
        widthb: 0,
        done: false,
        reset: true,
        eraser: None,
        rand_bits: 0,
        rand_left: 0,
    };
    st.reset = true;
    st.default_fg_pixel = fg;
    Box::new(st)
}

impl Coral {
    #[inline]
    fn getdot(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return false;
        }
        self.board[(y * self.widthb + (x >> 5)) as usize] & (1 << (x & 31)) != 0
    }

    #[inline]
    fn setdot(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        self.board[(y * self.widthb + (x >> 5)) as usize] |= 1 << (x & 31);
    }

    /// Two bits of randomness, conserving calls to the generator. Upstream
    /// measures this at 5-10%, which is worth having in the walker loop.
    fn rand_2(&mut self) -> u32 {
        if self.rand_left == 0 {
            self.rand_left = 16;
            self.rand_bits = random();
        }
        self.rand_left -= 1;
        let j = self.rand_bits & 3;
        self.rand_bits >>= 2;
        j
    }

    fn init_coral(&mut self, d: &mut Dpy) {
        d.clear_window();
        self.width = d.width();
        self.widthb = (d.width() + 31) >> 5;
        self.height = d.height();

        // Retina displays: a one-pixel walker is invisible on a huge window.
        self.scale = if self.width > 2560 || self.height > 2560 {
            2
        } else {
            1
        };

        self.board = vec![0u32; (self.widthb * self.height) as usize];
        self.colors = make_uniform_colormap(NCOLORS_MAX);
        self.colorindex = (random() as usize) % self.colors.len().max(1);

        let mut density = d.res.int("density");
        if density < 1 {
            density = 1;
        }
        if density > 100 {
            density = 90; // more like mold than coral
        }
        let nwalkers = ((self.width as i64 * self.height as i64 * density as i64) / 100) as usize;

        let mut seeds = d.res.int("seeds");
        seeds = seeds.clamp(1, 1000);

        self.colorsloth = nwalkers * 2 / self.colors.len().max(1);
        let color = self.colors[self.colorindex].pixel;
        self.draw_gc.set_foreground(color);

        if self.width <= 2 || self.height <= 2 {
            self.walkers.clear();
            return;
        }

        for _ in 0..seeds {
            let mut max_repeat = 10;
            let (x, y) = loop {
                let x = 1 + random_below(self.width - 2);
                let y = 1 + random_below(self.height - 2);
                if !self.getdot(x, y) || max_repeat == 0 {
                    break (x, y);
                }
                max_repeat -= 1;
            };
            for dy in -1..=1 {
                for dx in -1..=1 {
                    self.setdot(x + dx, y + dy);
                }
            }
            d.win().draw_point(&self.draw_gc, x, y);
        }

        self.walkers = (0..nwalkers)
            .map(|_| XPoint {
                x: random_below(self.width - 2) + 1,
                y: random_below(self.height - 2) + 1,
            })
            .collect();
    }

    /// One pass over every walker. Returns true when they have all stuck.
    fn step(&mut self, d: &mut Dpy) -> bool {
        let mut i = 0;
        while i < self.walkers.len() {
            let x = self.walkers[i].x;
            let y = self.walkers[i].y;

            if self.getdot(x, y) {
                self.pointbuf.push(XRectangle {
                    x,
                    y,
                    width: self.scale,
                    height: self.scale,
                });

                // Mark the surrounding area as sticky.
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if dx != 0 || dy != 0 {
                            self.setdot(x + dx, y + dy);
                        }
                    }
                }

                // Swap the last walker into this slot rather than shifting.
                let last = self.walkers.len() - 1;
                self.walkers[i] = self.walkers[last];
                self.walkers.pop();

                // Upstream: `0 == (colorsloth ? nwalkers % colorsloth : 0)`,
                // so a zero sloth steps the colour on every stick.
                let color =
                    self.colorsloth == 0 || self.walkers.len().is_multiple_of(self.colorsloth);

                if color || self.walkers.is_empty() || self.pointbuf.len() >= MAX_POINTS {
                    d.win().fill_rectangles(&self.draw_gc, &self.pointbuf);
                    self.pointbuf.clear();
                }

                if color {
                    self.colorindex += 1;
                    if self.colorindex == self.colors.len() {
                        self.colorindex = 0;
                    }
                    let c = self.colors[self.colorindex].pixel;
                    self.draw_gc.set_foreground(c);
                }
                // The swapped-in walker has not moved yet, so do not advance.
                continue;
            }

            // Move it a notch.
            match self.rand_2() {
                0 if x > self.scale => self.walkers[i].x -= self.scale,
                1 if x < self.width - 2 * self.scale => self.walkers[i].x += self.scale,
                2 if y > self.scale => self.walkers[i].y -= self.scale,
                3 if y < self.height - 2 * self.scale => self.walkers[i].y += self.scale,
                _ => {}
            }
            i += 1;
        }
        self.walkers.is_empty()
    }
}

impl Screenhack for Coral {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.eraser.is_some() || self.done {
            self.done = false;
            self.eraser = erase_window(d, self.eraser.take());
            return self.delay2.max(0) as u32;
        }

        if self.reset {
            self.init_coral(d);
        }
        let finished = self.step(d);
        self.reset = finished;
        self.done = finished;

        if self.reset {
            (self.delay.max(0) as u32).saturating_mul(1_000_000)
        } else {
            self.delay2.max(0) as u32
        }
    }

    fn reshape(&mut self, d: &mut Dpy, _width: i32, _height: i32) {
        self.init_coral(d);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.reset = true;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background:	black",
    ".foreground:	white",
    "*fpsSolid:	true",
    "*density:	25",
    // Too many for 640x480, too few for 1280x1024.
    "*seeds:	20",
    "*delay:	5",
    "*delay2:	20000",
    "*eraseSeconds:	1",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay2", "Frame rate", 1.0, 500_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("delay", "Linger", 1.0, 60.0, 1.0, 0, "5"),
    Opt::slider("density", "Density", 1.0, 90.0, 1.0, 0, "25"),
    Opt::slider("seeds", "Seeds", 1.0, 100.0, 1.0, 0, "20"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "coral",
    label: "Coral",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Frederick Roeber",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=3WTSvzJcQhw"),
        blurb: "A coral grown by diffusion-limited aggregation.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
