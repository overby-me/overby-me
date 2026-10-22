//! Port of `hacks/cloudlife.c`.
//!
//! ```text
//! cloudlife by Don Marti <dmarti@zgp.org>
//!
//! Based on Conway's Life, but with one rule change to make it a better
//! screensaver: cells have a max age.
//!
//! When a cell exceeds the max age, it counts as 3 for populating the next
//! generation.  This makes long-lived formations explode instead of just
//! sitting there burning a hole in your screen.
//!
//! Cloudlife only draws one pixel of each cell per tick, whether the cell is
//! alive or dead.  So gliders look like little comets.
//!
//! 20 May 2003 -- now includes color cycling and a man page.
//!
//! Based on several examples from the hacks directory of:
//!
//! xscreensaver, Copyright (c) 1997, 1998, 2002 Jamie Zawinski <jwz@jwz.org>
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
//! Conway's Life with one change: a cell older than the maximum age counts for
//! three rather than one when its neighbours are counted, so a stable formation
//! eventually blows itself apart instead of sitting there. Only one pixel of
//! each cell is plotted per generation, at a random spot inside it, which is
//! what turns the cells into the soft clouds the name promises and gliders into
//! little comets.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, XEvent, XPoint, random,
    screenhack_event_helper,
};

struct Field {
    width: usize,
    height: usize,
    max_age: u32,
    cell_size: u32,
    cells: Vec<u8>,
    new_cells: Vec<u8>,
}

struct CloudLife {
    fgc: Gc,
    bgc: Gc,
    cycles: u32,
    /// Where in the colormap the foreground currently sits, and how long until
    /// it moves on.
    colorindex: usize,
    colortimer: i32,
    cycle_delay: u32,
    cycle_colors: i32,
    ncolors: usize,
    colors: Vec<XColor>,
    /// How likely a cell is to start alive, out of 256.
    density: i32,
    field: Field,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let cycle_colors = d.res.int("cycleColors");
    let ncolors = d.res.int("ncolors").max(1) as usize;
    let colors = if cycle_colors != 0 {
        make_smooth_colormap(ncolors)
    } else {
        Vec::new()
    };

    Box::new(CloudLife {
        fgc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        bgc: Gc::new(d.res.pixel("background"), d.res.pixel("background")),
        cycles: 0,
        colorindex: 0,
        colortimer: 0,
        cycle_delay: d.res.int("cycleDelay").max(0) as u32,
        cycle_colors,
        ncolors,
        colors,
        density: (d.res.int("initialDensity") % 100 * 256) / 100,
        field: Field::new(d),
    })
}

impl Field {
    fn new(d: &Dpy) -> Self {
        Self {
            width: 0,
            height: 0,
            // Upstream refuses to start when this will not fit in the byte it
            // is stored in.
            max_age: d.res.int("maxAge").clamp(0, 255) as u32,
            cell_size: d.res.int("cellSize").clamp(0, 20) as u32,
            cells: Vec::new(),
            new_cells: Vec::new(),
        }
    }

    fn cell(&self, x: usize, y: usize) -> u8 {
        self.cells[y * self.width + x]
    }

    fn resize(&mut self, w: usize, h: usize) {
        self.width = w;
        self.height = h;
        self.cells = vec![0; w * h];
        self.new_cells = vec![0; w * h];
    }

    /// How much a cell contributes to its neighbours' counts: nothing when
    /// dead, three once it is past its prime, one otherwise.
    fn value(c: u8, age: u32) -> u32 {
        if c == 0 {
            0
        } else if c as u32 > age {
            3
        } else {
            1
        }
    }

    fn is_alive(&self, x: usize, y: usize) -> u8 {
        let mut count = 0;
        for i in x - 1..=x + 1 {
            for j in y - 1..=y + 1 {
                if y != j || x != i {
                    count += Self::value(self.cell(i, j), self.max_age);
                }
            }
        }

        let p = self.cell(x, y);
        if p != 0 {
            if count == 2 || count == 3 {
                p.wrapping_add(1)
            } else {
                0
            }
        } else if count == 3 {
            1
        } else {
            0
        }
    }

    fn tick(&mut self) -> u32 {
        let mut count = 0;
        for x in 1..self.width - 1 {
            for y in 1..self.height - 1 {
                let v = self.is_alive(x, y);
                self.new_cells[y * self.width + x] = v;
                count += v as u32;
            }
        }
        self.cells.copy_from_slice(&self.new_cells);
        count
    }

    fn random_cell(p: i32) -> u8 {
        u8::from((random() & 0xff) < p as u32)
    }

    fn populate(&mut self, p: i32) {
        for c in self.cells.iter_mut() {
            *c = Self::random_cell(p);
        }
    }

    fn populate_edges(&mut self, p: i32) {
        let (w, h) = (self.width, self.height);
        for i in (0..w).rev() {
            self.cells[i] = Self::random_cell(p);
            self.cells[(h - 1) * w + i] = Self::random_cell(p);
        }
        for i in (0..h).rev() {
            self.cells[i * w + w - 1] = Self::random_cell(p);
            self.cells[i * w] = Self::random_cell(p);
        }
    }
}

impl CloudLife {
    /// Plot one pixel per cell, at a random spot inside it, live cells in the
    /// foreground and dead ones in the background.
    fn draw_field(&mut self, d: &mut Dpy) {
        let f = &self.field;
        if f.width < 3 || f.height < 3 {
            return;
        }
        let size = 1i32 << f.cell_size;
        let mask = (size - 1) as u32;

        let mut fg_points = Vec::with_capacity(f.width);
        let mut bg_points = Vec::with_capacity(f.width);

        for y in 1..f.height - 1 {
            fg_points.clear();
            bg_points.clear();

            for x in 1..f.width - 1 {
                let rx = random();
                let ry = (rx >> f.cell_size) & mask;
                let rx = rx & mask;

                let p = XPoint {
                    x: x as i32 * size - rx as i32 - 1,
                    y: y as i32 * size - ry as i32 - 1,
                };
                if f.cell(x, y) != 0 {
                    fg_points.push(p);
                } else {
                    bg_points.push(p);
                }
            }

            d.win().draw_points(&self.fgc, &fg_points);
            d.win().draw_points(&self.bgc, &bg_points);
        }
    }
}

impl Screenhack for CloudLife {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.cycle_colors != 0 && !self.colors.is_empty() {
            if self.colortimer == 0 {
                self.colortimer = self.cycle_colors;
                if self.colorindex == 0 {
                    self.colorindex = self.ncolors;
                }
                self.colorindex -= 1;
                let c = self.colors[self.colorindex.min(self.colors.len() - 1)].pixel;
                self.fgc.set_foreground(c);
            }
            self.colortimer -= 1;
        }

        let cell = 1usize << self.field.cell_size;
        let want_w = d.width() as usize / cell + 2;
        let want_h = d.height() as usize / cell + 2;
        if self.field.width != want_w || self.field.height != want_h {
            self.field.resize(want_w, want_h);
            let density = self.density;
            self.field.populate(density);
        }

        self.draw_field(d);

        if self.field.width >= 3 && self.field.height >= 3 {
            // A field that has burnt down to almost nothing gets reseeded.
            let live = self.field.tick();
            if live < (self.field.height + self.field.width) as u32 / 4 {
                let density = self.density;
                self.field.populate(density);
            }

            // Every so often, stir the edges so new gliders wander in.
            let period = (self.field.max_age / 2).max(1);
            if self.cycles.is_multiple_of(period) {
                let density = self.density;
                self.field.populate_edges(density);
                self.field.tick();
                self.field.populate_edges(0);
            }
        }

        self.cycles += 1;
        self.cycle_delay
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            d.clear_window();
            self.cycles = 0;
            self.field = Field::new(d);
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: blue",
    "*fpsSolid: true",
    "*cycleDelay: 25000",
    "*cycleColors: 2",
    "*ncolors: 64",
    "*maxAge: 64",
    "*initialDensity: 30",
    "*cellSize: 3",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "cycleDelay",
        "Frame rate",
        0.0,
        100_000.0,
        1000.0,
        0,
        "25000",
    )
    .inverted(),
    Opt::slider("maxAge", "Max age", 2.0, 255.0, 1.0, 0, "64"),
    Opt::slider("initialDensity", "Initial density", 1.0, 99.0, 1.0, 0, "30"),
    Opt::slider("cellSize", "Cell size", 1.0, 20.0, 1.0, 0, "3"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cloudlife",
    label: "Cloud Life",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Don Marti",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=TkVDO3nTTsE"),
        blurb: "Cloud-like formations from a variant of Conway's Life.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
