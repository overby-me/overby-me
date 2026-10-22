//! Port of `hacks/lmorph.c`.
//!
//! ```text
//! lmorph, Copyright (c) 1993-1999 Sverre H. Huseby and Glenn T. Lines
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//!  FILE            lmorph.c
//!  MODULE OF       xscreensaver
//!
//!  DESCRIPTION     Smooth and non-linear morphing between 1D curves.
//!
//!  WRITTEN BY      Sverre H. Huseby                Glenn T. Lines
//!                  Kurvn. 30                       Ostgaardsgt. 5
//!                  N-0495 Oslo                     N-0474 Oslo
//!                  Norway                          Norway
//!
//!                  E-mail: sverrehu@online.no      E-mail: glennli@ifi.uio.no
//!                  URL:    http://home.sol.no/~sverrehu/
//!
//!                  The original idea, and the bilinear interpolation
//!                  mathematics used, emerged in the head of the wise
//!                  Glenn T. Lines.
//!
//!  MODIFICATIONS   october 1999 (shh)
//!                    * Removed option to use integer arithmetic.
//!                    * Increased default number of points, and brightened
//!                      the foreground color a little bit.
//!                    * Minor code cleanup (very minor, that is).
//!                    * Default number of steps is no longer random.
//!                    * Added -linewidth option (and resource).
//!
//!                  october 1999 (gtl)
//!                    * Added cubic interpolation between shapes
//!                    * Added non-linear transformation speed
//!
//!                  june 1998 (shh)
//!                    * Minor code cleanup.
//!
//!                  january 1997 (shh)
//!                    * Some code reformatting.
//!                    * Added possibility to use float arithmetic.
//!                    * Added -figtype option.
//!                    * Made color blue default.
//!
//!                  december 1995 (jwz)
//!                    * Function headers converted from ANSI to K&R.
//!                    * Added posibility for random number of steps, and
//!                      made this the default.
//!
//!                  march 1995 (shh)
//!                    * Converted from an MS-Windows program to X Window.
//!
//!                  november 1993 (gtl, shh, lots of beer)
//!                    * Original Windows version (we didn't know better).
//! ```
//!
//! A dozen closed and open figures are generated once, all with the same
//! number of points, and the hack cubic-interpolates from whichever one it is
//! showing to the next. The interpolation is not linear in time: each point
//! gets a sinusoidal speed offset around the ring, so the figure appears to
//! unwind rather than fade from one shape into the other.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XPoint, random_below,
};

const TWO_PI: f64 = 2.0 * std::f64::consts::PI;

const FT_OPEN: u32 = 1;
const FT_CLOSED: u32 = 2;
const FT_ALL: u32 = FT_OPEN | FT_CLOSED;

struct State {
    /// The figure arrays, all `num_points` long.
    figs: Vec<Vec<XPoint>>,
    num_points: usize,
    /// Current work array number.
    n_work: usize,
    n_from: usize,
    n_to: usize,
    n_next: usize,
    /// Shifts the starting point of a figure.
    shift: usize,

    delay: u32,

    /// Working arrays.
    work: [Vec<XPoint>; 2],
    /// Slope at the start and at the end of the morph.
    slope_from: Vec<XPoint>,
    slope_to: Vec<XPoint>,

    scr_width: i32,
    scr_height: i32,
    curr_gamma: f64,
    max_gamma: f64,
    delta_gamma: f64,
    gc_draw: Gc,
}

fn init_point_arrays(
    width: i32,
    height: i32,
    num_points: usize,
    fig_type: u32,
) -> Vec<Vec<XPoint>> {
    let mx = (width - 1) as f64;
    let my = (height - 1) as f64;
    let mp = (num_points - 1).max(1) as f64;
    let n = num_points;
    let mut figs: Vec<Vec<XPoint>> = Vec::new();

    let pt = |x: f64, y: f64| XPoint {
        x: x as i32,
        y: y as i32,
    };

    if fig_type & FT_CLOSED != 0 {
        // Rectangle.
        let mut f = vec![XPoint::default(); n];
        let s = n / 4;
        for q in 0..s {
            let t = q as f64 / s as f64;
            f[q] = pt(t * mx, 0.0);
            f[s + q] = pt(mx, t * my);
            f[2 * s + q] = pt(mx - t * mx, my);
            f[3 * s + q] = pt(0.0, my - t * my);
        }
        for item in f.iter_mut().take(n).skip(4 * s) {
            *item = XPoint { x: 0, y: 0 };
        }
        f[n - 1] = f[0];
        figs.push(f);

        // Upstream keeps these as ints, so the halving truncates.
        let rx = ((width - 1) / 2) as f64;
        let ry = ((height - 1) / 2) as f64;

        // A figure eight lying down, and the same standing up.
        for (sx, sy, cx, cy) in [(rx, 1.0, ry, 3.0), (ry, 3.0, ry, 1.0)] {
            let mut f = vec![XPoint::default(); n];
            for (q, item) in f.iter_mut().enumerate() {
                let t = q as f64 / mp;
                *item = pt(
                    mx / 2.0 + sx * (sy * TWO_PI * t).sin(),
                    my / 2.0 + cx * (cy * TWO_PI * t).cos(),
                );
            }
            f[n - 1] = f[0];
            figs.push(f);
        }

        // A cog: a circle with thirty teeth.
        let mut f = vec![XPoint::default(); n];
        for (q, item) in f.iter_mut().enumerate() {
            let t = q as f64 / mp;
            let r = 0.8 - 0.2 * (30.0 * TWO_PI * t).sin();
            *item = pt(
                mx / 2.0 + ry * r * (TWO_PI * t).sin(),
                my / 2.0 + ry * r * (TWO_PI * t).cos(),
            );
        }
        f[n - 1] = f[0];
        figs.push(f);

        // A circle, then an ellipse filling the window.
        let mut f = vec![XPoint::default(); n];
        for (q, item) in f.iter_mut().enumerate() {
            let t = q as f64 / mp;
            *item = pt(
                mx / 2.0 + ry * (TWO_PI * t).sin(),
                my / 2.0 + ry * (TWO_PI * t).cos(),
            );
        }
        f[n - 1] = f[0];
        figs.push(f);

        let mut f = vec![XPoint::default(); n];
        for (q, item) in f.iter_mut().enumerate() {
            let t = q as f64 / mp;
            *item = pt(
                mx / 2.0 + rx * (TWO_PI * t).cos(),
                my / 2.0 + ry * (TWO_PI * t).sin(),
            );
        }
        f[n - 1] = f[0];
        figs.push(f);

        // A Lissajous knot.
        let mut f = vec![XPoint::default(); n];
        for (q, item) in f.iter_mut().enumerate() {
            let t = q as f64 / mp;
            *item = pt(
                mx / 2.0 + rx * (2.0 * TWO_PI * t).sin(),
                my / 2.0 + ry * (3.0 * TWO_PI * t).cos(),
            );
        }
        f[n - 1] = f[0];
        figs.push(f);
    }

    if fig_type & FT_OPEN != 0 {
        // Sine wave, one period.
        let mut f = vec![XPoint::default(); n];
        for (q, item) in f.iter_mut().enumerate() {
            *item = pt(
                (q as f64 / n as f64) * mx,
                (1.0 - ((q as f64 / mp) * TWO_PI).sin()) * my / 2.0,
            );
        }
        figs.push(f);

        // Cosine, three periods.
        let mut f = vec![XPoint::default(); n];
        for (q, item) in f.iter_mut().enumerate() {
            *item = pt(
                (q as f64 / mp) * mx,
                (1.0 - ((q as f64 / mp) * 3.0 * TWO_PI).cos()) * my / 2.0,
            );
        }
        figs.push(f);

        let ry = ((height - 1) / 2) as f64;

        // Spiral, one endpoint at the bottom, then one at the top.
        for (turns, sign) in [(5.0, 1.0), (6.0, -1.0)] {
            let mut f = vec![XPoint::default(); n];
            for (q, item) in f.iter_mut().enumerate() {
                let t = q as f64 / mp;
                *item = pt(
                    mx / 2.0 + ry * (turns * TWO_PI * t).sin() * t,
                    my / 2.0 + sign * ry * (turns * TWO_PI * t).cos() * t,
                );
            }
            figs.push(f);
        }

        // Sine, five periods.
        let mut f = vec![XPoint::default(); n];
        for (q, item) in f.iter_mut().enumerate() {
            *item = pt(
                (q as f64 / mp) * mx,
                (1.0 - ((q as f64 / mp) * 5.0 * TWO_PI).sin()) * my / 2.0,
            );
        }
        figs.push(f);
    }

    // Make some space around the figures.
    let marginx = (width) / 10;
    let marginy = (height) / 10;
    let scalex = (width - 2 * marginx) as f64 / width as f64;
    let scaley = (height - 2 * marginy) as f64 / height as f64;
    for f in &mut figs {
        for p in f.iter_mut() {
            p.x = marginx + (p.x as f64 * scalex) as i32;
            p.y = marginy + (p.y as f64 * scaley) as i32;
        }
    }
    figs
}

impl State {
    /// Upstream calls this 55% of execution time. Each point rides its own
    /// cubic between the figure it came from and the one it is going to, at
    /// its own moment: `speed` runs a sine around the ring so one part of the
    /// curve arrives before another.
    fn create_points(&mut self) {
        let n = self.num_points;
        let (from, to) = (&self.figs[self.n_from], &self.figs[self.n_to]);
        let out = &mut self.work[self.n_work];
        for i in 0..n {
            let q = n - i;
            let speed = 0.45 * (TWO_PI * (q + self.shift) as f64 / (n as f64 - 1.0)).sin();
            let e = self.curr_gamma - 0.5 + 0.7 * speed;
            let fg = self.curr_gamma + 1.67 * speed * (-200.0 * e * e).exp();
            let f1g = 1.0 - fg;

            let (p1, p2) = (from[i], to[i]);
            let (q1, q2) = (self.slope_from[i], self.slope_to[i]);
            out[i] = XPoint {
                x: (f1g * f1g * f1g * p1.x as f64
                    + f1g * f1g * fg * (3 * p1.x + q1.x) as f64
                    + f1g * fg * fg * (3 * p2.x - q2.x) as f64
                    + fg * fg * fg * p2.x as f64) as i32,
                y: (f1g * f1g * f1g * p1.y as f64
                    + f1g * f1g * fg * (3 * p1.y + q1.y) as f64
                    + f1g * fg * fg * (3 * p2.y - q2.y) as f64
                    + fg * fg * fg * p2.y as f64) as i32,
            };
        }
    }

    fn animate(&mut self, d: &mut Dpy) {
        if self.curr_gamma > self.max_gamma {
            self.curr_gamma = 0.0;
            self.n_from = self.n_to;
            self.n_to = self.n_next;
            loop {
                self.n_next = random_below(self.figs.len() as i32) as usize;
                if self.n_next != self.n_to || self.figs.len() < 2 {
                    break;
                }
            }

            self.shift = random_below(self.num_points as i32) as usize;
            if random_below(2) == 1 {
                // Reverse the array to get more variation.
                self.figs[self.n_next].reverse();
            }

            // Calculate the slopes.
            for i in 0..self.num_points {
                self.slope_from[i] = self.slope_to[i];
                self.slope_to[i] = XPoint {
                    x: self.figs[self.n_next][i].x - self.figs[self.n_to][i].x,
                    y: self.figs[self.n_next][i].y - self.figs[self.n_to][i].y,
                };
            }
        }

        self.create_points();

        d.clear_window();
        let pts = std::mem::take(&mut self.work[self.n_work]);
        d.win().draw_lines(&self.gc_draw, &pts);
        self.work[self.n_work] = pts;

        self.n_work ^= 1;
        self.curr_gamma += self.delta_gamma;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let num_points = d.res.int("points").clamp(4, 5000) as usize;
    let mut steps = d.res.int("steps");
    if steps <= 0 {
        steps = random_below(400) + 100;
    }

    let fig_type = match d.res.string("figtype") {
        "open" => FT_OPEN,
        "closed" => FT_CLOSED,
        _ => FT_ALL,
    };

    let mut gc_draw = Gc::new(d.res.pixel("foreground"), d.res.pixel("background"));
    let width = d.res.int("linewidth");
    gc_draw.set_line_width(if width == 1 { 0 } else { width });

    let figs = init_point_arrays(d.width(), d.height(), num_points, fig_type);
    let n_to = random_below(figs.len() as i32) as usize;
    let mut n_next = n_to;
    while n_next == n_to && figs.len() > 1 {
        n_next = random_below(figs.len() as i32) as usize;
    }

    let mut st = State {
        num_points,
        n_work: 0,
        n_from: n_to,
        n_to,
        n_next,
        shift: 0,
        delay: d.res.int("delay").max(0) as u32,
        work: [
            vec![XPoint::default(); num_points],
            vec![XPoint::default(); num_points],
        ],
        slope_from: vec![XPoint::default(); num_points],
        slope_to: vec![XPoint::default(); num_points],
        scr_width: d.width(),
        scr_height: d.height(),
        // Force creation of a new figure at startup.
        max_gamma: 1.0,
        curr_gamma: 2.0,
        delta_gamma: 1.0 / steps as f64,
        gc_draw,
        figs,
    };
    st.n_work = 0;
    d.clear_window();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.animate(d);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape handler at all, so the figures keep the size
        // of the window they were made in. A canvas is resized far more often
        // than an X window was, so rebuild them.
        if width != self.scr_width || height != self.scr_height {
            self.scr_width = width;
            self.scr_height = height;
            let fig_type = match d.res.string("figtype") {
                "open" => FT_OPEN,
                "closed" => FT_CLOSED,
                _ => FT_ALL,
            };
            self.figs = init_point_arrays(width, height, self.num_points, fig_type);
            self.n_from = self.n_from.min(self.figs.len() - 1);
            self.n_to = self.n_to.min(self.figs.len() - 1);
            self.n_next = self.n_next.min(self.figs.len() - 1);
            d.clear_window();
        }
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: #4444FF",
    "*points: 200",
    "*steps: 150",
    "*delay: 70000",
    "*figtype: all",
    "*linewidth: 5",
];

const TYPES: &[SelectItem] = &[
    SelectItem {
        value: "all",
        label: "Open and closed figures",
    },
    SelectItem {
        value: "open",
        label: "Open figures",
    },
    SelectItem {
        value: "closed",
        label: "Closed figures",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "70000").inverted(),
    Opt::slider("points", "Control points", 10.0, 1000.0, 10.0, 0, "200"),
    Opt::slider("steps", "Interpolation steps", 100.0, 500.0, 10.0, 0, "150"),
    Opt::slider("linewidth", "Lines", 1.0, 50.0, 1.0, 0, "5"),
    Opt::select("figtype", "Figures", TYPES, "all"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "lmorph",
    label: "LMorph",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Sverre H. Huseby and Glenn T. Lines",
        year: "1995",
        video: Some("https://www.youtube.com/watch?v=yMbMB7xQMkA"),
        blurb: "Generates random spline-ish line drawings and morphs between them.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
