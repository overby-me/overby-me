//! Port of `hacks/fluidballs.c`.
//!
//! ```text
//! fluidballs, Copyright (c) 2000 by Peter Birtles <peter@bqdesign.com.au>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Ported to X11 and xscreensaver by jwz, 27-Feb-2002.
//!
//! http://paulbourke.net/miscellaneous/particle/
//!
//! Some physics improvements by Steven Barker <steve@blckknght.org>
//! ```
//!
//! A box of balls under gravity. Every pair is tested every frame, and an
//! overlapping pair is first pushed apart by half the overlap each and then
//! given the one-dimensional elastic collision along the line between their
//! centres, scaled by an elasticity that is always a little under one. Mass
//! goes as the cube of the radius, so a big ball shrugs off a small one.
//!
//! The box is shaken when the balls stop moving. Once the furthest any ball
//! travelled in a frame drops below a threshold, or thirty seconds pass,
//! "down" is permuted to one of the four axis directions and everything falls
//! the other way. That is the entire reason the hack does not simply settle
//! into a heap and stay there.
//!
//! A ball can be picked up with the mouse and dragged through the others; its
//! velocity is then taken from how far the pointer moved, so it can be used to
//! throw the rest of them about.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, XColor};
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, frand, random,
};

struct State {
    delay: u32,
    gc: Gc,
    /// Most of the balls, and the one being dragged with the mouse.
    fg: Pixel,
    fg2: Pixel,
    background: Pixel,

    count: usize,
    /// The box the balls are kept in.
    xmin: f32,
    ymin: f32,
    xmax: f32,
    ymax: f32,

    /// Index of the ball being dragged, if any.
    mouse_ball: Option<usize>,
    mouse_x: f32,
    mouse_y: f32,

    /// Time constant: a time-warp multiplier.
    tc: f32,
    /// Horizontal acceleration, which is wind.
    accx: f32,
    /// Vertical acceleration, which is gravity.
    accy: f32,

    vx: Vec<f32>,
    vy: Vec<f32>,
    px: Vec<f32>,
    py: Vec<f32>,
    opx: Vec<f32>,
    opy: Vec<f32>,
    r: Vec<f32>,
    /// Ball mass, precalculated from the radius.
    m: Vec<f32>,
    /// Coefficient of elasticity.
    e: f32,
    max_radius: f32,

    shake_p: bool,
    shake_threshold: f32,
    time_since_shake: i64,
    last_time: f64,
}

/// A colour that is at least half bright in every channel.
fn bright() -> Pixel {
    XColor::from_rgb16(
        (0x8888 + random() % 0x8888) as u16,
        (0x8888 + random() % 0x8888) as u16,
        (0x8888 + random() % 0x8888) as u16,
    )
    .pixel
}

impl State {
    /// Re-pick the colours of the balls, and of the mouse-ball.
    fn recolor(&mut self) {
        self.fg = bright();
        self.fg2 = bright();
    }

    /// Permute "down" to be in a random direction.
    fn shake(&mut self) {
        let (a, b) = (self.accx, self.accy);
        match random() % 4 {
            0 => {
                self.accx = a;
                self.accy = b;
            }
            1 => {
                self.accx = -a;
                self.accy = -b;
            }
            2 => {
                self.accx = b;
                self.accy = a;
            }
            _ => {
                self.accx = -b;
                self.accy = -a;
            }
        }
        self.time_since_shake = 0;
        self.recolor();
    }

    /// Erase the balls at their previous positions, and draw the new ones.
    fn repaint_balls(&mut self, d: &mut Dpy) {
        let mut max_d = 0.0f32;

        for a in 0..self.count {
            let x1a = (self.opx[a] - self.r[a] - self.xmin) as i32;
            let y1a = (self.opy[a] - self.r[a] - self.ymin) as i32;
            let x2a = (self.opx[a] + self.r[a] - self.xmin) as i32;
            let y2a = (self.opy[a] + self.r[a] - self.ymin) as i32;

            let x1b = (self.px[a] - self.r[a] - self.xmin) as i32;
            let y1b = (self.py[a] - self.r[a] - self.ymin) as i32;
            let x2b = (self.px[a] + self.r[a] - self.xmin) as i32;
            let y2b = (self.py[a] + self.r[a] - self.ymin) as i32;

            // Erasing every ball unconditionally rather than only the ones
            // that moved: upstream notes that the optimisation leaves turds.
            self.gc.set_foreground(self.background);
            d.win()
                .fill_arc(&self.gc, x1a, y1a, x2a - x1a, y2a - y1a, 0, FULL_CIRCLE);

            let p = if self.mouse_ball == Some(a) {
                self.fg2
            } else {
                self.fg
            };
            self.gc.set_foreground(p);
            d.win()
                .fill_arc(&self.gc, x1b, y1b, x2b - x1b, y2b - y1b, 0, FULL_CIRCLE);

            if self.shake_p {
                // Distance this ball moved this frame.
                let dx = self.px[a] - self.opx[a];
                let dy = self.py[a] - self.opy[a];
                max_d = max_d.max(dx * dx + dy * dy);
            }

            self.opx[a] = self.px[a];
            self.opy[a] = self.py[a];
        }

        if self.shake_p && self.time_since_shake > 5 {
            max_d /= self.max_radius;
            // When it is stable, or when thirty seconds have passed.
            if max_d < self.shake_threshold || self.time_since_shake > 30 {
                self.shake();
            }
        }

        // Upstream reads the wall clock here; the runtime hands the hack its
        // own clock, and only whole seconds are accumulated either way.
        self.time_since_shake += d.time.floor() as i64 - self.last_time.floor() as i64;
        self.last_time = d.time;
    }

    /// Implement the laws of physics: move balls to their new positions.
    fn update_balls(&mut self) {
        // If we are currently tracking the mouse, update that ball first.
        if let Some(i) = self.mouse_ball {
            self.px[i] = self.mouse_x;
            self.py[i] = self.mouse_y;
            self.vx[i] = 0.1 * (self.px[i] - self.opx[i]) * self.tc;
            self.vy[i] = 0.1 * (self.py[i] - self.opy[i]) * self.tc;
        }

        // For each ball, compute the influence of every other ball.
        for a in 0..self.count.saturating_sub(1) {
            for b in (a + 1)..self.count {
                let mut d = (self.px[a] - self.px[b]) * (self.px[a] - self.px[b])
                    + (self.py[a] - self.py[b]) * (self.py[a] - self.py[b]);
                let dee2 = (self.r[a] + self.r[b]) * (self.r[a] + self.r[b]);
                if d >= dee2 {
                    continue;
                }

                d = d.sqrt();
                let dd = self.r[a] + self.r[b] - d;
                let cdx = (self.px[b] - self.px[a]) / d;
                let cdy = (self.py[b] - self.py[a]) / d;

                // Move each ball apart from the other by half the collision
                // distance.
                self.px[a] -= 0.5 * dd * cdx;
                self.py[a] -= 0.5 * dd * cdy;
                self.px[b] += 0.5 * dd * cdx;
                self.py[b] += 0.5 * dd * cdy;

                let (ma, mb) = (self.m[a], self.m[b]);
                let (mut vxa, mut vya) = (self.vx[a], self.vy[a]);
                let (mut vxb, mut vyb) = (self.vx[b], self.vy[b]);

                // The component of each velocity along the axis of the
                // collision.
                let vca = vxa * cdx + vya * cdy;
                let vcb = vxb * cdx + vyb * cdy;

                // Elastic collision, with some energy lost to inelasticity.
                let mut dva = (vca * (ma - mb) + vcb * 2.0 * mb) / (ma + mb) - vca;
                let mut dvb = (vcb * (mb - ma) + vca * 2.0 * ma) / (ma + mb) - vcb;
                dva *= self.e;
                dvb *= self.e;

                vxa += dva * cdx;
                vya += dva * cdy;
                vxb += dvb * cdx;
                vyb += dvb * cdy;

                self.vx[a] = vxa;
                self.vy[a] = vya;
                self.vx[b] = vxb;
                self.vy[b] = vyb;
            }
        }

        // Force all balls to be on screen.
        for a in 0..self.count {
            if self.px[a] <= self.xmin + self.r[a] {
                self.px[a] = self.xmin + self.r[a];
                self.vx[a] = -self.vx[a] * self.e;
            }
            if self.px[a] >= self.xmax - self.r[a] {
                self.px[a] = self.xmax - self.r[a];
                self.vx[a] = -self.vx[a] * self.e;
            }
            if self.py[a] <= self.ymin + self.r[a] {
                self.py[a] = self.ymin + self.r[a];
                self.vy[a] = -self.vy[a] * self.e;
            }
            if self.py[a] >= self.ymax - self.r[a] {
                self.py[a] = self.ymax - self.r[a];
                self.vy[a] = -self.vy[a] * self.e;
            }
        }

        // Apply gravity to all balls.
        for a in 0..self.count {
            if self.mouse_ball == Some(a) {
                continue;
            }
            self.vx[a] += self.accx * self.tc;
            self.vy[a] += self.accy * self.tc;
            self.px[a] += self.vx[a] * self.tc;
            self.py[a] += self.vy[a] * self.tc;
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.width(), d.height());
    let (xmin, ymin) = (0.0f32, 0.0f32);
    let (xmax, ymax) = (width as f32, height as f32);
    let (extx, exty) = (xmax - xmin, ymax - ymin);

    let mut count = d.res.int("count");
    if count < 1 {
        count = 20;
    }

    let mut max_radius = d.res.float("size") as f32 / 2.0;
    if max_radius < 1.0 {
        max_radius = 1.0;
    }
    if width > 2560 || height > 2560 {
        max_radius *= 3.0; // Retina displays.
    }
    if (width < 100 || height < 100) && max_radius > 5.0 {
        max_radius = 5.0; // Tiny window.
    }

    let random_sizes_p = d.res.bool("random");

    // If the initial window size is too small to hold all these balls, make
    // fewer of them.
    {
        let r = if random_sizes_p {
            max_radius * 0.7
        } else {
            max_radius
        };
        let ball_area = std::f32::consts::PI * r * r;
        let balls_area = count as f32 * ball_area;
        // Do not pack it completely full.
        let window_area = width as f32 * height as f32 * 0.75;
        if balls_area > window_area {
            count = (window_area / ball_area) as i32;
        }
    }
    let count = count.max(0) as usize;

    let mut accx = d.res.float("wind") as f32;
    if !(-1.0..=1.0).contains(&accx) {
        accx = 0.0;
    }
    let mut accy = d.res.float("gravity") as f32;
    if !(-1.0..=1.0).contains(&accy) {
        accy = 0.01;
    }
    let mut e = d.res.float("elasticity") as f32;
    if !(0.2..=1.0).contains(&e) {
        e = 0.97;
    }
    let mut tc = d.res.float("timeScale") as f32;
    if tc <= 0.0 || tc > 10.0 {
        tc = 1.0;
    }

    let background = d.res.pixel("background");
    let mut st = State {
        delay: d.res.int("delay").max(0) as u32,
        gc: Gc::new(d.res.pixel("foreground"), background),
        fg: d.res.pixel("foreground"),
        fg2: d.res.pixel("mouseForeground"),
        background,
        count,
        xmin,
        ymin,
        xmax,
        ymax,
        mouse_ball: None,
        mouse_x: 0.0,
        mouse_y: 0.0,
        tc,
        accx,
        accy,
        vx: vec![0.0; count],
        vy: vec![0.0; count],
        px: vec![0.0; count],
        py: vec![0.0; count],
        opx: vec![0.0; count],
        opy: vec![0.0; count],
        r: vec![0.0; count],
        m: vec![0.0; count],
        e,
        max_radius,
        shake_p: d.res.bool("shake"),
        shake_threshold: d.res.float("shakeThreshold") as f32,
        time_since_shake: 0,
        last_time: 0.0,
    };
    st.recolor();

    for i in 0..count {
        st.px[i] = frand(extx as f64) as f32 + xmin;
        st.py[i] = frand(exty as f64) as f32 + ymin;
        st.vx[i] = frand(0.2) as f32 - 0.1;
        st.vy[i] = frand(0.2) as f32 - 0.1;
        st.r[i] = if random_sizes_p {
            (0.2 + frand(0.8) as f32) * max_radius
        } else {
            max_radius
        };
        // Mass goes as the cube of the radius, so a big ball shrugs off a
        // small one.
        st.m[i] = st.r[i].powi(3) * std::f32::consts::PI * 1.3333;
    }
    st.opx.copy_from_slice(&st.px);
    st.opy.copy_from_slice(&st.py);

    d.clear_window();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.repaint_balls(d);
        self.update_balls();
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        // Upstream polls the window geometry every frame rather than waiting
        // for an event, so it follows a resize as it happens.
        self.xmax = self.xmin + width as f32;
        self.ymax = self.ymin + height as f32;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        match *event {
            XEvent::MotionNotify { x, y } => {
                self.mouse_x = x as f32;
                self.mouse_y = y as f32;
                false
            }
            XEvent::ButtonPress { x, y, .. } => {
                self.mouse_x = x as f32;
                self.mouse_y = y as f32;
                if self.mouse_ball.is_some() {
                    // A second down-click drops the ball.
                    self.mouse_ball = None;
                    return true;
                }
                // Look for a click directly inside a ball first, then widen
                // the search until something nearby turns up.
                let max = self.max_radius * 4.0;
                let step = max / 10.0;
                let mut r2 = step;
                while r2 < max {
                    for i in 0..self.count {
                        let dx = self.px[i] - x as f32;
                        let dy = self.py[i] - y as f32;
                        let d = dx * dx + dy * dy;
                        let r = self.r[i].max(r2);
                        if d < r * r {
                            self.mouse_ball = Some(i);
                            return true;
                        }
                    }
                    r2 += step;
                }
                true
            }
            XEvent::ButtonRelease { .. } => {
                self.mouse_ball = None;
                true
            }
            _ => false,
        }
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: yellow",
    ".textColor: yellow",
    "*mouseForeground: white",
    "*delay: 10000",
    "*count: 300",
    "*size: 25",
    "*random: True",
    "*gravity: 0.01",
    "*wind: 0.00",
    "*elasticity: 0.97",
    "*timeScale: 1.0",
    "*shake: True",
    "*shakeThreshold: 0.015",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Number of balls", 1.0, 3000.0, 10.0, 0, "300"),
    Opt::slider("size", "Ball size", 3.0, 200.0, 1.0, 0, "25"),
    Opt::slider("gravity", "Gravity", 0.0, 0.1, 0.005, 3, "0.01"),
    Opt::slider("wind", "Wind", 0.0, 0.1, 0.005, 3, "0.00"),
    Opt::slider("elasticity", "Friction", 0.2, 1.0, 0.01, 2, "0.97"),
    Opt::boolean("random", "Various ball sizes", "true"),
    Opt::boolean("shake", "Shake box", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "fluidballs",
    label: "Fluid Balls",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Peter Birtles and Jamie Zawinski",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=5Iz9V-vOrxA"),
        blurb: "A particle system of bouncing balls. Gravity moves around to shake the box.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
