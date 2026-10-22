//! Port of `hacks/hexadrop.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1999-2019 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Draws a grid of hexagons or other shapes and drops them out.
//! Created 8-Jul-2013.
//! ```
//!
//! A tiling of triangles, squares, hexagons or octagons, where each tile holds
//! two colours and the inner one shrinks away to nothing before flipping to the
//! outer. Cell zero picks the next colour at random and every other cell copies
//! whatever cell zero has, so a single hue washes across the grid at whatever
//! rate each tile happens to be falling.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{make_random_colormap, make_smooth_colormap};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XColor, XEvent,
    XPoint, frand, random, screenhack_event_helper,
};

/// Upstream works on a ten-times grid to keep corners from rounding badly.
const SCALE: f64 = 10.0;

/// The shapes the random pick draws from, weighted.
const SIDE_CHOICES: [i32; 11] = [3, 3, 3, 4, 6, 6, 6, 6, 8, 8, 8];

#[derive(Clone, Copy, Default)]
struct Cell {
    sides: i32,
    cx: i32,
    cy: i32,
    th: f64,
    radius: f64,
    /// How far the inner shape has left to shrink.
    i: f64,
    speed: f64,
    colors: [usize; 2],
    initted: bool,
}

struct Hexadrop {
    gc: Gc,
    delay: u32,
    speed: f64,
    sides: i32,
    grid_size: i32,
    lockstep: bool,
    uniform: bool,
    ncolors: usize,
    colors: Vec<XColor>,
    cells: Vec<Cell>,
    gw: i32,
    gh: i32,
    width: i32,
    height: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut st = Hexadrop {
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        delay: 0,
        speed: 0.0,
        sides: 6,
        grid_size: 15,
        lockstep: false,
        uniform: false,
        ncolors: 2,
        colors: Vec::new(),
        cells: Vec::new(),
        gw: 0,
        gh: 0,
        width: d.width(),
        height: d.height(),
    };
    st.setup(d, true);
    Box::new(st)
}

impl Hexadrop {
    /// `hexadrop_init_1`: everything but the cells' own state, which
    /// [`Self::make_cells`] keeps across a reshape.
    fn setup(&mut self, d: &mut Dpy, fresh: bool) {
        self.delay = d.res.int("delay").max(0) as u32;
        self.ncolors = d.res.int("ncolors").max(2) as usize;
        self.speed = d.res.float("speed").max(0.0);
        self.grid_size = d.res.int("size");
        self.width = d.width();
        self.height = d.height();

        self.colors = if self.ncolors < 10 {
            make_random_colormap(self.ncolors, false)
        } else {
            make_smooth_colormap(self.ncolors)
        };
        // Upstream also paints the window background with the first colour;
        // the grid covers every pixel, so it would never show.

        // Both knobs default to "Maybe". When both are being rolled, upstream
        // refuses to turn on two at once.
        let s1 = d.res.string("uniform").to_ascii_lowercase();
        let s2 = d.res.string("lockstep").to_ascii_lowercase();
        let maybe1 = s1.is_empty() || s1 == "maybe";
        let maybe2 = s2.is_empty() || s2 == "maybe";
        if maybe1 && maybe2 {
            self.uniform = random() & 1 == 1;
            self.lockstep = if self.uniform {
                false
            } else {
                random() & 1 == 1
            };
        } else {
            self.uniform = if maybe1 {
                random() & 1 == 1
            } else {
                d.res.bool("uniform")
            };
            self.lockstep = if maybe2 {
                random() & 1 == 1
            } else {
                d.res.bool("lockstep")
            };
        }

        self.sides = d.res.int("sides");
        if !matches!(self.sides, 0 | 3 | 4 | 6 | 8) {
            self.sides = 0;
        }
        if self.sides == 0 {
            self.sides = SIDE_CHOICES[(random() % SIDE_CHOICES.len() as u32) as usize];
        }

        if fresh {
            self.cells = Vec::new();
            self.gw = 0;
            self.gh = 0;
        }
        self.make_cells();

        let c = self.colors[0].pixel;
        self.gc.set_foreground(c);
    }

    fn make_cells(&mut self) {
        let grid_size = self.grid_size.max(5);
        // A window smaller than one tile would divide by zero.
        let mut size = (self.width.max(self.height) / grid_size).max(1);

        let mut gw = self.width / size;
        let mut gh = self.height / size;
        let r: i32;
        let th: f64;

        match self.sides {
            8 => {
                r = (size as f64 * 0.75) as i32;
                th = std::f64::consts::PI / 8.0;
                gw = (gw as f64 * 1.25) as i32;
                gh = (gh as f64 * 1.25) as i32;
            }
            6 => {
                r = (size as f64 / 3.0f64.sqrt()) as i32;
                th = std::f64::consts::PI / 6.0;
                gh = (gh as f64 * 1.2) as i32;
            }
            3 => {
                size *= 2;
                r = (size as f64 / 3.0f64.sqrt()) as i32;
                th = std::f64::consts::PI / 3.0 / 2.0;
            }
            _ => {
                size = (size / 2).max(1);
                r = (size as f64 * 2.0f64.sqrt()) as i32;
                th = std::f64::consts::PI / 4.0;
            }
        }

        // Leave a few extra columns off screen just in case.
        gw += 3;
        gh += 3;

        let ncells = (gw * gh).max(0) as usize;
        let mut cells2 = vec![Cell::default(); ncells];

        // Keep whatever the old grid and the new one have in common, so a
        // resize does not restart every tile.
        for y in 0..self.gh.min(gh) {
            for x in 0..self.gw.min(gw) {
                cells2[(y * gw + x) as usize] = self.cells[(y * self.gw + x) as usize];
            }
        }
        self.cells = cells2;
        self.gw = gw;
        self.gh = gh;

        let sizef = size as f64;
        let mut i = 0usize;
        for y in 0..gh {
            for x in 0..gw {
                let c = &mut self.cells[i];
                c.sides = self.sides;
                c.radius = SCALE * r as f64;
                c.th = th;

                match self.sides {
                    8 => {
                        // Every other column is a small square, which is what
                        // fills the gaps an octagon tiling leaves.
                        if x & 1 == 1 {
                            c.cx = (SCALE * x as f64 * sizef) as i32;
                            c.radius /= 2.0;
                            c.th = std::f64::consts::FRAC_PI_4;
                            c.sides = 4;
                            c.radius *= 1.1;
                        } else {
                            c.cx = (SCALE * x as f64 * sizef) as i32;
                            c.radius *= 1.02;
                            c.radius -= 1.0;
                        }
                        if y & 1 == 1 {
                            c.cx -= (SCALE * sizef) as i32;
                        }
                        c.cy = (SCALE * y as f64 * sizef) as i32;
                    }
                    6 => {
                        c.cx = (SCALE * x as f64 * sizef) as i32;
                        c.cy = (SCALE * y as f64 * sizef * 3.0f64.sqrt() / 2.0) as i32;
                        if y & 1 == 1 {
                            c.cx -= (SCALE * sizef * 0.5) as i32;
                        }
                    }
                    3 => {
                        c.cx = (SCALE * x as f64 * sizef * 0.5) as i32;
                        c.cy = (SCALE * y as f64 * sizef * 3.0f64.sqrt() / 2.0) as i32;
                        if (x & 1) ^ (y & 1) == 1 {
                            c.th = th + std::f64::consts::PI;
                            c.cy -= (SCALE * r as f64 * 0.5) as i32;
                        }
                    }
                    _ => {
                        c.cx = (SCALE * x as f64 * sizef * 2.0) as i32;
                        c.cy = (SCALE * y as f64 * sizef * 2.0) as i32;
                    }
                }

                if !c.initted {
                    c.speed = self.speed * if self.uniform { 1.0 } else { 0.1 + frand(0.9) };
                    c.i = if self.lockstep {
                        0.0
                    } else {
                        (random() % r.max(1) as u32) as f64
                    };
                    c.colors[0] = if self.lockstep {
                        0
                    } else {
                        (random() % self.ncolors as u32) as usize
                    };
                    c.colors[1] = 0;
                    c.initted = true;
                }

                // Avoid single-pixel erase rounding errors.
                c.radius += SCALE;

                if c.i > c.radius {
                    c.i = c.radius;
                }
                c.colors[0] = c.colors[0].min(self.ncolors - 1);
                c.colors[1] = c.colors[1].min(self.ncolors - 1);

                i += 1;
            }
        }
    }

    fn draw_cell(&mut self, d: &mut Dpy, at: usize) {
        let c = self.cells[at];
        let mut points = [XPoint::default(); 8];
        for j in 0..=1usize {
            let r = if j == 0 { c.radius } else { c.i };
            let n = c.sides.clamp(3, 8) as usize;
            for (i, p) in points.iter_mut().enumerate().take(n) {
                let th = i as f64 * std::f64::consts::PI * 2.0 / c.sides as f64;
                p.x = ((c.cx as f64 + r * (th + c.th).cos() + 0.5) / SCALE) as i32;
                p.y = ((c.cy as f64 + r * (th + c.th).sin() + 0.5) / SCALE) as i32;
            }
            let color = self.colors[c.colors[j]].pixel;
            self.gc.set_foreground(color);
            d.win().fill_polygon(&self.gc, &points[..n]);
        }

        // Cell zero leads and everyone else follows it a beat later.
        let lead = self.cells[0].colors[0];
        let next = if at == 0 { None } else { Some(lead) };
        let ncolors = self.ncolors;

        let c = &mut self.cells[at];
        c.i -= SCALE * c.speed;
        if c.i < 0.0 {
            c.i = c.radius;
            c.colors[1] = c.colors[0];
            c.colors[0] = match next {
                Some(lead) => lead,
                None => (random() % ncolors as u32) as usize,
            };
        }
    }
}

impl Screenhack for Hexadrop {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for at in 0..self.cells.len() {
            self.draw_cell(d, at);
        }
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.make_cells();
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            if !random().is_multiple_of(5) {
                // Change everything.
                self.setup(d, true);
            } else {
                // Change colours only: keep the geometry that is already on
                // screen and re-roll everything around it.
                let cells = std::mem::take(&mut self.cells);
                let sides = self.sides;
                self.setup(d, true);
                self.cells = cells;
                for c in self.cells.iter_mut() {
                    c.initted = false;
                    c.colors[0] = c.colors[0].min(self.ncolors - 1);
                    c.colors[1] = c.colors[1].min(self.ncolors - 1);
                }
                self.sides = sides;
            }
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*delay: 30000",
    "*sides: 0",
    "*size: 15",
    "*speed: 1.0",
    "*ncolors: 128",
    "*uniform: Maybe",
    "*lockstep: Maybe",
];

const SIDES: &[SelectItem] = &[
    SelectItem {
        value: "0",
        label: "Random shape",
    },
    SelectItem {
        value: "3",
        label: "Triangles",
    },
    SelectItem {
        value: "4",
        label: "Squares",
    },
    SelectItem {
        value: "6",
        label: "Hexagons",
    },
    SelectItem {
        value: "8",
        label: "Octagons",
    },
];

const UNIFORM: &[SelectItem] = &[
    SelectItem {
        value: "Maybe",
        label: "Random speed",
    },
    SelectItem {
        value: "True",
        label: "Uniform speed",
    },
    SelectItem {
        value: "False",
        label: "Non-uniform speed",
    },
];

const LOCKSTEP: &[SelectItem] = &[
    SelectItem {
        value: "Maybe",
        label: "Random sync",
    },
    SelectItem {
        value: "True",
        label: "Synchronized",
    },
    SelectItem {
        value: "False",
        label: "Non-synchronized",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 50000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 4.0, 0.1, 1, "1.0"),
    Opt::slider("size", "Tile size", 5.0, 50.0, 1.0, 0, "15").inverted(),
    Opt::select("sides", "Shape", SIDES, "0"),
    Opt::select("uniform", "Speed spread", UNIFORM, "Maybe"),
    Opt::select("lockstep", "Sync", LOCKSTEP, "Maybe"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hexadrop",
    label: "Hexadrop",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2013",
        video: Some("https://www.youtube.com/watch?v=HMPVzQUGW-Q"),
        blurb: "A grid of hexagons or other shapes, with tiles dropping out.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
