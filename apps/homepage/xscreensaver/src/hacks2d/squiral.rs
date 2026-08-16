//! Port of `hacks/squiral.c`.
//!
//! ```text
//! squiral, by "Jeff Epler" <jepler@inetnebr.com>, 18-mar-1999.
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
//! Square-spiral-producing automata. Each seed is a worm that would rather turn
//! one way than go straight and would rather go straight than turn the other
//! way, so left to itself it winds into a square spiral; when it runs into
//! something it goes around it instead, and when it is boxed in on all three it
//! gives up and starts again somewhere else. Once the screen is full enough the
//! whole thing is wiped from the edges inward and the worms start over.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_uniform_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, XEvent, frand,
    random_below, screenhack_event_helper,
};

const NCOLORS_MAX: i32 = 255;

/// `R(x)`.
fn r(x: i32) -> i32 {
    random_below(x)
}

/// `PROB(x)`.
fn prob(x: f64) -> bool {
    frand(1.0) < x
}

struct Worm {
    h: i32,
    v: i32,
    /// Handedness times four plus heading: upstream packs both into one field.
    s: i32,
    c: i32,
    /// How far the colour walks per step. Zero unless colour cycling is on.
    cc: i32,
}

struct Squiral {
    draw_gc: Gc,
    erase_gc: Gc,
    delay: u32,
    /// The grid, in cells rather than pixels.
    width: i32,
    height: i32,
    count: i32,
    cycle: bool,
    /// How full the screen gets before it is wiped.
    frac: f64,
    disorder: f64,
    handedness: f64,
    ncolors: i32,
    colors: Vec<XColor>,
    /// Cells filled since the last wipe.
    cov: i32,
    /// Heading vectors. The negative ones are written as a wrap so every index
    /// stays positive through the modulo.
    dirh: [i32; 4],
    dirv: [i32; 4],
    fill: Vec<u8>,
    worms: Vec<Worm>,
    /// How far the wipe has come in from each edge; equal to `height` when no
    /// wipe is running.
    inclear: i32,
    scale: i32,
    oscale: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    d.clear_window();

    let oscale = d.res.int("scale").max(1);
    let mut scale = oscale;
    if d.width() > 2560 || d.height() > 2560 {
        scale *= 3; // Retina displays
    }

    let fg = d.res.pixel("foreground");
    let (ncolors, colors) = if d.mono_p {
        (1, vec![XColor::from_pixel(fg)])
    } else {
        let mut n = d.res.int("ncolors");
        if !(0..=NCOLORS_MAX).contains(&n) {
            n = NCOLORS_MAX;
        }
        let colors = make_uniform_colormap(n.max(0) as usize);
        if colors.is_empty() {
            (1, vec![XColor::from_pixel(fg)])
        } else {
            (colors.len() as i32, colors)
        }
    };

    let mut frac = d.res.int("fill") as f64 * 0.01;
    frac = frac.clamp(0.01, 0.99);

    // A window narrower than one cell would leave no grid at all.
    let width = (d.width() / scale).max(1);
    let height = (d.height() / scale).max(1);

    let mut count = d.res.int("count");
    if count == 0 {
        count = width / 32;
    }
    count = count.clamp(1, 1000);

    let mut st = Squiral {
        draw_gc: Gc::new(fg, d.res.pixel("background")),
        erase_gc: Gc::new(d.res.pixel("background"), d.res.pixel("background")),
        delay: d.res.int("delay").max(0) as u32,
        width,
        height,
        count,
        cycle: d.res.bool("cycle"),
        frac,
        disorder: d.res.float("disorder"),
        handedness: d.res.float("handedness"),
        ncolors,
        colors,
        cov: 0,
        dirh: [0; 4],
        dirv: [0; 4],
        fill: Vec::new(),
        worms: Vec::new(),
        inclear: 0,
        scale,
        oscale,
    };
    st.init_1();
    Box::new(st)
}

impl Squiral {
    /// `squiral_init_1`: a fresh grid and a fresh set of worms.
    fn init_1(&mut self) {
        self.fill = vec![0; (self.width * self.height) as usize];

        self.dirh = [0, 1, 0, self.width - 1];
        self.dirv = [self.height - 1, 0, 1, 0];

        self.worms = (0..self.count)
            .map(|_| Worm {
                h: r(self.width),
                v: r(self.height),
                s: r(4) + 4 * prob(self.handedness) as i32,
                c: r(self.ncolors),
                cc: if self.cycle { r(3) + self.ncolors } else { 0 },
            })
            .collect();
    }

    /// `CLEAR1`: is the cell at this (unwrapped) position still empty?
    fn clear1(&self, x: i32, y: i32) -> bool {
        self.fill[((y % self.height) * self.width + x % self.width) as usize] == 0
    }

    /// `MOVE1`: claim a cell and paint it.
    fn move1(&mut self, d: &mut Dpy, x: i32, y: i32) {
        let (x, y) = (x % self.width, y % self.height);
        self.fill[(y * self.width + x) as usize] = 1;
        let scale = self.scale;
        d.win()
            .fill_rectangle(&self.draw_gc, x * scale, y * scale, scale, scale);
        self.cov += 1;
    }

    /// `CLEAR(d)`: are both cells of a step in this heading free?
    fn clear(&self, w: usize, dir: i32) -> bool {
        let (dh, dv) = (self.dirh[dir as usize], self.dirv[dir as usize]);
        let (h, v) = (self.worms[w].h, self.worms[w].v);
        self.clear1(h + dh, v + dv) && self.clear1(h + dh + dh, v + dv + dv)
    }

    /// `MOVE(d)`: take a step of two cells in this heading.
    fn step(&mut self, d: &mut Dpy, w: usize, dir: i32) {
        let (dh, dv) = (self.dirh[dir as usize], self.dirv[dir as usize]);
        let (h, v) = (self.worms[w].h, self.worms[w].v);
        let color = self.colors[self.worms[w].c as usize].pixel;
        self.draw_gc.set_foreground(color);
        self.move1(d, h + dh, v + dv);
        self.move1(d, h + dh + dh, v + dv + dv);
        self.worms[w].h = h + dh * 2;
        self.worms[w].v = v + dv * 2;
    }

    fn do_worm(&mut self, d: &mut Dpy, w: usize) {
        let mut kind = self.worms[w].s / 4;
        let mut dir = self.worms[w].s % 4;

        self.worms[w].c = (self.worms[w].c + self.worms[w].cc) % self.ncolors;

        if prob(self.disorder) {
            kind = prob(self.handedness) as i32;
        }

        // Turn one way if you can, else go straight, else turn the other way.
        let ccw = (dir + 3) % 4;
        let cw = (dir + 1) % 4;
        let tries = if kind == 0 {
            [ccw, dir, cw]
        } else {
            [cw, dir, ccw]
        };

        let mut moved = false;
        for cand in tries {
            if self.clear(w, cand) {
                self.step(d, w, cand);
                dir = cand;
                moved = true;
                break;
            }
        }

        if !moved {
            // Boxed in: give up and start somewhere else.
            self.worms[w].h = r(self.width);
            self.worms[w].v = r(self.height);
            self.worms[w].c = r(self.ncolors);
            kind = r(2);
            dir = r(4);
            if self.cycle {
                self.worms[w].cc = r(3) + self.ncolors;
            }
        }

        self.worms[w].s = kind * 4 + dir;
        self.worms[w].h %= self.width;
        self.worms[w].v %= self.height;
    }

    /// One band of the wipe: a row of background and the matching row of the
    /// grid, at whichever end of the screen still has one.
    fn wipe_row(&mut self, d: &mut Dpy, row: i32) {
        let scale = self.scale;
        let width = self.width;
        d.win()
            .fill_rectangle(&self.erase_gc, 0, row * scale, (width - 1) * scale, scale);
        if row >= 0 && row < self.height {
            let from = (row * width) as usize;
            self.fill[from..from + width as usize].fill(0);
        }
    }
}

impl Screenhack for Squiral {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.inclear < self.height {
            let (a, b) = (self.inclear, self.height - self.inclear - 1);
            self.wipe_row(d, a);
            self.wipe_row(d, b);
            self.inclear += 1;
            let (a, b) = (self.inclear, self.height - self.inclear - 1);
            self.wipe_row(d, a);
            self.wipe_row(d, b);
            self.inclear += 1;
            if self.inclear > self.height / 2 {
                self.inclear = self.height;
            }
        } else if self.cov as f64 > self.frac * self.width as f64 * self.height as f64 {
            self.inclear = 0;
            self.cov = 0;
        }

        for w in 0..self.worms.len() {
            self.do_worm(d, w);
        }
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.scale = self.oscale;
        if width > 2560 || height > 2560 {
            self.scale *= 3; // Retina displays
        }
        self.width = (width / self.scale).max(1);
        self.height = (height / self.scale).max(1);
        self.init_1();
        d.clear_window();
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.init_1();
            d.clear_window();
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*fill: 75",
    "*count: 0",
    "*ncolors: 100",
    "*delay: 10000",
    "*disorder: 0.005",
    "*cycle: False",
    "*handedness: 0.5",
    "*scale: 1",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("disorder", "Randomness", 0.0, 0.5, 0.005, 3, "0.005"),
    Opt::spin("count", "Seeds", 0.0, 200.0, "0"),
    Opt::slider("scale", "Scale", 1.0, 10.0, 1.0, 0, "1"),
    Opt::slider("handedness", "Handedness", 0.0, 1.0, 0.05, 2, "0.5"),
    Opt::slider("fill", "Density", 0.0, 100.0, 1.0, 0, "75"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "100"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "squiral",
    label: "Squiral",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jeff Epler",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=WPhqyM9Bb4o"),
        blurb: "Square-spiral-producing automata.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
