//! Port of `hacks/mountain.c`.
//!
//! ```text
//! Copyright (c) 1995 by Pascal Pensa <pensa@aurora.unice.fr>
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
//! 10-May-1997: Compatible with xscreensaver
//! 1995: Written
//! ```
//!
//! Papo's mountain range: scatter a few peaks across a 50x50 grid, smooth it by
//! averaging each cell with its neighbours, roughen it again with a little
//! noise, then draw the whole thing one quad at a time in isometric projection.
//! Each cell's colour comes from its own height, so the range shades naturally.
//! One frame draws one quad, so the landscape builds up in front of you.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

/// World size, x by y.
const WORLD_WIDTH: usize = 50;

/// `RANGE_RAND(min, max)`.
fn range_rand(min: i32, max: i32) -> i32 {
    min + nrand(max - min)
}

struct Mountain {
    mi: ModeInfo,
    /// True on a window too small for the outlines to read.
    pixelmode: bool,
    x: usize,
    y: usize,
    offset: usize,
    /// 0 draws, 1 waits, 2 starts a new range.
    stage: u8,
    h: Vec<Vec<i32>>,
    time: i32,
    wireframe: bool,
    /// One range in ten is drawn with a random mix of outlines and fills.
    joke: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let mut st = Mountain {
        mi,
        pixelmode: false,
        x: 0,
        y: 0,
        offset: 0,
        stage: 0,
        h: vec![vec![0; WORLD_WIDTH]; WORLD_WIDTH],
        time: 0,
        wireframe: false,
        joke: false,
    };
    st.restart(d);
    Box::new(st)
}

impl Mountain {
    /// Maximum peak height, scaled to the window.
    fn max_height(&self) -> i32 {
        3 * (self.mi.width + self.mi.height)
    }

    /// Average a cell with its eight neighbours, which is what turns the
    /// scattered peaks into a range.
    fn spread(h: &mut [Vec<i32>], x: usize, y: usize) {
        // Each neighbour is written once from its own old value plus this
        // cell's, so the visit order does not matter.
        let v = h[x][y];
        let (x0, x1) = (x.saturating_sub(1), (x + 1).min(WORLD_WIDTH - 1));
        let (y0, y1) = (y.saturating_sub(1), (y + 1).min(WORLD_WIDTH - 1));
        for column in &mut h[x0..=x1] {
            for cell in &mut column[y0..=y1] {
                *cell = (*cell + v) / 2;
            }
        }
    }

    fn restart(&mut self, d: &mut Dpy) {
        self.mi.width = d.width();
        self.mi.height = d.height();
        self.pixelmode = self.mi.width + self.mi.height < 200;
        self.stage = 0;
        self.time = 0;
        self.x = 0;
        self.y = 0;

        // Upstream runs fullrandom under xscreensaver, so both of these are
        // rolled rather than read from a resource.
        self.joke = nrand(10) == 0;
        self.wireframe = lrand() & 1 == 1;

        d.clear_window();

        for row in self.h.iter_mut() {
            row.fill(0);
        }

        let mut j = self.mi.count;
        if j < 0 {
            j = nrand(-j) + 1;
        }
        let max = self.max_height().max(1);
        for _ in 0..j {
            let x = range_rand(1, WORLD_WIDTH as i32 - 1) as usize;
            let y = range_rand(1, WORLD_WIDTH as i32 - 1) as usize;
            self.h[x][y] = nrand(max);
        }

        for y in 0..WORLD_WIDTH {
            for x in 0..WORLD_WIDTH {
                Self::spread(&mut self.h, x, y);
            }
        }

        for y in 0..WORLD_WIDTH {
            for x in 0..WORLD_WIDTH {
                self.h[x][y] = self.h[x][y] + nrand(10) - 5;
                if self.h[x][y] < 10 {
                    self.h[x][y] = 0;
                }
            }
        }

        self.offset = if self.mi.npixels() > 2 {
            nrand(self.mi.npixels()) as usize
        } else {
            0
        };
    }

    fn draw_a_mountain(&mut self, d: &mut Dpy) {
        let (w, h) = (self.mi.width, self.mi.height);
        let (x, y) = (self.x, self.y);

        let mut c = 0usize;
        if self.mi.npixels() > 2 {
            let avg =
                (self.h[x][y] + self.h[x + 1][y] + self.h[x][y + 1] + self.h[x + 1][y + 1]) / 4;
            c = ((avg / 10) as usize + self.offset) % self.mi.npixels() as usize;
        }

        // Isometric projection: x shears left as y increases, and the height
        // lifts the point straight up.
        let cell = |gx: usize, gy: usize| -> (i32, i32) {
            (
                gx as i32 * (2 * w) / (3 * WORLD_WIDTH as i32),
                gy as i32 * (2 * h) / (3 * WORLD_WIDTH as i32),
            )
        };
        let (x2, y2) = cell(x, y);
        let (x3, y3) = cell(x + 1, y);
        let (_, y4) = cell(x + 1, y + 1);
        let (_, y5) = cell(x, y + 1);

        let p = [
            XPoint {
                x: (x2 - y2 / 2) + (w / 4),
                y: (y2 - self.h[x][y]) + h / 4,
            },
            XPoint {
                x: (x3 - y3 / 2) + (w / 4),
                y: (y3 - self.h[x + 1][y]) + h / 4,
            },
            XPoint {
                x: (x3 - y4 / 2) + (w / 4),
                y: (y4 - self.h[x + 1][y + 1]) + h / 4,
            },
            XPoint {
                x: (x2 - y5 / 2) + (w / 4),
                y: (y5 - self.h[x][y + 1]) + h / 4,
            },
        ];
        // The fifth point closes the outline; the fill uses only the first four.
        let outline = [p[0], p[1], p[2], p[3], p[0]];

        let color = if self.mi.npixels() > 2 {
            self.mi.pixel(c)
        } else {
            self.mi.white
        };
        self.mi.gc.set_foreground(color);

        let filled = if self.joke {
            lrand() & 1 == 0
        } else {
            !self.wireframe
        };

        if filled {
            d.win().fill_polygon(&self.mi.gc, &p);
            if !self.pixelmode {
                let black = self.mi.black;
                self.mi.gc.set_foreground(black);
                d.win().draw_lines(&self.mi.gc, &outline);
            }
        } else {
            d.win().draw_lines(&self.mi.gc, &outline);
        }

        self.x += 1;
        if self.x == WORLD_WIDTH - 1 {
            self.y += 1;
            self.x = 0;
        }
        if self.y == WORLD_WIDTH - 1 {
            self.stage += 1;
        }
    }
}

impl Screenhack for Mountain {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        match self.stage {
            0 => self.draw_a_mountain(d),
            1 => {
                self.time += 1;
                if self.time > self.mi.cycles {
                    self.stage += 1;
                }
            }
            _ => self.restart(d),
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.restart(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 20000",
    "*count: 30",
    "*cycles: 4000",
    "*ncolors: 64",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("count", "Peaks", 1.0, 100.0, 1.0, 0, "30"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "mountain",
    label: "Mountain",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Pascal Pensa",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=knqnPcZGqkA"),
        blurb: "Papo's mountain range, built one quad at a time.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
