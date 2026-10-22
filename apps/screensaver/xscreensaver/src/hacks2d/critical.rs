//! Port of `hacks/critical.c`.
//!
//! ```text
//! critical -- Self-organizing-criticality display hack for XScreenSaver
//! Copyright (C) 1998, 1999, 2000 Martin Pool <mbp@humbug.org.au>
//!
//! Permission to use, copy, modify, distribute, and sell this software
//! and its documentation for any purpose is hereby granted without
//! fee, provided that the above copyright notice appear in all copies
//! and that both that copyright notice and this permission notice
//! appear in supporting documentation.  No representations are made
//! about the suitability of this software for any purpose.  It is
//! provided "as is" without express or implied warranty.
//!
//! See `critical.man' for more information.
//!
//! Revision history:
//! 13 Nov 1998: Initial version, Martin Pool <mbp@humbug.org.au>
//! 08 Feb 2000: Change to keeping and erasing a trail, <mbp>
//!
//! It would be nice to draw curvy shapes rather than just straight
//! lines, but X11 doesn't have spline primitives (?) so we'd have to
//! do all the work ourselves
//! ```
//!
//! Self-organizing criticality, drawn as a walk. A grid holds a random value
//! per cell; each step finds the highest cell, joins it to the last one with a
//! line, and rerolls it and its eight neighbours. That is enough to make the
//! walk settle out of random squiggles into structure. A trail of past
//! positions is erased behind the pen, so the picture never fills in.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{make_random_colormap, make_smooth_colormap, make_uniform_colormap};
use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, XPoint, random,
};

/// The grid is a fixed width in cells: scaling it with the screen just makes
/// the hack boring on a large one.
const MODEL_W: i32 = 80;

/// How many lines share a colour.
const LINES_PER_COLOR: i32 = 10;

struct Critical {
    width: i32,
    height: i32,
    cells: Vec<u16>,
    /// Cell coordinates of the last `trail` steps.
    history: Vec<XPoint>,
    trail: usize,
    cell_size: i32,
    batchcount: i32,
    n_restart: i32,
    i_restart: i32,
    delay: u32,
    fgc: Gc,
    bgc: Gc,
    colors: Vec<XColor>,
    color_scheme: String,
    i_color: usize,
    pos: usize,
    wrapped: bool,
    i_batch: i32,
    eraser: Option<Eraser>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let cell_size = d.width() / MODEL_W;
    let model_h = if cell_size != 0 {
        d.height() / cell_size
    } else {
        1
    };

    let trail = d.res.int("trail").clamp(2, 1000) as usize;
    let batchcount = d.res.int("batchcount").max(5);

    let mut st = Critical {
        width: MODEL_W,
        height: model_h.max(1),
        cells: Vec::new(),
        history: vec![XPoint::default(); trail],
        trail,
        cell_size,
        batchcount,
        n_restart: d.res.int("restart").max(1),
        i_restart: 0,
        delay: d.res.int("delay").max(0) as u32,
        fgc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        bgc: Gc::new(d.res.pixel("background"), d.res.pixel("background")),
        colors: Vec::new(),
        color_scheme: d.res.string("colorscheme").to_string(),
        i_color: 0,
        pos: 1,
        wrapped: false,
        i_batch: batchcount,
        eraser: None,
    };
    st.cells = vec![0; (st.width * st.height) as usize];
    st.setup_colormap(d);
    st.model_initialize();
    st.history[0] = st.model_step();
    Box::new(st)
}

impl Critical {
    fn setup_colormap(&mut self, d: &Dpy) {
        let n = d.res.int("ncolors").max(3) as usize;
        self.colors = match self.color_scheme.as_str() {
            "random" => make_random_colormap(n, true),
            "smooth" => make_smooth_colormap(n),
            _ => make_uniform_colormap(n),
        };
    }

    fn model_initialize(&mut self) {
        for c in self.cells.iter_mut() {
            *c = random() as u16;
        }
    }

    /// Find the highest cell, then reroll it and its eight neighbours.
    fn model_step(&mut self) -> XPoint {
        let mut top_value = 0u16;
        let (mut top_x, mut top_y) = (0i32, 0i32);
        let mut i = 0usize;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.cells[i] >= top_value {
                    top_value = self.cells[i];
                    top_x = x;
                    top_y = y;
                }
                i += 1;
            }
        }

        for dy in -1..=1 {
            let yy = top_y + dy;
            if yy < 0 || yy >= self.height {
                continue;
            }
            for dx in -1..=1 {
                let xx = top_x + dx;
                if xx < 0 || xx >= self.width {
                    continue;
                }
                self.cells[(yy * self.width + xx) as usize] = random() as u16;
            }
        }

        XPoint { x: top_x, y: top_y }
    }

    fn draw_step(&self, d: &mut Dpy, fg: bool, pos: usize) {
        let half = self.cell_size / 2;
        let old_pos = (pos + self.trail - 1) % self.trail;
        let pos = pos % self.trail;
        let gc = if fg { &self.fgc } else { &self.bgc };
        d.win().draw_line(
            gc,
            self.history[pos].x * self.cell_size + half,
            self.history[pos].y * self.cell_size + half,
            self.history[old_pos].x * self.cell_size + half,
            self.history[old_pos].y * self.cell_size + half,
        );
    }
}

impl Screenhack for Critical {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.eraser.is_some() {
            self.eraser = erase_window(d, self.eraser.take());
            return self.delay;
        }

        if self.i_batch % LINES_PER_COLOR == 0 {
            self.i_color = (self.i_color + 1) % self.colors.len();
            let c = self.colors[self.i_color].pixel;
            self.fgc.set_foreground(c);
        }

        self.history[self.pos] = self.model_step();
        self.draw_step(d, true, self.pos);

        // The history is a ring buffer, but nothing is erased until it has
        // wrapped around once.
        self.pos += 1;
        if self.pos >= self.trail {
            self.pos -= self.trail;
            self.wrapped = true;
        }
        if self.wrapped {
            let pos = self.pos + 1;
            self.draw_step(d, false, pos);
        }

        self.i_batch -= 1;
        if self.i_batch >= 0 {
            return self.delay;
        }
        self.i_batch = self.batchcount;

        self.i_restart = (self.i_restart + 1) % self.n_restart;
        if self.i_restart == 0 {
            // Time to start a new simulation: this one has probably got to be
            // a bit boring.
            self.setup_colormap(d);
            self.eraser = erase_window(d, self.eraser.take());
            self.model_initialize();
            self.history[0] = self.model_step();
            self.pos = 1;
            self.wrapped = false;
            self.i_batch = self.batchcount;
        }

        self.delay
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*colorscheme: smooth",
    "*delay: 10000",
    "*ncolors: 64",
    "*restart: 8",
    "*batchcount: 1500",
    "*trail: 50",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("ncolors", "Number of colors", 3.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "critical",
    label: "Critical",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Martin Pool",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=HN2ykbM2cTk"),
        blurb: "A system of self-organizing lines.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
