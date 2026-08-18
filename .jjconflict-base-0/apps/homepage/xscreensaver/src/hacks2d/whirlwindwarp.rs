//! Port of `hacks/whirlwindwarp.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 2000 Paul "Joey" Clark <pclark@bris.ac.uk>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! 19971004: Johannes Keukelaar <johannes@nada.kth.se>: Use helix screen
//!           eraser.
//!
//! WhirlwindWarp: moving stars.  Ported from QBasic by Joey.
//! Version 1.3.  Smooth with pretty colours.
//!
//! This code adapted from original program by jwz/jk above.
//! Freely distrubtable.  Please keep this tag with
//! this code, and add your own if you contribute.
//! I would be delighted to hear if have made use of this code.
//! If you find this code useful or have any queries, please
//! contact me: pclark@cs.bris.ac.uk / joeyclark@usa.net
//! Paul "Joey" Clark, hacking for humanity, Feb 99
//! www.cs.bris.ac.uk/~pclark | www.changetheworld.org.uk
//!
//! 15/May/05: Added colour rotation, limit on max FPS, scaling size dots, and smoother drivers.
//!  4/Mar/01: Star colours are cycled when new colour can not be allocated.
//!  4/Mar/01: Stars are plotted as squares with size relative to screen.
//! 28/Nov/00: Submitted to xscreensaver as "whirlwindwarp".
//! 10/Oct/00: Ported to xscreensaver as "twinkle".
//! 19/Feb/98: Meters and interaction added for Ivor's birthday "stars11f".
//! 11/Aug/97: Original QBasic program.
//! ```
//!
//! A few hundred stars in a square from -1 to 1, pushed around by sixteen
//! force fields laid one on top of another: warp, rotation, two asymptotes,
//! a squirge towards each edge, a split down the middle and a pair of waves.
//! Each field drifts through its own strength on a random walk and switches
//! itself on and off, so the picture never settles into one shape. A ring
//! buffer of past positions is erased behind the stars, which is the trail.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{hsv_to_rgb, rgb};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, random_below,
};

/// Maximum number of points, maximum tail length, and the number of
/// forcefields/effects (hard-coded).
const MAXPS: usize = 1000;
const MAXTS: usize = 50;
const FS: usize = 16;

/// Upstream caps itself at 200 frames a second with a wall-clock check. The
/// runtime already paces on the delay a hack returns, so ask for that instead.
const MAX_FPS_DELAY: u32 = 1_000_000 / 200;

/// The wave phases are seeded from upstream's `3.141`, which is pi as the
/// author typed it rather than pi as the machine knows it. Written as a
/// fraction so it stays that number and does not get quietly rounded up to the
/// real constant.
const AUTHORS_PI: f32 = 3141.0 / 1000.0;

struct State {
    gc: Gc,
    default_fg_pixel: Pixel,
    bg_pixel: Pixel,

    scrwid: i32,
    scrhei: i32,
    starsize: i32,

    /// Current x,y of stars in realspace.
    cx: Vec<f32>,
    cy: Vec<f32>,
    /// Previous x,y plots in pixelspace, for removal later.
    tx: Vec<i32>,
    ty: Vec<i32>,

    /// Is field on or off?
    fon: [bool; FS],
    /// Current parameter.
    var: [f32; FS],
    /// Optimum (central/mean) value.
    op: [f32; FS],
    acc: [f32; FS],
    vel: [f32; FS],

    ps: usize,
    ts: usize,
    meters: bool,

    initted: bool,
    /// The colour assigned to each star.
    color: Vec<Pixel>,
    nt: usize,
    resets: i32,
    lastresets: i32,
    colsavailable: usize,
    hue: i32,
}

/// Between -1.0 (inclusive) and +1.0 (exclusive).
fn myrnd() -> f32 {
    2.0 * ((random_below(10_000_000) as f32 / 10_000_000.0) - 0.5)
}

/// Adjust a variable `var` about optimum `op`, with `damp` = dampening about
/// op and `force` = force of random perturbation.
fn perturb(var: f32, op: f32, damp: f32, force: f32) -> f32 {
    op + damp * (var - op) + force * myrnd() / 4.0
}

fn star_color() -> Pixel {
    color_at(random_below(360))
}

fn color_at(hue: i32) -> Pixel {
    let (r, g, b) = hsv_to_rgb(
        hue,
        (0.6 + 0.4 * myrnd()) as f64,
        (0.6 + 0.4 * myrnd()) as f64,
    );
    rgb((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8)
}

impl State {
    fn stars_newp(&mut self, pp: usize) {
        self.cx[pp] = myrnd();
        self.cy[pp] = myrnd();
    }

    /// Get pixel coordinates of a star.
    fn scrpos_x(&self, pp: usize) -> i32 {
        (self.scrwid as f32 * (self.cx[pp] + 1.0) / 2.0) as i32
    }

    fn scrpos_y(&self, pp: usize) -> i32 {
        (self.scrhei as f32 * (self.cy[pp] + 1.0) / 2.0) as i32
    }

    /// Draw a meter of a forcefield's parameter.
    fn draw_meter(&mut self, d: &mut Dpy, ff: usize) {
        let mut x = self.scrwid / 2;
        let y = ff as i32 * 10;
        let mut w = ((self.var[ff] - self.op[ff]) * self.scrwid as f32 * 4.0) as i32;
        let h = 7;
        if w < 0 {
            w = -w;
            x -= w;
        }
        if self.fon[ff] {
            d.win().fill_rectangle(&self.gc, x, y, w, h);
        }
    }

    /// Move a star according to acting forcefields.
    ///
    /// In theory all these checks are unnecessary, since each forcefield
    /// effect should do nothing when its var = op. But they are good for
    /// efficiency because this runs once for every point.
    fn stars_move(&mut self, pp: usize) {
        let mut x = self.cx[pp];
        let mut y = self.cy[pp];

        // Squirge towards edges (makes a leaf shape, previously split the
        // screen in 4 but now only 1). These must go first, to avoid
        // x + 1.0 < 0.
        if self.fon[6] {
            x = -1.0 + 2.0 * ((x + 1.0) / 2.0).powf(self.var[6]);
        }
        if self.fon[7] {
            y = -1.0 + 2.0 * ((y + 1.0) / 2.0).powf(self.var[7]);
        }

        // Warping in/out.
        if self.fon[1] {
            x *= self.var[1];
            y *= self.var[1];
        }

        // Rotation.
        if self.fon[2] {
            let a = 1.1 * self.var[2];
            let nx = x * a.cos() + y * a.sin();
            let ny = -x * a.sin() + y * a.cos();
            x = nx;
            y = ny;
        }

        // Asymptotes: looks like a plane with a horizon, equivalent to a 1D
        // warp.
        if self.fon[3] {
            // Horizontal asymptote.
            y *= self.var[3];
        }
        if self.fon[4] {
            // Vertical asymptote. Same maths as the last, but with op = 0.
            x += self.var[4] * x;
        }
        if self.fon[5] {
            // Vertical asymptote at right of screen.
            x = (x - 1.0) * self.var[5] + 1.0;
        }

        // Splitting (whirlwind effect).
        let num_splits = 2 + (self.var[0].abs() * 1000.0) as i32;
        let thru = ((num_splits as f32 * pp as f32 / self.ps as f32) as i32) as f32
            / (num_splits - 1) as f32;
        if self.fon[8] {
            x += 0.5 * self.var[8] * (-1.0 + 2.0 * thru);
        }
        if self.fon[9] {
            y += 0.5 * self.var[9] * (-1.0 + 2.0 * thru);
        }

        // Waves.
        if self.fon[10] {
            y += 0.4 * self.var[10] * (300.0 * self.var[12] * x + 600.0 * self.var[11]).sin();
        }
        if self.fon[13] {
            x += 0.4 * self.var[13] * (300.0 * self.var[15] * y + 600.0 * self.var[14]).sin();
        }

        self.cx[pp] = x;
        self.cy[pp] = y;
    }

    /// Turns a forcefield on, and ensures its vars are suitable.
    fn turn_on_field(&mut self, ff: usize) {
        if !self.fon[ff] {
            self.acc[ff] = 0.02 * myrnd();
            self.vel[ff] = 0.0;
            self.var[ff] = self.op[ff];
        }
        self.fon[ff] = true;
        if ff == 10 {
            self.turn_on_field(11);
            self.turn_on_field(12);
        }
        if ff == 13 {
            self.turn_on_field(14);
            self.turn_on_field(15);
        }
    }

    fn setup(&mut self, d: &mut Dpy) {
        d.clear_window();
        self.scrwid = d.width();
        self.scrhei = d.height();
        self.starsize = (self.scrhei / 480).max(1);

        self.color = (0..self.ps).map(|_| star_color()).collect();
        self.colsavailable = self.ps.saturating_sub(1);

        // Set up central (optimal) points for each different forcefield.
        // The names upstream gives them, in order: warp, rotation, horizontal
        // asymptote, vertical asymptote, vertical asymptote right, squirge x,
        // squirge y, split number, split velocity x and y, then the amplitude,
        // phase and frequency of each of the two waves.
        self.op = [0.0; FS];
        self.op[1] = 1.0;
        self.op[3] = 1.0;
        self.op[5] = 1.0;
        self.op[6] = 1.0;
        self.op[7] = 1.0;
        self.op[11] = myrnd() * AUTHORS_PI;
        self.op[12] = 0.01;
        self.op[14] = myrnd() * AUTHORS_PI;
        self.op[15] = 0.01;

        // Initialise parameters to optimum, all off.
        for f in 0..FS {
            self.var[f] = self.op[f];
            self.fon[f] = myrnd() > 0.5;
            self.acc[f] = 0.02 * myrnd();
            self.vel[f] = 0.0;
        }

        for p in 0..self.ps {
            self.stars_newp(p);
        }

        // tx[nt], ty[nt] remember earlier screen plots (tails of stars) which
        // are deleted when nt comes round again.
        self.tx = vec![0; self.ps * self.ts];
        self.ty = vec![0; self.ps * self.ts];
        self.nt = 0;
        self.resets = 0;
        self.hue = (180.0 + 180.0 * myrnd()) as i32;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");
    let ps = d.res.int("points").clamp(1, MAXPS as i32) as usize;
    let ts = d.res.int("tails").clamp(1, MAXTS as i32) as usize;

    Box::new(State {
        gc: Gc::new(fg, bg),
        default_fg_pixel: fg,
        bg_pixel: bg,
        scrwid: d.width(),
        scrhei: d.height(),
        starsize: 1,
        cx: vec![0.0; ps],
        cy: vec![0.0; ps],
        tx: Vec::new(),
        ty: Vec::new(),
        fon: [false; FS],
        var: [0.0; FS],
        op: [0.0; FS],
        acc: [0.0; FS],
        vel: [0.0; FS],
        ps,
        ts,
        meters: d.res.bool("meters"),
        initted: false,
        color: Vec::new(),
        nt: 0,
        resets: 0,
        lastresets: 0,
        colsavailable: 0,
        hue: 0,
    })
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if !self.initted {
            self.initted = true;
            self.setup(d);
        }

        if myrnd() > 0.75 {
            // Change one of the colours to something near the current hue. By
            // changing a random one, we sometimes get a tight colour spread,
            // sometimes a diverse one.
            let pp = (self.colsavailable as f32 * (0.5 + myrnd() / 2.0)) as usize;
            if pp < self.color.len() {
                self.color[pp] = color_at(self.hue);
            }
            self.hue = (self.hue as f32 + 0.5 + myrnd() * 9.0) as i32;
            if self.hue < 0 {
                self.hue += 360;
            }
            if self.hue >= 360 {
                self.hue -= 360;
            }
        }

        // Move current points.
        self.lastresets = self.resets;
        self.resets = 0;
        let size = self.starsize;
        for p in 0..self.ps {
            // Erase old.
            self.gc.set_foreground(self.bg_pixel);
            let (ox, oy) = (self.tx[self.nt], self.ty[self.nt]);
            d.win().fill_rectangle(&self.gc, ox, oy, size, size);

            self.stars_move(p);
            // If moved off screen, create a new one.
            if self.cx[p] <= -0.9999
                || self.cx[p] >= 0.9999
                || self.cy[p] <= -0.9999
                || self.cy[p] >= 0.9999
                || self.cx[p].abs() < 0.0001
                || self.cy[p].abs() < 0.0001
            {
                self.stars_newp(p);
                self.resets += 1;
            } else if myrnd() > 0.99 {
                // Reset at random.
                self.stars_newp(p);
            }

            let sx = self.scrpos_x(p);
            let sy = self.scrpos_y(p);
            self.gc.set_foreground(self.color[p]);
            d.win().fill_rectangle(&self.gc, sx, sy, size, size);

            // Remember it for removal later.
            self.tx[self.nt] = sx;
            self.ty[self.nt] = sy;
            self.nt = (self.nt + 1) % (self.ps * self.ts);
        }

        // Adjust force fields.
        let mut cnt = 0;
        for f in 0..FS {
            if self.meters {
                // Remove meter from display.
                self.gc.set_foreground(self.bg_pixel);
                self.draw_meter(d, f);
            }

            if self.fon[f] {
                // This configuration produces vars usually below 0.01.
                self.acc[f] = perturb(self.acc[f], 0.0, 0.98, 0.005);
                self.vel[f] = perturb(self.vel[f] + 0.03 * self.acc[f], 0.0, 0.995, 0.0);
                self.var[f] =
                    self.op[f] + (self.var[f] - self.op[f]) * 0.9995 + 0.001 * self.vel[f];
            }

            // Decide whether to turn this forcefield on or off. The splitting
            // effects are made less likely than the rest.
            let prob_on = if f == 8 || f == 9 { 0.999975 } else { 0.9999 };
            if !self.fon[f] && myrnd() > prob_on {
                self.turn_on_field(f);
            } else if self.fon[f]
                && myrnd() > 0.99
                && (self.var[f] - self.op[f]).abs() < 0.0005
                && self.vel[f].abs() < 0.005
            {
                // Only turn it off if it has gently returned to its optimum,
                // as opposed to rapidly passing through it.
                self.fon[f] = false;
            }

            if self.meters {
                // Redraw the meter.
                self.gc.set_foreground(self.color[f % self.color.len()]);
                self.draw_meter(d, f);
            }

            if self.fon[f] {
                cnt += 1;
            }
        }

        // Ensure at least three forcefields are on. Picking randomly might not
        // be enough since 0, 11, 12, 14 and 15 do nothing, but then what is
        // wrong with a rare gentle twinkle?
        if cnt < 3 {
            self.turn_on_field(random_below(FS as i32) as usize);
        }

        if self.meters {
            let (last, now) = (self.lastresets, self.resets);
            self.gc.set_foreground(self.bg_pixel);
            d.win().draw_rectangle(&self.gc, 0, 0, last * 5, 3);
            self.gc.set_foreground(self.default_fg_pixel);
            d.win().draw_rectangle(&self.gc, 0, 0, now * 5, 3);
        }

        MAX_FPS_DELAY
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.scrwid = width;
        self.scrhei = height;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*points: 400",
    "*tails: 8",
    "*meters: false",
];

const OPTS: &[Opt] = &[
    Opt::slider("points", "Particles", 10.0, 1000.0, 10.0, 0, "400"),
    Opt::slider("tails", "Trail size", 1.0, 50.0, 1.0, 0, "8"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "whirlwindwarp",
    label: "Whirlwind Warp",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Paul 'Joey' Clark",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=eWrRhSYzimY"),
        blurb: "Floating stars are acted upon by a mixture of simple 2D force fields.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
