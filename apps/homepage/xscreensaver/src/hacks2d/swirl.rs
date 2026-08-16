//! Port of `hacks/swirl.c`.
//!
//! ```text
//! swirl --- swirly color-cycling patterns.
//!
//! Copyright (c) 1994 M.Dobie <mrd@ecs.soton.ac.uk>
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! 13-May-1997: jwz@jwz.org: turned into a standalone program.
//! 21-Apr-1995: improved startup time for TrueColour displays
//!              (limited to 16bpp to save memory) S.Early <sde1000@cam.ac.uk>
//! 09-Jan-1995: fixed colour maps (more colourful) and the image now spirals
//!              outwards from the centre with a fixed number of points drawn
//!              every iteration. Thanks to M.Dobie <mrd@ecs.soton.ac.uk>.
//! 1994:        written.   Copyright (c) 1994 M.Dobie <mrd@ecs.soton.ac.uk>
//!              based on original code by R.Taylor
//! ```
//!
//! Half a dozen knots are dropped on the screen, and the colour of every pixel
//! is decided by adding up what each knot has to say about it. A knot has a
//! mass, which can be negative, and one of four characters: one shades by plain
//! distance, so it makes a disc; one shades by angle with a ripple laid over it,
//! so it makes a pinwheel; one shades by the sine of twice the angle, so it
//! makes a four-pointed star; and one adds angle to distance, so it makes a
//! spiral. Sum a few of those, wrap the total into the palette, and the result
//! is the interference pattern between them.
//!
//! Since every pixel is independent, the picture is drawn in passes of doubling
//! detail rather than in one go. The first pass fills the screen in blocks a
//! hundred and twenty-eight pixels square, the next in blocks half that, and so
//! on down to single pixels, so the whole pattern is visible almost at once and
//! then sharpens. Each pass walks a square spiral outwards from the centre,
//! which is why the sharpening arrives as a growing square, and it is walked a
//! hundred blocks a frame. Off-screen stretches of the spiral are skipped rather
//! than plotted, and two consecutive skipped stretches mean the pass is done.
//!
//! Three times in ten it draws in two planes instead: each knot is given a
//! second, different character, the two are computed alternately, and the block
//! at each step is split into four quarters that alternate between them. That
//! is the mode with the fine chequered texture, and it stops one pass short of
//! single pixels because the quarters are already single pixels.
//!
//! Two notes on the port. Upstream's fifth knot character, which shades by the
//! cosine of distance and makes concentric rings, is unreachable: it is only
//! switched on by a knot-type option that nothing sets, and the catch-all that
//! everything does use lists the other four. And its colour cycling needs a
//! writable colormap, which no display has had for twenty years, so upstream
//! does not animate the palette either.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::Pixel;
use crate::runtime::xlockmore::{ColorScheme, MAXRAND, ModeInfo, lrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs};

/// The largest mass a knot can have.
const MASS: i32 = 4;
/// The coarsest and finest block sizes, as shift counts.
const MIN_RES: i32 = 7;
const MAX_RES: i32 = 1;
/// The chance, as a percentage, of drawing in two planes.
const TWO_PLANE_PCNT: i32 = 30;
/// Frames to sit on a finished picture before starting a new one.
const RESTART: i32 = 2500;
/// Blocks to draw per frame.
const BATCH_DRAW: i32 = 100;

/// What a knot has to say about the pixels around it.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum KnotType {
    #[default]
    None,
    /// Shades by distance: a disc.
    Orbit,
    /// Shades by angle, with a ripple: a pinwheel.
    Wheel,
    /// Shades by the sine of twice the angle: a four-pointed star.
    Ray,
    /// Angle plus distance: a spiral.
    Hook,
}

#[derive(Clone, Copy, Default)]
struct Knot {
    x: i32,
    y: i32,
    /// Mass, which can be negative, in which case the knot subtracts.
    m: i32,
    t: KnotType,
    /// The character in the second plane, when there is one.
    t2: KnotType,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Right,
    Down,
    Left,
    Up,
}

/// `random_no`: a random integer between zero and n inclusive.
fn random_no(n: i32) -> i32 {
    ((n + 1) as f64 * lrand() as f64 / MAXRAND) as i32
}

struct Swirl {
    mi: ModeInfo,
    width: i32,
    height: i32,

    knots: Vec<Knot>,
    resolution: i32,
    max_resolution: i32,
    /// The current block size, in pixels.
    r: i32,
    two_plane: bool,
    first_plane: bool,
    start_again: i32,

    x: i32,
    y: i32,
    direction: Dir,
    dir_todo: i32,
    dir_done: i32,
    started: bool,
    drawing: bool,
    off_screen: bool,

    colours: i32,
    dcolours: i32,
}

impl Swirl {
    fn new(d: &mut Dpy) -> Self {
        let mi = ModeInfo::new(d, ColorScheme::Smooth);
        let mut st = Self {
            width: mi.width,
            height: mi.height,
            mi,
            knots: Vec::new(),
            resolution: MIN_RES + 1,
            max_resolution: MAX_RES,
            r: 1,
            two_plane: false,
            first_plane: false,
            start_again: -1,
            x: 0,
            y: 0,
            direction: Dir::Right,
            dir_todo: 1,
            dir_done: 0,
            started: false,
            drawing: false,
            off_screen: false,
            colours: 0,
            dcolours: 0,
        };
        st.init(d);
        st
    }

    /// `init_swirl`: a new set of knots, back to the coarsest blocks.
    fn init(&mut self, d: &mut Dpy) {
        self.width = self.mi.width;
        self.height = self.mi.height;
        self.max_resolution = MAX_RES;
        self.start_again = -1;

        self.mi.clear_window(d);

        self.colours = self.mi.npixels().max(1);
        self.dcolours = self.colours;

        // Resolution starts off chunky.
        self.resolution = MIN_RES + 1;
        self.r = 1 << (self.resolution - 1);

        let count = self.mi.count;
        let n_knots = random_no(count / 2) + count + 1;

        // Use two-plane mode occasionally.
        self.two_plane = random_no(100) <= TWO_PLANE_PCNT;
        if self.two_plane {
            self.first_plane = true;
            self.max_resolution = 2;
        }

        self.create_knots(n_knots.max(0) as usize);

        self.started = true;
        self.drawing = false;
    }

    /// `create_knots`. The catch-all knot type upstream always uses leaves out
    /// the rings, so only four of the five characters ever appear.
    fn create_knots(&mut self, n: usize) {
        const AVAILABLE: [KnotType; 5] = [
            KnotType::Orbit,
            KnotType::Wheel,
            // Picasso is switched off by the catch-all.
            KnotType::None,
            KnotType::Ray,
            KnotType::Hook,
        ];

        self.knots = Vec::with_capacity(n);
        for _ in 0..n {
            let mut k = Knot {
                x: random_no(self.width),
                y: random_no(self.height),
                m: random_no(MASS) + 1,
                ..Knot::default()
            };
            // Can be negative.
            if random_no(100) > 50 {
                k.m *= -1;
            }
            while k.t == KnotType::None {
                k.t = AVAILABLE[random_no(4) as usize];
            }
            if self.two_plane {
                while k.t2 == KnotType::None || k.t2 == k.t {
                    k.t2 = AVAILABLE[random_no(4) as usize];
                }
            }
            self.knots.push(k);
        }
    }

    /// `do_point`: what colour the knots between them make of one pixel.
    fn do_point(&mut self, i: i32, j: i32) -> Pixel {
        let dcolours = self.dcolours.max(2);
        let qcolours = dcolours / 4;
        // The colour step around a circle.
        let rads = dcolours as f64 / (2.0 * std::f64::consts::PI);
        let mut value: i32 = 0;

        for knot in &self.knots {
            let dx = (i - knot.x) as f64;
            let dy = (j - knot.y) as f64;
            let t = if self.two_plane {
                if self.first_plane { knot.t } else { knot.t2 }
            } else {
                knot.t
            };
            let dist = (dx * dx + dy * dy).sqrt();
            let mut add = 0;

            if dist > 0.1 {
                add = match t {
                    KnotType::Orbit => {
                        (dcolours as f64 / (1.0 + 0.01 * knot.m.abs() as f64 * dist)) as i32
                    }
                    KnotType::Wheel => {
                        // Avoiding a domain error at the knot itself.
                        let theta = if dy == 0.0 && dx == 0.0 {
                            1.0
                        } else {
                            (dy.atan2(dx) + std::f64::consts::PI) / std::f64::consts::PI
                        };
                        let ripple = (0.1 * knot.m as f64 * dist).sin()
                            * qcolours as f64
                            * (-0.01 * dist).exp();
                        if theta < 1.0 {
                            (dcolours as f64 * theta + ripple) as i32
                        } else {
                            (dcolours as f64 * (theta - 1.0) + ripple) as i32
                        }
                    }
                    KnotType::Ray => {
                        if dy == 0.0 && dx == 0.0 {
                            0
                        } else {
                            (dcolours as f64 * (2.0 * dy.atan2(dx)).sin().abs()) as i32
                        }
                    }
                    KnotType::Hook => {
                        let spiral = 0.05 * (knot.m.abs() - 1) as f64 * dist;
                        if dy == 0.0 && dx == 0.0 {
                            spiral as i32
                        } else {
                            (rads * dy.atan2(dx) + spiral) as i32
                        }
                    }
                    KnotType::None => 0,
                };
            }

            // A positive mass adds its contribution, a negative one takes it
            // off.
            if knot.m > 0 {
                value = value.wrapping_add(add);
            } else {
                value = value.wrapping_sub(add);
            }
        }

        self.first_plane = !self.first_plane;

        // Fold the total into the palette, negatives from the far end.
        value = if value >= 0 {
            (value % dcolours) + 2
        } else {
            dcolours - (value.abs() % (dcolours - 1))
        };
        value %= self.colours.max(1);
        self.mi.pixel(value.max(0) as usize)
    }

    /// A square block of one colour.
    fn draw_block(&self, d: &mut Dpy, x: i32, y: i32, s: i32, v: Pixel) {
        for a in 0..s {
            for b in 0..s {
                d.win().put_pixel(x + b, y + a, v);
            }
        }
    }

    /// `draw_point`: the block at the current step of the spiral. In two-plane
    /// mode it is four quarter blocks, alternating planes.
    fn draw_point(&mut self, d: &mut Dpy) {
        let (x, y, r) = (self.x, self.y, self.r);
        if x < 0 || x > self.width - r || y < 0 || y > self.height - r {
            return;
        }
        if self.two_plane {
            let r2 = r / 2;
            let v = self.do_point(x, y);
            self.draw_block(d, x, y, r2, v);
            let v = self.do_point(x + r2, y);
            self.draw_block(d, x + r2, y, r2, v);
            let v = self.do_point(x + r2, y + r2);
            self.draw_block(d, x + r2, y + r2, r2, v);
            let v = self.do_point(x, y + r2);
            self.draw_block(d, x, y + r2, r2, v);
        } else {
            let v = self.do_point(x, y);
            self.draw_block(d, x, y, r, v);
        }
    }

    /// `next_point`: one step along the square spiral, skipping whole
    /// stretches that lie off the screen. Two skipped stretches in a row and
    /// the pass is finished.
    fn next_point(&mut self) {
        if self.dir_done < self.dir_todo {
            match self.direction {
                Dir::Right => self.x += self.r,
                Dir::Down => self.y += self.r,
                Dir::Left => self.x -= self.r,
                Dir::Up => self.y -= self.r,
            }
            self.dir_done += 1;
            return;
        }

        self.dir_done = 0;
        let off = match self.direction {
            Dir::Right => {
                self.direction = Dir::Down;
                let off = self.x > self.width - self.r;
                if off {
                    self.y += self.dir_todo * self.r;
                }
                off
            }
            Dir::Down => {
                self.direction = Dir::Left;
                self.dir_todo += 1;
                let off = self.y > self.height - self.r;
                if off {
                    self.x -= self.dir_todo * self.r;
                }
                off
            }
            Dir::Left => {
                self.direction = Dir::Up;
                let off = self.x < 0;
                if off {
                    self.y -= self.dir_todo * self.r;
                }
                off
            }
            Dir::Up => {
                self.direction = Dir::Right;
                self.dir_todo += 1;
                let off = self.y < 0;
                if off {
                    self.x += self.dir_todo * self.r;
                }
                off
            }
        };
        if off {
            self.dir_done = self.dir_todo;
            if self.off_screen {
                self.drawing = false;
            }
        }
        self.off_screen = off;
    }
}

impl Screenhack for Swirl {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if !self.started {
            return self.mi.delay;
        }

        if self.drawing {
            let mut todo = BATCH_DRAW;
            while todo > 0 && self.drawing {
                self.draw_point(d);
                self.next_point();
                todo -= 1;
            }
        } else if self.resolution > self.max_resolution {
            // Move to a finer pass, starting from the middle again.
            self.resolution -= 1;
            self.r = 1 << (self.resolution - 1);
            self.drawing = true;
            self.x = (self.width - self.r) / 2;
            self.y = (self.height - self.r) / 2;
            self.direction = Dir::Right;
            self.dir_todo = 1;
            self.dir_done = 0;
        } else if self.start_again == -1 {
            self.start_again = RESTART;
        } else if self.start_again == 0 {
            self.start_again = -1;
            // A new palette to go with the new pattern.
            let ncolors = self.mi.npixels().max(1) as usize;
            self.mi.remake_colors(ColorScheme::Smooth, ncolors);
            self.init(d);
        } else {
            self.start_again -= 1;
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.init(d);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(Swirl::new(d))
}

const DEFAULTS: &[&str] = &[
    "*count: 5",
    "*delay: 10000",
    "*ncolors: 200",
    "*fpsSolid: true",
    "*ignoreRotation: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Count", 0.0, 20.0, 1.0, 0, "5"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "swirl",
    label: "Swirl",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "M. Dobie and R. Taylor",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=o_VRQxPCB7w"),
        blurb: "Flowing, swirly patterns.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
