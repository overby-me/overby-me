//! Port of `hacks/anemotaxis.c`.
//!
//! ```text
//! anemotaxis, Copyright (c) 2004 Eugene Balkovski
//!
//! Permission to use, copy, modify, distribute, and sell this software
//! and its documentation for any purpose is hereby granted without
//! fee, provided that the above copyright notice appear in all copies
//! and that both that copyright notice and this permission notice
//! appear in supporting documentation.  No representations are made
//! about the suitability of this software for any purpose. It is
//! provided "as is" without express or implied warranty.
//!
//! FILE            anemotaxis.c
//!
//! DESCRIPTION     Anemotaxis
//!
//!                 This code illustrates an optimal algorithm designed
//!                 for searching a source of particles on a plane.
//!                 The particles drift in one direction and walk randomly
//!                 in the other. The only information available to the
//!                 searcher is the presence of a particle at its location
//!                 and the local direction from where particle arrived.
//!                 The algorithm "explains" the behavior
//!                 of some animals and insects
//!                 who use olfactory and directional cues to find
//!                 sources of odor (mates, food, home etc) in
//!                 turbulent atmosphere (odor-modulated anemotaxis),
//!                 e.g. male moths locating females who release
//!                 pheromones to attract males. The search trajectories
//!                 resemble the trajectories that the animals follow.
//!
//! WRITTEN BY      Eugene Balkovski
//!
//! MODIFICATIONS   june 2004 started
//! ```
//!
//! A moth finding a female in the dark. Sources on the left emit particles
//! that drift right and stagger up or down a step at a time, and a searcher
//! knows two things only: whether a particle is where it is standing, and
//! which way that particle came from. From those two facts it does the
//! provably right thing.
//!
//! When it meets a particle, everything to the left of that point in a
//! widening cone could hold the source, so the searcher sweeps that cone: one
//! step up-left, then right along the diagonal, one step up-right, then left
//! along the other diagonal, back and forth, the sweep growing as the cone
//! does. The moment another particle turns up the cone is thrown away and a
//! new, narrower one starts from there. What comes out looks like a moth
//! casting about in the wind, because it is the same algorithm.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{WHITE, XColor, make_random_colormap};
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XEvent, random,
    random_below,
};

const MAX_DIST: i32 = 250;
const MIN_DIST: i32 = 10;
const MAX_INV_RATE: i32 = 5;

/// `RND`.
fn rnd(x: i32) -> i32 {
    random_below(x)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Point {
    x: i32,
    y: i32,
}

/// One lattice site of one source's plume: where the particle is, and which
/// way it stepped to get there. A velocity of two means there is no particle.
#[derive(Clone, Copy)]
struct Yv {
    y: i32,
    v: i32,
}

const NO_PARTICLE: i32 = 2;

struct Source {
    /// `yv[i]` is the particle at `(i + 1, yv[i].y)` relative to the source.
    yv: Vec<Yv>,
    /// Inverse rate of particle emission. Zero means the source has stopped
    /// emitting, though its old particles are still in flight.
    inv_rate: i32,
    r: Point,
    color: Pixel,
}

/// Where in the sweep of the current cone the searcher is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StateT {
    UpLeft,
    UpRight,
    Left,
    Right,
    Done,
}

struct Searcher {
    r: Point,
    /// The vertex of the most recent cone, which is the region where the
    /// source is located. We do exhaustive search in the cone until we
    /// encounter a new particle, which gives us a new cone.
    vertex: Point,
    state: StateT,
    /// Concentration at `r`.
    c: u8,
    /// Velocity at `r`, good only when `c` is not zero.
    v: i32,
    /// The trajectory, most recent first.
    hist: Vec<Point>,
    /// A small shift of the x-coordinate, to avoid painting over the same x.
    rs: i32,
    color: Pixel,
}

impl Searcher {
    fn write_hist(&mut self) {
        self.hist.push(self.r);
    }

    fn move_(&mut self) {
        if self.c == 1 {
            self.write_hist();
            self.r.x -= 1;
            self.r.y -= self.v;
            self.write_hist();
            self.state = if rnd(2) == 0 {
                StateT::UpLeft
            } else {
                StateT::UpRight
            };
            self.vertex = self.r;
            return;
        }

        match self.state {
            StateT::UpLeft => {
                self.r.x -= 1;
                self.r.y += 1;
                self.state = StateT::Right;
                self.write_hist();
            }
            StateT::Right => {
                self.r.y -= 1;
                if self.vertex.x - self.r.x == self.vertex.y - self.r.y {
                    self.write_hist();
                    self.state = StateT::UpRight;
                }
            }
            StateT::UpRight => {
                self.r.x -= 1;
                self.r.y -= 1;
                self.state = StateT::Left;
                self.write_hist();
            }
            StateT::Left => {
                self.r.y += 1;
                if self.vertex.x - self.r.x == self.r.y - self.vertex.y {
                    self.write_hist();
                    self.state = StateT::UpLeft;
                }
            }
            StateT::Done => {}
        }
    }
}

impl Source {
    fn evolve(&mut self) {
        // Propagate existing particles: each one takes a random step across
        // the wind as it drifts one place along it.
        for i in (1..self.yv.len()).rev() {
            if self.yv[i - 1].v == NO_PARTICLE {
                self.yv[i].v = NO_PARTICLE;
            } else {
                self.yv[i].v = rnd(3) - 1;
                self.yv[i].y = self.yv[i - 1].y + self.yv[i].v;
            }
        }

        if self.inv_rate > 0 && rnd(self.inv_rate) == 0 {
            // Emit a particle.
            let v = rnd(3) - 1;
            self.yv[0].y = v;
            self.yv[0].v = v;
        } else {
            self.yv[0].v = NO_PARTICLE;
        }
    }

    /// The concentration and wind direction the searcher can sense here.
    fn get_v(&self, m: &mut Searcher) {
        let x = m.r.x - self.r.x - 1;
        m.c = 0;
        if x < 0 || x >= self.yv.len() as i32 {
            return;
        }
        let cell = self.yv[x as usize];
        if cell.v == NO_PARTICLE || cell.y != m.r.y - self.r.y {
            return;
        }
        m.c = 1;
        m.v = cell.v;
        m.color = self.color;
    }
}

struct State {
    source: Vec<Option<Source>>,
    searcher: Vec<Option<Searcher>>,
    max_dist: i32,
    max_src: usize,
    max_searcher: usize,
    /// The lattice-to-pixel transform.
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    dx: i32,
    dy: i32,
    delay: u32,
    scr_width: i32,
    scr_height: i32,
    gc_draw: Gc,
    gc_clear: Gc,
    colors: Vec<XColor>,
}

impl State {
    /// `X()`.
    fn tx(&self, x: i32) -> i32 {
        (self.ax * x as f64 + self.bx) as i32
    }

    /// `Y()`.
    fn ty(&self, y: i32) -> i32 {
        (self.ay * y as f64 + self.by) as i32
    }

    fn a_color(&self) -> Pixel {
        self.colors[(random() as usize) % self.colors.len()].pixel
    }

    fn new_searcher(&self) -> Searcher {
        let mut y;
        loop {
            y = rnd(2 * self.max_dist);
            if y >= MIN_DIST && y <= 2 * self.max_dist - MIN_DIST {
                break;
            }
        }
        let r = Point {
            x: self.max_dist,
            y,
        };
        Searcher {
            r,
            vertex: r,
            state: if rnd(2) == 0 {
                StateT::UpRight
            } else {
                StateT::UpLeft
            },
            c: 0,
            v: 0,
            hist: Vec::new(),
            color: self.a_color(),
            rs: rnd(self.dx),
        }
    }

    fn new_source(&self) -> Source {
        let r = if self.max_searcher == 0 {
            Point {
                x: 0,
                y: rnd(2 * self.max_dist),
            }
        } else {
            let x = rnd(self.max_dist / 3);
            let mut y;
            loop {
                y = rnd(2 * self.max_dist);
                if y >= MIN_DIST && y <= 2 * self.max_dist - MIN_DIST {
                    break;
                }
            }
            Point { x, y }
        };

        let n = (self.max_dist - r.x).max(1) as usize;
        let mut inv_rate = rnd(MAX_INV_RATE);
        if inv_rate == 0 {
            inv_rate = 1;
        }
        Source {
            yv: vec![
                Yv {
                    y: 0,
                    v: NO_PARTICLE
                };
                n
            ],
            inv_rate,
            r,
            color: self.a_color(),
        }
    }

    /// Recompute the lattice-to-pixel transform for the current window.
    fn fit(&mut self, width: i32, height: i32) {
        self.scr_width = width;
        self.scr_height = height;
        self.ax = self.scr_width as f64 / self.max_dist as f64;
        self.ay = self.scr_height as f64 / (2.0 * self.max_dist as f64);
        self.bx = 0.0;
        self.by = 0.0;
        self.dx = (self.scr_width / (2 * self.max_dist)).max(1);
        self.dy = (self.scr_height / (4 * self.max_dist)).max(1);
        self.gc_draw.line_width = self.dx / 3 + 1;
    }

    fn draw_searcher(&mut self, d: &mut Dpy, i: usize) {
        let (color, rs, r, hist) = match &self.searcher[i] {
            Some(m) => (m.color, m.rs, m.r, m.hist.clone()),
            None => return,
        };

        let mut r1 = Point {
            x: self.tx(r.x) + rs,
            y: self.ty(r.y),
        };
        self.gc_draw.set_foreground(color);
        d.win()
            .fill_rectangle(&self.gc_draw, r1.x - 2, r1.y - 2, 4, 4);

        // The trajectory, walked back from where the searcher stands now.
        for p in hist.iter().rev() {
            let r2 = Point {
                x: self.tx(p.x) + rs,
                y: self.ty(p.y),
            };
            d.win().draw_line(&self.gc_draw, r1.x, r1.y, r2.x, r2.y);
            r1 = r2;
        }
    }

    fn draw_image(&mut self, d: &mut Dpy) {
        // Retina displays.
        let size = if self.scr_width > 2560 || self.scr_height > 2560 {
            8
        } else {
            4
        };

        for i in 0..self.max_src {
            let (color, inv_rate, sr, plume) = match &self.source[i] {
                Some(s) => (
                    s.color,
                    s.inv_rate,
                    s.r,
                    s.yv.iter()
                        .enumerate()
                        .filter(|(_, c)| c.v != NO_PARTICLE)
                        .map(|(j, c)| (j as i32, c.y))
                        .collect::<Vec<_>>(),
                ),
                None => continue,
            };
            self.gc_draw.set_foreground(color);

            // The source itself: a disc whose size says how hard it is
            // emitting. Only drawn when there is someone looking for it.
            if inv_rate > 0 && self.max_searcher > 0 {
                let x = self.tx(sr.x);
                let y = self.ty(sr.y);
                let j = (self.dx * (MAX_INV_RATE + 1 - inv_rate) / (2 * MAX_INV_RATE)).max(1);
                d.win()
                    .fill_arc(&self.gc_draw, x - j, y - j, 2 * j, 2 * j, 0, FULL_CIRCLE);
            }

            for (j, cy) in plume {
                // Move the particles slightly off lattice.
                let x = self.tx(sr.x + 1 + j) + rnd(self.dx);
                let y = self.ty(sr.y + cy) + rnd(self.dy);
                d.win().fill_arc(
                    &self.gc_draw,
                    x - size / 2,
                    y - size / 2,
                    size,
                    size,
                    0,
                    FULL_CIRCLE,
                );
            }
        }

        for i in 0..self.max_searcher {
            self.draw_searcher(d, i);
        }
    }

    fn animate(&mut self, d: &mut Dpy) {
        for i in 0..self.max_src {
            let Some(s) = &mut self.source[i] else {
                continue;
            };
            s.evolve();

            // Reap dead sources for which all particles are gone.
            if s.inv_rate == 0 && s.yv.iter().all(|c| c.v == NO_PARTICLE) {
                self.source[i] = None;
            }
        }

        // Decide if we want to add new sources.
        for i in 0..self.max_src {
            if self.source[i].is_none() && rnd(self.max_dist * self.max_src as i32) == 0 {
                let s = self.new_source();
                self.source[i] = Some(s);
            }
        }

        // Kill some sources when searchers do not do that.
        if self.max_searcher == 0 {
            for i in 0..self.max_src {
                if self.source[i].is_some() && rnd(self.max_dist * self.max_src as i32) == 0 {
                    self.source[i] = None;
                }
            }
        }

        for i in 0..self.max_searcher {
            if matches!(&self.searcher[i], Some(m) if m.state == StateT::Done) {
                self.searcher[i] = None;
            }

            if self.searcher[i].is_none() {
                if rnd(self.max_dist * self.max_searcher as i32) == 0 {
                    let m = self.new_searcher();
                    self.searcher[i] = Some(m);
                } else {
                    continue;
                }
            }

            // Check if the searcher found a source or missed all of them.
            for j in 0..self.max_src {
                let Some(s) = &self.source[j] else { continue };
                if s.inv_rate == 0 {
                    continue;
                }
                let m = self.searcher[i].as_mut().unwrap();
                if m.r.x < 0 {
                    m.state = StateT::Done;
                    break;
                }
                if s.r == m.r {
                    m.state = StateT::Done;
                    // The source disappears, and the searcher flashes.
                    m.color = WHITE;
                    self.source[j].as_mut().unwrap().inv_rate = 0;
                    break;
                }
            }

            // Set it here in case we do not get to get_v().
            let m = self.searcher[i].as_mut().unwrap();
            m.c = 0;

            if m.state != StateT::Done {
                for j in 0..self.max_src {
                    let Some(s) = &self.source[j] else { continue };
                    let m = self.searcher[i].as_mut().unwrap();
                    s.get_v(m);
                    if m.c == 1 {
                        break;
                    }
                }
            }

            self.searcher[i].as_mut().unwrap().move_();
        }

        self.draw_image(d);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let ncolors = (d.res.int("colors").max(0) + 1).max(1) as usize;
    let max_dist = d.res.int("distance").clamp(MIN_DIST + 1, MAX_DIST);
    let max_src = d.res.int("sources").max(1) as usize;
    let max_searcher = d.res.int("searchers").max(0) as usize;

    let mut st = State {
        source: (0..max_src).map(|_| None).collect(),
        searcher: (0..max_searcher).map(|_| None).collect(),
        max_dist,
        max_src,
        max_searcher,
        ax: 1.0,
        ay: 1.0,
        bx: 0.0,
        by: 0.0,
        dx: 1,
        dy: 1,
        delay: d.res.int("delay").max(0) as u32,
        scr_width: d.width(),
        scr_height: d.height(),
        gc_draw: Gc::default(),
        gc_clear: Gc::new(d.res.pixel("background"), d.res.pixel("background")),
        colors: make_random_colormap(ncolors, true),
    };
    st.fit(d.width(), d.height());
    let first = st.new_source();
    st.source[0] = Some(first);
    d.clear_window();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (w, h) = (self.scr_width, self.scr_height);
        d.win().fill_rectangle(&self.gc_clear, 0, 0, w, h);
        self.animate(d);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.fit(width, height);
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    "*distance: 40",
    "*sources: 25",
    "*searchers: 25",
    "*delay: 20000",
    "*colors: 20",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("distance", "Distance", 10.0, 250.0, 5.0, 0, "40"),
    Opt::slider("sources", "Sources", 1.0, 100.0, 1.0, 0, "25"),
    Opt::slider("searchers", "Searchers", 1.0, 100.0, 1.0, 0, "25"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "anemotaxis",
    label: "Anemotaxis",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Eugene Balkovsky",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=hIqmIQbQkW8"),
        blurb: "Searches for a source of odor in a turbulent atmosphere.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
