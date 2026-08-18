//! Port of `hacks/anemone.c`.
//!
//! ```text
//! anemone, Copyright (c) 2001 Gabriel Finch
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! FILE            anemone.c
//! MODULE OF       xscreensaver
//!
//! DESCRIPTION     Anemone.
//!
//! WRITTEN BY      Gabriel Finch
//!
//! MODIFICATIONS   june 2001 started
//! ```
//!
//! Wiggling tentacles. Each arm is a chain of points that grows outwards a
//! segment at a time, each new segment inheriting its predecessor's velocity
//! with a small random nudge, and then withdraws again. Every point is jittered
//! by a pixel or two as it is drawn, which is where the wiggle comes from, and
//! the whole thing turns slowly about the vertical axis with each arm drawn at
//! a slightly later angle than the last.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XColor, random_below,
};

/// `RND(x)`.
fn rnd(x: i32) -> i32 {
    random_below(x)
}

#[derive(Clone, Copy, Default)]
struct Vertex {
    x: f64,
    y: f64,
    z: f64,
    sx: i32,
    sy: i32,
    sz: i32,
}

struct Arm {
    col: Pixel,
    numpt: usize,
    growth: i32,
    /// How eager this arm is to change: bigger is more often.
    rate: i32,
    pts: Vec<Vertex>,
}

struct Anemone {
    arms: usize,
    finpoints: usize,
    delay: u32,
    scr_width: i32,
    scr_height: i32,
    gc_draw: Gc,
    gc_clear: Gc,
    width: i32,
    limbs: Vec<Arm>,
    turn: f64,
    turndelta: f64,
    mx: i32,
    my: i32,
    withdraw: i32,
    colors: Vec<XColor>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut width = d.res.int("width").max(1);
    let arms = d.res.int("arms").max(1) as usize;
    let mut finpoints = d.res.int("finpoints").max(2) as usize;
    let mut withdraw = d.res.int("withdraw").max(1);

    if d.width() > 2560 || d.height() > 2560 {
        // Retina displays.
        width *= 4;
        finpoints *= 2;
        withdraw *= 2;
    }

    let ncolors = d.res.int("colors").max(0) as usize + 3;
    let colors = make_smooth_colormap(ncolors);

    let mut gc_draw = Gc::new(d.res.pixel("foreground"), d.res.pixel("background"));
    gc_draw.set_line_width(width);

    let mut st = Anemone {
        arms,
        finpoints,
        delay: d.res.int("delay").max(0) as u32,
        scr_width: d.width(),
        scr_height: d.height(),
        gc_draw,
        gc_clear: Gc::new(d.res.pixel("background"), d.res.pixel("background")),
        width,
        limbs: Vec::new(),
        turn: 0.0,
        turndelta: d.res.float("turnspeed") / 100000.0,
        mx: 0,
        my: 0,
        withdraw,
        colors,
    };
    st.init_appendages();
    Box::new(st)
}

impl Anemone {
    fn init_appendages(&mut self) {
        self.mx = self.scr_width - 1;
        self.my = self.scr_height - 1;

        self.limbs = (0..self.arms)
            .map(|_| {
                let col = self.colors[random_below(self.colors.len() as i32) as usize].pixel;
                let growth = (self.finpoints / 2) as i32 + rnd((self.finpoints / 2).max(1) as i32);
                let rate = rnd(11) * rnd(11);

                // Upstream means to pick a random point inside the unit
                // sphere, but the division here is between two integers, so
                // each coordinate is only ever -1, 0 or 1 and the loop runs
                // until all three come up zero. Every arm therefore starts at
                // the centre, at rest.
                let (mut x, mut y, mut z);
                loop {
                    x = (1 - rnd(1001) / 500) as f64;
                    y = (1 - rnd(1001) / 500) as f64;
                    z = (1 - rnd(1001) / 500) as f64;
                    if x * x + y * y + z * z < 1.0 {
                        break;
                    }
                }

                let mut pts = vec![Vertex::default(); self.finpoints + 1];
                pts[0].x = x * 200.0;
                pts[0].y = self.my as f64 / 2.0 + y * 200.0;
                pts[0].z = z * 200.0;

                // Start the arm going outwards.
                pts[0].sx = (pts[0].x / 5.0) as i32;
                pts[0].sy = ((pts[0].y - self.my as f64 / 2.0) / 5.0) as i32;
                pts[0].sz = (pts[0].z / 5.0) as i32;

                pts[1].x = pts[0].x + pts[0].sx as f64;
                pts[1].y = pts[0].y + pts[0].sy as f64;
                pts[1].z = pts[0].z + pts[0].sz as f64;

                Arm {
                    col,
                    numpt: 1,
                    growth,
                    rate,
                    pts,
                }
            })
            .collect();
    }

    /// Grow the arms a segment at a time, or pull them all in at once.
    fn create_points(&mut self) {
        let withdrawall = rnd(self.withdraw);
        let finpoints = self.finpoints;

        for i in 0..self.limbs.len() {
            if withdrawall == 0 {
                self.limbs[i].growth = -(finpoints as i32);
                self.turndelta = -self.turndelta;
            } else if withdrawall < 11 {
                self.limbs[i].growth = -(self.limbs[i].numpt as i32);
            } else if rnd(100) < self.limbs[i].rate {
                let a = &mut self.limbs[i];
                if a.growth > 0 {
                    a.growth -= 1;
                    if a.growth == 0 {
                        a.growth = -rnd(finpoints as i32) - 1;
                    }
                    if a.numpt < finpoints - 1 {
                        // Add a piece, carrying the last one's velocity.
                        let n = a.numpt;
                        a.numpt += 1;
                        a.pts[n].sx = a.pts[n - 1].sx + rnd(3) - 1;
                        a.pts[n].sy = a.pts[n - 1].sy + rnd(3) - 1;
                        a.pts[n].sz = a.pts[n - 1].sz + rnd(3) - 1;
                        a.pts[n + 1].x = a.pts[n].x + a.pts[n].sx as f64;
                        a.pts[n + 1].y = a.pts[n].y + a.pts[n].sy as f64;
                        a.pts[n + 1].z = a.pts[n].z + a.pts[n].sz as f64;
                    }
                }
            }
        }
    }

    fn draw_arm(&mut self, d: &mut Dpy, at: usize, sint: f64, cost: f64) {
        let numpt = self.limbs[at].numpt;
        if numpt == 1 {
            return;
        }
        let col = self.limbs[at].col;
        self.gc_draw.set_foreground(col);
        let mx2 = self.mx / 2;

        let (mut cx, mut cy, mut cz) = {
            let p = self.limbs[at].pts[0];
            (p.x, p.y, p.z)
        };
        let (mut nx, mut ny, mut nz) = (0.0, 0.0, 0.0);

        for q in 0..numpt - 1 {
            let p = self.limbs[at].pts[q + 1];
            // Two pixels of jitter per point per frame: this is the wiggle.
            nx = p.x + 2.0 - rnd(5) as f64;
            ny = p.y + 2.0 - rnd(5) as f64;
            nz = p.z + 2.0 - rnd(5) as f64;

            d.win().draw_line(
                &self.gc_draw,
                mx2 + (cx * cost - cz * sint) as i32,
                cy as i32,
                mx2 + (nx * cost - nz * sint) as i32,
                ny as i32,
            );

            cx = nx;
            cy = ny;
            cz = nz;
        }

        // The tip is a fat dot: the two ends of this line are the same point.
        self.gc_draw.set_line_width(self.width * 3);
        d.win().draw_line(
            &self.gc_draw,
            self.mx / 2 + (cx * cost - cz * sint) as i32,
            cy as i32,
            self.mx / 2 + (nx * cost - nz * sint) as i32,
            ny as i32,
        );
        self.gc_draw.set_line_width(self.width);
    }
}

impl Screenhack for Anemone {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (w, h) = (self.scr_width, self.scr_height);
        d.win().fill_rectangle(&self.gc_clear, 0, 0, w, h);

        let (sint, cost) = (self.turn.sin(), self.turn.cos());
        for i in 0..self.limbs.len() {
            if rnd(25) < self.limbs[i].rate && self.limbs[i].growth < 0 {
                let a = &mut self.limbs[i];
                if a.numpt > 1 {
                    a.numpt -= 1;
                }
                a.growth += 1;
                if a.growth == 0 {
                    a.growth = rnd((self.finpoints - a.numpt).max(1) as i32) + 1;
                }
            }
            self.draw_arm(d, i, sint, cost);
            // Each arm is drawn at a slightly later angle than the last, which
            // is what makes the whole anemone appear to turn.
            self.turn += self.turndelta;
        }
        self.create_points();

        if self.turn >= std::f64::consts::TAU {
            self.turn -= std::f64::consts::TAU;
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.scr_width = width;
        self.scr_height = height;
        self.mx = width - 1;
        self.my = height - 1;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*arms: 128",
    "*width: 2",
    "*finpoints: 64",
    "*delay: 40000",
    "*withdraw: 1200",
    "*turnspeed: 50",
    "*colors: 20",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Speed", 0.0, 80000.0, 1000.0, 0, "40000").inverted(),
    Opt::slider("arms", "Arms", 2.0, 500.0, 1.0, 0, "128"),
    Opt::slider("finpoints", "Tentacles", 3.0, 200.0, 1.0, 0, "64"),
    Opt::slider("width", "Thickness", 1.0, 10.0, 1.0, 0, "2"),
    Opt::slider(
        "withdraw",
        "Withdraw frequency",
        12.0,
        10000.0,
        10.0,
        0,
        "1200",
    ),
    Opt::slider("turnspeed", "Turn speed", 0.0, 1000.0, 10.0, 0, "50"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "anemone",
    label: "Anemone",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Gabriel Finch",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=usITxM2YJZs"),
        blurb: "Wiggling tentacles.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
