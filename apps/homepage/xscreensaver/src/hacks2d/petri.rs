//! Port of `hacks/petri.c`.
//!
//! ```text
//! petri, simulate mold in a petri dish. v2.7
//! by Dan Bornstein, danfuzz@milk.com
//! with help from Jamie Zawinski, jwz@jwz.org
//! Copyright (c) 1992-1999 Dan Bornstein.
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
//! Mold in a dish. A colony spreads one cell at a time, and a cell only
//! spreads once it has grown enough: orthogonally at one unit of growth,
//! diagonally at `diaglim` units, and then it dies. That single number is the
//! shape of a colony. One gives square colonies, two gives diamonds, and the
//! square root of two, which is the default, gives circles, because the extra
//! distance to a corner is exactly the extra growth a diagonal step costs.
//!
//! Where two colonies meet the boundary is not a line but a spiral, and
//! nothing in the program draws it. Each colony spreads at its own random
//! speed, so the faster one keeps overtaking the slower along a curve that
//! turns as the two fronts sweep past each other.
//!
//! A newly claimed cell is drawn bright and repainted dim when it dies, so the
//! live edge of every colony glows and the interior is flat. Every so often
//! the dish is poisoned: black death cells are seeded, spread much faster than
//! anything alive, and eat the lot.
//!
//! Only the cells that are currently growing are visited, so the work per
//! frame is the length of the colony fronts rather than the area of the dish.
//! Upstream threads a doubly linked list through the grid to do that; the same
//! list is here, as indices rather than pointers.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{XColor, make_random_colormap};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XEvent, random,
};

/// `RAND_FLOAT`.
fn rand_float() -> f32 {
    ((random() & 0xffff) as f32) / 65536.0
}

/// Not in the list. Upstream uses a null `prev` for this.
const NONE: usize = usize::MAX;

#[derive(Clone, Copy, Default)]
struct Cell {
    col: u8,
    isnext: bool,
    nextcol: u8,
    next: usize,
    prev: usize,
    speed: f32,
    growth: f32,
    nextspeed: f32,
}

/// What upstream's `sizeof(cell)` comes to on a 64-bit build, which is what
/// the memory throttle is measured against. Ours is a different size, but the
/// throttle exists to pick a cell size, and the cell size is the picture.
const UPSTREAM_CELL_BYTES: i64 = 40;

struct State {
    arr_width: i32,
    arr_height: i32,
    count: usize,
    /// The grid, followed by the two list sentinels.
    arr: Vec<Cell>,
    head: usize,
    tail: usize,
    blastcount: i32,
    /// Two per mold: the dim tone at `col` and the bright one at `col + count`.
    colors: Vec<Pixel>,
    gc: Gc,

    window_width: i32,
    window_height: i32,
    x_offset: i32,
    y_offset: i32,
    x_size: i32,
    y_size: i32,

    orthlim: f32,
    diaglim: f32,
    anychan: f32,
    minorchan: f32,
    instantdeathchan: f32,
    minlifespan: i32,
    maxlifespan: i32,
    minlifespeed: f32,
    maxlifespeed: f32,
    mindeathspeed: f32,
    maxdeathspeed: f32,
    delay: u32,
}

impl State {
    fn cell_x(&self, c: usize) -> i32 {
        if self.arr_width != 0 {
            (c % self.arr_width as usize) as i32
        } else {
            0
        }
    }

    fn cell_y(&self, c: usize) -> i32 {
        if self.arr_width != 0 {
            (c / self.arr_width as usize) as i32
        } else {
            0
        }
    }

    fn random_life_value(&self) -> i32 {
        (rand_float() * (self.maxlifespan - self.minlifespan) as f32) as i32 + self.minlifespan
    }

    fn drawblock(&mut self, d: &mut Dpy, x: i32, y: i32, c: u8) {
        let p = self.colors[(c as usize).min(self.colors.len() - 1)];
        self.gc.set_foreground(p);
        if self.x_size == 1 && self.y_size == 1 {
            d.win()
                .draw_point(&self.gc, x + self.x_offset, y + self.y_offset);
        } else {
            d.win().fill_rectangle(
                &self.gc,
                x * self.x_size + self.x_offset,
                y * self.y_size + self.y_offset,
                self.x_size,
                self.y_size,
            );
        }
    }

    fn setup_arr(&mut self, d: &mut Dpy) {
        let bg = self.colors[0];
        self.gc.set_foreground(bg);
        let (w, h) = (self.window_width, self.window_height);
        d.win().fill_rectangle(&self.gc, 0, 0, w, h);

        self.arr_width = self.arr_width.max(1);
        self.arr_height = self.arr_height.max(1);

        let n = (self.arr_width * self.arr_height) as usize;
        self.head = n;
        self.tail = n + 1;
        self.arr = vec![
            Cell {
                prev: NONE,
                next: NONE,
                ..Cell::default()
            };
            n + 2
        ];

        let (head, tail) = (self.head, self.tail);
        self.arr[head].next = tail;
        self.arr[head].prev = head;
        self.arr[tail].next = tail;
        self.arr[tail].prev = head;

        self.blastcount = self.random_life_value();
    }

    fn newcell(&mut self, c: usize, col: u8, sp: f32) {
        if self.arr[c].col == col {
            return;
        }

        self.arr[c].nextcol = col;
        self.arr[c].nextspeed = sp;
        self.arr[c].isnext = true;

        if self.arr[c].prev == NONE {
            let head = self.head;
            let after = self.arr[head].next;
            self.arr[c].next = after;
            self.arr[c].prev = head;
            self.arr[head].next = c;
            self.arr[after].prev = c;
        }
    }

    /// Unlink a cell, but leave its forward pointer alone: `update` is walking
    /// the list and takes its next step through the cell it just killed.
    fn killcell(&mut self, d: &mut Dpy, c: usize) {
        let (p, nx) = (self.arr[c].prev, self.arr[c].next);
        self.arr[p].next = nx;
        self.arr[nx].prev = p;
        self.arr[c].prev = NONE;
        self.arr[c].speed = 0.0;
        let (x, y, col) = (self.cell_x(c), self.cell_y(c), self.arr[c].col);
        self.drawblock(d, x, y, col);
    }

    fn randblip(&mut self, d: &mut Dpy, doit: bool) {
        let mut b = false;
        let mut n;

        if !doit {
            let was = self.blastcount;
            self.blastcount -= 1;
            if was >= 0 && rand_float() > self.anychan {
                return;
            }
        }

        if self.blastcount < 0 {
            b = true;
            n = 2;
            self.blastcount = self.random_life_value();
            if rand_float() < self.instantdeathchan {
                // Clear everything every so often, to keep from getting into
                // a rut.
                self.setup_arr(d);
                b = false;
            }
        } else if rand_float() <= self.minorchan {
            n = 2;
        } else {
            n = (random() % 3) as i32 + 3;
        }

        while n > 0 {
            n -= 1;
            let x = if self.arr_width != 0 {
                (random() % self.arr_width as u32) as i32
            } else {
                0
            };
            let y = if self.arr_height != 0 {
                (random() % self.arr_height as u32) as i32
            } else {
                0
            };
            let (c, s) = if b {
                (
                    0u8,
                    rand_float() * (self.maxdeathspeed - self.mindeathspeed) + self.mindeathspeed,
                )
            } else {
                let c = if self.count > 1 {
                    (random() % (self.count as u32 - 1)) as u8 + 1
                } else {
                    1
                };
                (
                    c,
                    rand_float() * (self.maxlifespeed - self.minlifespeed) + self.minlifespeed,
                )
            };
            let i = (y * self.arr_width + x) as usize;
            self.newcell(i, c, s);
        }
    }

    fn update(&mut self, d: &mut Dpy) {
        const ALL_COORDS: [(i32, i32); 8] = [
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
        ];

        // New cells go in at the head, behind where this walk has already
        // been, so a cell born this frame is not grown this frame. Killing a
        // cell leaves its forward pointer alone, which is what lets the walk
        // step through it.
        let mut a = self.arr[self.head].next;
        while a != self.tail {
            if self.arr[a].speed != 0.0 {
                self.arr[a].growth += self.arr[a].speed;
                let growth = self.arr[a].growth;

                // Orthogonal neighbours first, then the diagonals once the
                // cell has grown far enough to reach a corner.
                let coords: &[(i32, i32)] = if growth >= self.diaglim {
                    &ALL_COORDS
                } else if growth >= self.orthlim {
                    &ALL_COORDS[4..]
                } else {
                    &[]
                };

                if !coords.is_empty() {
                    let (col, speed) = (self.arr[a].col, self.arr[a].speed);
                    let (ax, ay) = (self.cell_x(a), self.cell_y(a));
                    for &(dx, dy) in coords {
                        let mut x = ax + dx;
                        let mut y = ay + dy;
                        if x < 0 {
                            x = self.arr_width - 1;
                        } else if x >= self.arr_width {
                            x = 0;
                        }
                        if y < 0 {
                            y = self.arr_height - 1;
                        } else if y >= self.arr_height {
                            y = 0;
                        }
                        let i = (y * self.arr_width + x) as usize;
                        self.newcell(i, col, speed);
                    }

                    if growth >= self.diaglim {
                        self.killcell(d, a);
                    }
                }
            }
            a = self.arr[a].next;
        }

        let empty = self.arr[self.head].next == self.tail;
        self.randblip(d, empty);

        let mut a = self.arr[self.head].next;
        while a != self.tail {
            if self.arr[a].isnext {
                self.arr[a].isnext = false;
                self.arr[a].speed = self.arr[a].nextspeed;
                self.arr[a].growth = 0.0;
                self.arr[a].col = self.arr[a].nextcol;
                let (x, y, col) = (self.cell_x(a), self.cell_y(a), self.arr[a].col);
                let bright = col as usize + self.count;
                self.drawblock(d, x, y, bright as u8);
            }
            a = self.arr[a].next;
        }
    }
}

/// The artist's original choices: primary and secondary colours at half
/// intensity for the settled cells and three quarters for the fresh ones.
fn original_colors(count: usize, bg: Pixel, fg: Pixel) -> Vec<Pixel> {
    let mut colors = vec![0u32; count * 2];
    colors[0] = bg;
    colors[count] = fg;
    for n in 1..count {
        let dim = XColor::from_rgb16(
            ((n & 0x01) != 0) as u16 * 0x8000,
            ((n & 0x02) != 0) as u16 * 0x8000,
            ((n & 0x04) != 0) as u16 * 0x8000,
        );
        colors[n] = dim.pixel;
        colors[n + count] =
            XColor::from_rgb16(dim.red + 0x4000, dim.green + 0x4000, dim.blue + 0x4000).pixel;
    }
    colors
}

/// A random palette, with each mold's settled tone at half the intensity of
/// its growing edge.
fn random_colors(count: usize, bg: Pixel, fg: Pixel) -> Vec<Pixel> {
    let bright = make_random_colormap(count.saturating_sub(1).max(1), true);
    let mut colors = vec![0u32; count * 2];
    colors[0] = bg;
    colors[count] = fg;
    for n in 1..count {
        let c = bright[(n - 1) % bright.len()];
        colors[n + count] = c.pixel;
        colors[n] = XColor::from_rgb16(c.red / 2, c.green / 2, c.blue / 2).pixel;
    }
    colors
}

/// `memThrottle`: a byte count, optionally with a K or M suffix.
fn parse_throttle(s: &str) -> i64 {
    let s = s.trim();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: i64 = digits.parse().unwrap_or(0);
    match s[digits.len()..].trim_start().chars().next() {
        Some('M') | Some('m') => n * (1 << 20),
        Some('K') | Some('k') => n * (1 << 10),
        _ => n,
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (window_width, window_height) = (d.width(), d.height());
    let mut cell_size = d.res.int("size").max(1);
    if window_width > 2560 || window_height > 2560 {
        cell_size *= 2; // Retina displays.
    }

    // The colour index lives in a byte, so it cannot be large.
    let originalcolors = d.res.bool("originalcolors");
    let mut count = d.res.int("count").clamp(2, 128) as usize;
    if originalcolors && count > 8 {
        count = 8;
    }

    let orthlim = 1.0f32;
    let diaglim = (d.res.float("diaglim") as f32).clamp(1.0, 2.0) * orthlim;
    let minlifespan = d.res.int("minlifespan").max(1);
    let minlifespeed = (d.res.float("minlifespeed") as f32).clamp(0.0, 1.0);
    let mindeathspeed = (d.res.float("mindeathspeed") as f32).clamp(0.0, 1.0);

    // Don't malloc more than the throttle allows; scale the cells up instead.
    let mem_throttle = parse_throttle(d.res.string("memThrottle"));
    let mut arr_width = window_width / cell_size;
    let mut arr_height = window_height / cell_size;
    if mem_throttle > 0 {
        while cell_size < window_width / 10
            && cell_size < window_height / 10
            && UPSTREAM_CELL_BYTES * arr_width as i64 * arr_height as i64 > mem_throttle
        {
            cell_size += 1;
            arr_width = window_width / cell_size;
            arr_height = window_height / cell_size;
        }
    }

    let mut x_size = if arr_width != 0 {
        window_width / arr_width
    } else {
        0
    };
    let mut y_size = if arr_height != 0 {
        window_height / arr_height
    } else {
        0
    };
    if x_size > y_size {
        x_size = y_size;
    } else {
        y_size = x_size;
    }

    let bg = d.res.pixel("background");
    let fg = d.res.pixel("foreground");
    let colors = if originalcolors {
        original_colors(count, bg, fg)
    } else {
        random_colors(count, bg, fg)
    };

    let mut st = State {
        arr_width,
        arr_height,
        count,
        arr: Vec::new(),
        head: 0,
        tail: 0,
        blastcount: 0,
        colors,
        gc: Gc::new(fg, bg),
        window_width,
        window_height,
        x_offset: (window_width - (arr_width * x_size)) / 2,
        y_offset: (window_height - (arr_height * y_size)) / 2,
        x_size,
        y_size,
        orthlim,
        diaglim,
        anychan: (d.res.float("anychan") as f32).clamp(0.0, 1.0),
        minorchan: (d.res.float("minorchan") as f32).clamp(0.0, 1.0),
        instantdeathchan: (d.res.float("instantdeathchan") as f32).clamp(0.0, 1.0),
        minlifespan,
        maxlifespan: d.res.int("maxlifespan").max(minlifespan),
        // The speeds are fractions of the fastest a cell could possibly grow,
        // which is one colony radius per `diaglim` of growth.
        minlifespeed: minlifespeed * diaglim,
        maxlifespeed: (d.res.float("maxlifespeed") as f32).clamp(minlifespeed, 1.0) * diaglim,
        mindeathspeed: mindeathspeed * diaglim,
        maxdeathspeed: (d.res.float("maxdeathspeed") as f32).clamp(mindeathspeed, 1.0) * diaglim,
        delay: d.res.int("delay").max(0) as u32,
    };
    st.setup_arr(d);
    st.randblip(d, true);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.update(d);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, _width: i32, _height: i32) {
        // Upstream has no reshape either: the dish keeps the size it was
        // poured at.
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*delay: 10000",
    "*count: 20",
    "*size: 2",
    "*diaglim: 1.414",
    "*anychan: 0.0015",
    "*minorchan: 0.5",
    "*instantdeathchan: 0.2",
    "*minlifespan: 500",
    "*maxlifespan: 1500",
    "*minlifespeed: 0.04",
    "*maxlifespeed: 0.13",
    "*mindeathspeed: 0.42",
    "*maxdeathspeed: 0.46",
    "*originalcolors: false",
    // Don't malloc more than this much; scale the pixels up if necessary.
    "*memThrottle: 22M",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("diaglim", "Colony shape", 1.0, 2.0, 0.05, 3, "1.414").inverted(),
    Opt::slider("anychan", "Fertility", 0.0, 0.25, 0.005, 4, "0.0015"),
    Opt::slider("minorchan", "Offspring", 0.0, 1.0, 0.05, 2, "0.5"),
    Opt::slider("instantdeathchan", "Death comes", 0.0, 1.0, 0.05, 2, "0.2"),
    Opt::slider(
        "minlifespeed",
        "Minimum rate of growth",
        0.0,
        1.0,
        0.01,
        2,
        "0.04",
    ),
    Opt::slider(
        "maxlifespeed",
        "Maximum rate of growth",
        0.0,
        1.0,
        0.01,
        2,
        "0.13",
    ),
    Opt::slider(
        "mindeathspeed",
        "Minimum rate of death",
        0.0,
        1.0,
        0.01,
        2,
        "0.42",
    ),
    Opt::slider(
        "maxdeathspeed",
        "Maximum rate of death",
        0.0,
        1.0,
        0.01,
        2,
        "0.46",
    ),
    Opt::slider(
        "minlifespan",
        "Minimum lifespan",
        0.0,
        3000.0,
        50.0,
        0,
        "500",
    ),
    Opt::slider(
        "maxlifespan",
        "Maximum lifespan",
        0.0,
        3000.0,
        50.0,
        0,
        "1500",
    ),
    Opt::spin("size", "Cell size", 0.0, 100.0, "2"),
    Opt::spin("count", "Mold varieties", 0.0, 20.0, "20"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "petri",
    label: "Petri",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Dan Bornstein",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=QkJ9cN0QQd8"),
        blurb: "Colonies of mold grow in a petri dish, leaving spiral interference in their wake.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
