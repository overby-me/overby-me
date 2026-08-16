//! Port of `hacks/helix.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2008 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Algorithm from a Mac program by Chris Tate, written in 1988 or so.
//!
//! 18-Sep-97: Johannes Keukelaar (johannes@nada.kth.se): Improved screen
//!            eraser.
//! 10-May-97: merged ellipse code by Dan Stromberg <strombrg@nis.acs.uci.edu>
//!            as found in xlockmore 4.03a10.
//! 1992:      jwz created.
//!
//! 25 April 2002: Matthew Strait <straitm@mathcs.carleton.edu> added
//! -subdelay option so the drawing process can be watched
//! ```
//!
//! String art. Two figures alternate: a helix, which chases a point around one
//! ellipse while chasing a second point around another and joins them with a
//! line, and a trig figure, which does the same with two points on the window's
//! own bounding ellipse. Both pick their frequency ratios so the walk closes
//! only after going all the way round, which is what fills the figure in. When
//! it closes, the picture lingers and is then wiped.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::hsv_to_rgb;
use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XColor, frand, random,
};

/// How long a wipe is given per frame while one is running.
const ERASE_DELAY: u32 = 10000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawState {
    Helix,
    DrawHelix,
    Trig,
    DrawTrig,
    Linger,
    Erase,
}

struct Helix {
    dstate: DrawState,
    sins: [f64; 360],
    coss: [f64; 360],
    gc: Gc,
    default_fg: Pixel,
    /// Seconds the finished figure lingers on screen.
    sleep_time: u32,
    subdelay: u32,
    eraser: Option<Eraser>,
    mono: bool,
    width: i32,
    height: i32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    angle: i32,
    i: i32,
    radius1: i32,
    radius2: i32,
    d_angle: i32,
    factor1: i32,
    factor2: i32,
    factor3: i32,
    factor4: i32,
    d_angle_offset: i32,
    offset: i32,
    dir: i32,
    density: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    let mut sins = [0.0; 360];
    let mut coss = [0.0; 360];
    for i in 0..360 {
        sins[i] = ((i as f64) / 180.0 * std::f64::consts::PI).sin();
        coss[i] = ((i as f64) / 180.0 * std::f64::consts::PI).cos();
    }

    Box::new(Helix {
        dstate: if random() & 1 == 1 {
            DrawState::Helix
        } else {
            DrawState::Trig
        },
        sins,
        coss,
        gc: Gc::new(fg, d.res.pixel("background")),
        default_fg: fg,
        sleep_time: d.res.int("delay").max(0) as u32,
        subdelay: d.res.int("subdelay").max(0) as u32,
        eraser: None,
        mono: d.mono_p,
        width: d.width(),
        height: d.height(),
        x1: 0,
        y1: 0,
        x2: 0,
        y2: 0,
        angle: 0,
        i: 0,
        radius1: 0,
        radius2: 0,
        d_angle: 0,
        factor1: 2,
        factor2: 2,
        factor3: 2,
        factor4: 2,
        d_angle_offset: 0,
        offset: 0,
        dir: 1,
        density: 16,
    })
}

fn gcd(mut a: i32, mut b: i32) -> i32 {
    while b > 0 {
        let tmp = a % b;
        a = b;
        b = tmp;
    }
    a.abs()
}

/// `pmod(x, y)`: a modulo that never comes back negative.
fn pmod(x: i32, y: i32) -> usize {
    x.rem_euclid(y) as usize
}

/// `random_factor()`: mostly one or two, occasionally three, either sign.
fn random_factor() -> i32 {
    let m = if !random().is_multiple_of(7) {
        (random() & 1) as i32 + 1
    } else {
        3
    };
    m * (((random() & 1) as i32 * 2) - 1)
}

impl Helix {
    /// Pick a colour for a fresh figure and clear the screen for it.
    fn new_figure(&mut self, d: &mut Dpy) {
        if self.mono {
            let fg = self.default_fg;
            self.gc.set_foreground(fg);
        } else {
            let (r, g, b) = hsv_to_rgb((random() % 360) as i32, frand(1.0), frand(0.5) + 0.5);
            self.gc.set_foreground(XColor::from_rgb16(r, g, b).pixel);
        }
        d.clear_window();
    }

    fn random_helix(&mut self, d: &mut Dpy) {
        let radius = self.width.min(self.height) / 2;

        self.i = 0;
        self.d_angle = 0;
        self.factor1 = 2;
        self.factor2 = 2;
        self.factor3 = 2;
        self.factor4 = 2;

        let divisor = (frand(3.0) + 1.0) * (((random() & 1) as f64 * 2.0) - 1.0);

        if random() & 1 == 0 {
            self.radius1 = radius;
            self.radius2 = (radius as f64 / divisor) as i32;
        } else {
            self.radius2 = radius;
            self.radius1 = (radius as f64 / divisor) as i32;
        }

        // Keep rolling until the step and a full turn share no factor, so the
        // walk only closes after it has been all the way round.
        while gcd(360, self.d_angle) >= 2 {
            self.d_angle = (random() % 360) as i32;
        }

        while gcd(
            gcd(gcd(self.factor1, self.factor2), self.factor3),
            self.factor4,
        ) != 1
        {
            self.factor1 = random_factor();
            self.factor2 = random_factor();
            self.factor3 = random_factor();
            self.factor4 = random_factor();
        }

        self.new_figure(d);
    }

    fn random_trig(&mut self, d: &mut Dpy) {
        self.d_angle = 0;
        self.factor1 = (random() % 8) as i32 + 1;
        loop {
            self.factor2 = (random() % 8) as i32 + 1;
            if self.factor1 != self.factor2 {
                break;
            }
        }

        self.dir = if random() & 1 == 1 { 1 } else { -1 };
        self.d_angle_offset = (random() % 360) as i32;
        self.offset = (((random() % ((360 / 4) - 1)) as i32) + 1) / 4;
        // Higher density, higher angles.
        self.density = 1 << ((random() % 4) + 4);

        self.new_figure(d);
    }

    fn helix(&mut self, d: &mut Dpy) {
        let xmid = self.width / 2;
        let ymid = self.height / 2;
        let limit = 1 + (360 / gcd(360, self.d_angle));

        if self.i == 0 {
            self.x1 = xmid;
            self.y1 = ymid + self.radius2;
            self.x2 = xmid;
            self.y2 = ymid + self.radius1;
            self.angle = 0;
        }

        self.x1 =
            xmid + (self.radius1 as f64 * self.sins[pmod(self.angle * self.factor1, 360)]) as i32;
        self.y1 =
            ymid + (self.radius2 as f64 * self.coss[pmod(self.angle * self.factor2, 360)]) as i32;
        let (x1, y1, x2, y2) = (self.x1, self.y1, self.x2, self.y2);
        d.win().draw_line(&self.gc, x1, y1, x2, y2);

        self.x2 =
            xmid + (self.radius2 as f64 * self.sins[pmod(self.angle * self.factor3, 360)]) as i32;
        self.y2 =
            ymid + (self.radius1 as f64 * self.coss[pmod(self.angle * self.factor4, 360)]) as i32;
        let (x1, y1, x2, y2) = (self.x1, self.y1, self.x2, self.y2);
        d.win().draw_line(&self.gc, x1, y1, x2, y2);

        self.angle += self.d_angle;
        self.i += 1;

        if self.i >= limit {
            self.dstate = DrawState::Linger;
        }
    }

    fn trig(&mut self, d: &mut Dpy) {
        let xmid = self.width / 2;
        let ymid = self.height / 2;

        let angle = self.d_angle + self.d_angle_offset;
        self.x1 = (self.sins[pmod(angle * self.factor1, 360)] * xmid as f64) as i32 + xmid;
        self.y1 = (self.coss[pmod(angle * self.factor1, 360)] * ymid as f64) as i32 + ymid;
        self.x2 =
            (self.sins[pmod(angle * self.factor2 + self.offset, 360)] * xmid as f64) as i32 + xmid;
        self.y2 =
            (self.coss[pmod(angle * self.factor2 + self.offset, 360)] * ymid as f64) as i32 + ymid;
        let (x1, y1, x2, y2) = (self.x1, self.y1, self.x2, self.y2);
        d.win().draw_line(&self.gc, x1, y1, x2, y2);

        // Do not want it getting stuck; would not need if floating point.
        let step = (360 / (2 * self.density * self.factor1 * self.factor2)).max(1);
        self.d_angle += self.dir * step;

        if !(-360..=360).contains(&self.d_angle) {
            self.dstate = DrawState::Linger;
        }
    }
}

impl Screenhack for Helix {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.eraser.is_some() {
            self.eraser = erase_window(d, self.eraser.take());
            return if self.eraser.is_some() {
                ERASE_DELAY
            } else {
                self.subdelay
            };
        }

        match self.dstate {
            DrawState::Linger => {
                self.dstate = DrawState::Erase;
                return self.sleep_time.saturating_mul(1_000_000);
            }
            DrawState::Erase => {
                self.eraser = erase_window(d, self.eraser.take());
                self.dstate = if random() & 1 == 1 {
                    DrawState::Helix
                } else {
                    DrawState::Trig
                };
                return ERASE_DELAY;
            }
            DrawState::DrawHelix => {
                for _ in 0..10 {
                    self.helix(d);
                    if self.dstate != DrawState::DrawHelix {
                        break;
                    }
                }
            }
            DrawState::DrawTrig => {
                for _ in 0..5 {
                    self.trig(d);
                    if self.dstate != DrawState::DrawTrig {
                        break;
                    }
                }
            }
            DrawState::Helix => {
                self.random_helix(d);
                self.dstate = DrawState::DrawHelix;
            }
            DrawState::Trig => {
                self.random_trig(d);
                self.dstate = DrawState::DrawTrig;
            }
        }

        self.subdelay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*delay: 5",
    "*subdelay: 20000",
];

const OPTS: &[Opt] = &[
    Opt::slider("subdelay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("delay", "Linger", 1.0, 60.0, 1.0, 0, "5"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "helix",
    label: "Helix",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=H-mMnadnPSs"),
        blurb: "Spirally string-art-ish patterns.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
