//! Port of `hacks/compass.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1999-2018 Jamie Zawinski <jwz@jwz.org>
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
//! An aircraft compass with three discs stacked on one spindle, each spinning
//! independently: the lettered card, a thick needle and a thin one. Only the
//! bezel and its eight index marks stay put, so nothing on the instrument ever
//! agrees with anything else.
//!
//! Each disc carries an angle, a speed and an acceleration, and rolls by adding
//! them up. The acceleration flips sign when the speed passes a limit, so a disc
//! swings back and forth rather than winding up; on top of that it flips at
//! random about once in a hundred and twenty frames, and once in two hundred it
//! is scaled by a fifth up or down. Since the acceleration is an integer, the
//! scaling down eventually reaches zero and that disc coasts.
//!
//! There is no font here, which is what makes this a 2D hack rather than a
//! typeset one. The letters and the compass numbers are polylines in polar
//! coordinates: each stroke is a list of "this fraction of the radius, this many
//! radians round from the card's heading", so the whole card is drawn from one
//! table and rotates by adding a single angle to every entry.
//!
//! Upstream double-buffers through a pixmap when the display has no
//! double-buffer extension, and forces that off under jwxyz, where the platform
//! already does it. This port takes the jwxyz path: the host blits the whole
//! framebuffer once per frame, so there is nothing to flicker, and the drawing
//! is identical either way.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint, XSegment, random,
};

/// `RAND(n)`.
fn rand(n: i32) -> i32 {
    ((random() & 0x7fff_ffff) % n as u32) as i32
}

/// `RANDSIGN()`.
fn randsign() -> i32 {
    if (random() & 1) != 0 { 1 } else { -1 }
}

/// One stroke of the compass card: how far round from the card's heading, and
/// then a list of (fraction of the radius, radians offset) for each point.
type Stroke = (f64, &'static [(f64, f64)]);

/// The whole card. Upstream writes these out point by point; the shape of every
/// one is the same, so they collapse to a table.
const LETTERS: &[Stroke] = &[
    // W
    (
        0.0,
        &[
            (0.8, -0.07),
            (0.7, -0.05),
            (0.78, 0.0),
            (0.7, 0.05),
            (0.8, 0.07),
        ],
    ),
    // 30 (1)
    (
        0.08333,
        &[
            (0.78, -0.13),
            (0.8, -0.08),
            (0.78, -0.03),
            (0.76, -0.03),
            (0.75, -0.08),
            (0.74, -0.03),
            (0.72, -0.03),
            (0.7, -0.08),
            (0.72, -0.13),
        ],
    ),
    // 30 (2), which closes back on its own first point
    (
        0.08333,
        &[
            (0.78, 0.03),
            (0.8, 0.08),
            (0.78, 0.13),
            (0.72, 0.13),
            (0.7, 0.08),
            (0.72, 0.03),
            (0.78, 0.03),
        ],
    ),
    // 33 (1)
    (
        0.16666,
        &[
            (0.78, -0.13),
            (0.8, -0.08),
            (0.78, -0.03),
            (0.76, -0.03),
            (0.75, -0.08),
            (0.74, -0.03),
            (0.72, -0.03),
            (0.7, -0.08),
            (0.72, -0.13),
        ],
    ),
    // 33 (2)
    (
        0.16666,
        &[
            (0.78, 0.03),
            (0.8, 0.08),
            (0.78, 0.13),
            (0.76, 0.13),
            (0.75, 0.08),
            (0.74, 0.13),
            (0.72, 0.13),
            (0.7, 0.08),
            (0.72, 0.03),
        ],
    ),
    // N
    (
        0.25,
        &[(0.7, -0.05), (0.8, -0.05), (0.7, 0.05), (0.8, 0.05)],
    ),
    // 3
    (
        0.33333,
        &[
            (0.78, -0.05),
            (0.8, 0.0),
            (0.78, 0.05),
            (0.76, 0.05),
            (0.75, 0.0),
            (0.74, 0.05),
            (0.72, 0.05),
            (0.7, 0.0),
            (0.72, -0.05),
        ],
    ),
    // 6
    (
        0.41666,
        &[
            (0.78, 0.05),
            (0.8, 0.0),
            (0.78, -0.05),
            (0.72, -0.05),
            (0.7, 0.0),
            (0.72, 0.05),
            (0.74, 0.05),
            (0.76, 0.0),
            (0.74, -0.05),
        ],
    ),
    // E
    (
        0.5,
        &[
            (0.8, 0.05),
            (0.8, -0.05),
            (0.75, -0.05),
            (0.75, 0.025),
            (0.75, -0.05),
            (0.7, -0.05),
            (0.7, 0.05),
        ],
    ),
    // 12 (1)
    (0.58333, &[(0.77, -0.06), (0.8, -0.03), (0.7, -0.03)]),
    // 12 (2)
    (
        0.58333,
        &[
            (0.78, 0.02),
            (0.8, 0.07),
            (0.78, 0.11),
            (0.76, 0.11),
            (0.74, 0.02),
            (0.71, 0.03),
            (0.7, 0.03),
            (0.7, 0.13),
        ],
    ),
    // 15 (1)
    (0.66666, &[(0.77, -0.06), (0.8, -0.03), (0.7, -0.03)]),
    // 15 (2)
    (
        0.66666,
        &[
            (0.8, 0.11),
            (0.8, 0.02),
            (0.76, 0.02),
            (0.77, 0.06),
            (0.76, 0.10),
            (0.73, 0.11),
            (0.72, 0.10),
            (0.7, 0.06),
            (0.72, 0.02),
        ],
    ),
    // S
    (
        0.75,
        &[
            (0.78, 0.05),
            (0.8, 0.0),
            (0.78, -0.05),
            (0.76, -0.05),
            (0.74, 0.05),
            (0.72, 0.05),
            (0.7, 0.0),
            (0.72, -0.05),
        ],
    ),
    // 21 (1)
    (
        0.83333,
        &[
            (0.78, -0.13),
            (0.8, -0.08),
            (0.78, -0.03),
            (0.76, -0.03),
            (0.74, -0.12),
            (0.71, -0.13),
            (0.7, -0.13),
            (0.7, -0.02),
        ],
    ),
    // 21 (2)
    (0.83333, &[(0.77, 0.03), (0.8, 0.06), (0.7, 0.06)]),
    // 24 (1)
    (
        0.91666,
        &[
            (0.78, -0.13),
            (0.8, -0.08),
            (0.78, -0.03),
            (0.76, -0.03),
            (0.74, -0.12),
            (0.71, -0.13),
            (0.7, -0.13),
            (0.7, -0.02),
        ],
    ),
    // 24 (2)
    (
        0.91666,
        &[(0.69, 0.09), (0.8, 0.09), (0.72, 0.01), (0.72, 0.13)],
    ),
];

/// Which of the three things stacked on the spindle a disc is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Face {
    Ticks,
    ThickArrow,
    ThinArrow,
}

struct Disc {
    /// 0 to 360*64, and negative once it has passed through zero.
    theta: i32,
    velocity: i32,
    acceleration: i32,
    limit: i32,
    gc: Gc,
    face: Face,
}

struct Compass {
    delay: u32,
    discs: [Disc; 3],
    x: i32,
    y: i32,
    size: i32,
    size2: i32,
    ptr_gc: Gc,
    erase_gc: Gc,
    width: i32,
    height: i32,
}

fn init_spin(gc: Gc, face: Face) -> Disc {
    Disc {
        limit: 5 * 64,
        theta: rand(360 * 64),
        velocity: rand(16) * randsign(),
        acceleration: rand(16) * randsign(),
        gc,
        face,
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let delay = d.res.int("delay").max(0) as u32;
    let background = d.res.pixel("background");
    let foreground = d.res.pixel("foreground");

    let (width, height) = (d.width(), d.height());
    let mut size2 = width.min(height);
    if width > height * 5 || height > width * 5 {
        size2 = width.max(height); // Goofy aspect ratio.
    }
    {
        let mut max = 600;
        if width > 2560 || height > 2560 {
            max *= 2; // Retina displays.
        }
        if size2 > max {
            size2 = max;
        }
    }
    let size = ((size2 / 2) as f64 * 0.8) as i32;

    let mut card_gc = Gc::new(foreground, background);
    card_gc.set_line_width(2.max(size / 60));
    let discs = [
        init_spin(card_gc, Face::Ticks),
        {
            let mut gc = Gc::new(d.res.pixel("arrow2Foreground"), background);
            gc.set_line_width(4.max(size / 30));
            init_spin(gc, Face::ThickArrow)
        },
        {
            let mut gc = Gc::new(d.res.pixel("arrow1Foreground"), background);
            gc.set_line_width(4.max(size / 30));
            init_spin(gc, Face::ThinArrow)
        },
    ];

    Box::new(Compass {
        delay,
        discs,
        x: width / 2,
        y: height / 2,
        size,
        size2,
        ptr_gc: Gc::new(d.res.pixel("pointerForeground"), background),
        erase_gc: Gc::new(background, background),
        width,
        height,
    })
}

impl Compass {
    /// The card's heading in radians.
    fn heading(disc: &Disc) -> f64 {
        std::f64::consts::TAU * (disc.theta as f64 / (360.0 * 64.0))
    }

    /// A point at `rf` of the radius, `ao` radians round from `th`.
    fn polar(&self, radius: f64, th: f64, rf: f64, ao: f64) -> XPoint {
        XPoint {
            x: self.x + (radius * rf * (th + ao).cos()) as i32,
            y: self.y + (radius * rf * (th + ao).sin()) as i32,
        }
    }

    fn draw_letters(&self, d: &mut Dpy, k: usize, radius: i32) {
        let th2 = Self::heading(&self.discs[k]);
        let r = radius as f64;
        for (fraction, stroke) in LETTERS {
            let th = th2 + std::f64::consts::TAU * fraction;
            let points: Vec<XPoint> = stroke
                .iter()
                .map(|(rf, ao)| self.polar(r, th, *rf, *ao))
                .collect();
            d.win().draw_lines(&self.discs[k].gc, &points);
        }
    }

    fn draw_ticks(&self, d: &mut Dpy, k: usize, radius: i32) {
        let tick = std::f64::consts::TAU / 72.0;
        let th2 = Self::heading(&self.discs[k]);
        let mut segs = [XSegment::default(); 72];
        for (i, seg) in segs.iter_mut().enumerate() {
            let radius2 = if i % 6 != 0 {
                radius - radius / 16
            } else {
                radius - radius / 8
            };
            let th = (i as f64 * tick) + th2;
            *seg = XSegment {
                x1: self.x + (radius as f64 * th.cos()) as i32,
                y1: self.y + (radius as f64 * th.sin()) as i32,
                x2: self.x + (radius2 as f64 * th.cos()) as i32,
                y2: self.y + (radius2 as f64 * th.sin()) as i32,
            };
        }
        d.win().draw_segments(&self.discs[k].gc, &segs);

        self.draw_letters(d, k, radius);
    }

    fn draw_thin_arrow(&self, d: &mut Dpy, k: usize, radius: i32) {
        let tick = (std::f64::consts::TAU / 72.0) * 2.0;
        let radius = (radius as f64 * 0.9) as i32;
        let radius2 = radius - (radius / 8) * 3;
        let th = Self::heading(&self.discs[k]);
        let (r, r2) = (radius as f64, radius2 as f64);

        let points = [
            self.polar(r, th, 1.0, 0.0),    // Tip.
            self.polar(r2, th, 1.0, -tick), // Tip left.
            self.polar(r2, th, 1.0, tick),  // Tip right.
        ];

        d.win().draw_line(
            &self.discs[k].gc,
            self.x + (r2 * th.cos()) as i32,
            self.y + (r2 * th.sin()) as i32,
            self.x + (-r * th.cos()) as i32,
            self.y + (-r * th.sin()) as i32,
        );
        d.win().fill_polygon(&self.discs[k].gc, &points);
    }

    fn draw_thick_arrow(&self, d: &mut Dpy, k: usize, radius: i32) {
        let tick = (std::f64::consts::TAU / 72.0) * 2.0;
        let radius = (radius as f64 * 0.9) as i32;
        let radius2 = radius - (radius / 8) * 3;
        let radius3 = radius - (radius / 8) * 2;
        let th = Self::heading(&self.discs[k]);
        let (r, r2, r3) = (radius as f64, radius2 as f64, radius3 as f64);

        let head = [
            self.polar(r, th, 1.0, 0.0),    // Tip.
            self.polar(r2, th, 1.0, -tick), // Tip left.
            self.polar(r2, th, 1.0, tick),  // Tip right.
            self.polar(r, th, 1.0, 0.0),
        ];
        d.win().draw_lines(&self.discs[k].gc, &head);

        // The negative radii put these on the far side of the spindle.
        let body = [
            self.polar(r2, th, 1.0, -tick / 2.0),  // Top left.
            self.polar(r2, th, -1.0, tick / 2.0),  // Bottom left.
            self.polar(r3, th, -1.0, 0.0),         // Bottom.
            self.polar(r, th, -1.0, 0.0),          // Bottom spike.
            self.polar(r3, th, -1.0, 0.0),         // Return.
            self.polar(r2, th, -1.0, -tick / 2.0), // Bottom right.
            self.polar(r2, th, 1.0, tick / 2.0),   // Top right.
        ];
        d.win().draw_lines(&self.discs[k].gc, &body);
    }

    fn roll_disc(disc: &mut Disc) {
        let mut th = disc.theta as f64;
        if th < 0.0 {
            th = -(th + disc.velocity as f64);
        } else {
            th += disc.velocity as f64;
        }

        if th > 360.0 * 64.0 {
            th -= 360.0 * 64.0;
        } else if th < 0.0 {
            th += 360.0 * 64.0;
        }

        disc.theta = if disc.theta > 0 {
            th as i32
        } else {
            -th as i32
        };

        disc.velocity += disc.acceleration;

        if disc.velocity > disc.limit || disc.velocity < -disc.limit {
            disc.acceleration = -disc.acceleration;
        }

        // Alter direction of rotational acceleration randomly.
        if random().is_multiple_of(120) {
            disc.acceleration = -disc.acceleration;
        }

        // Change acceleration very occasionally.
        if random().is_multiple_of(200) {
            if (random() & 1) != 0 {
                disc.acceleration = (disc.acceleration as f64 * 1.2) as i32;
            } else {
                disc.acceleration = (disc.acceleration as f64 * 0.8) as i32;
            }
        }
    }

    /// The bezel: eight fixed index marks around the outside, which are the
    /// only things on the instrument that do not spin.
    fn draw_pointer(&self, d: &mut Dpy) {
        let r = self.size as f64;
        let (x, y) = (self.x as f64, self.y as f64);
        let size = (r * 0.1) as i32;
        let p = |px: f64, py: f64| XPoint {
            x: px as i32,
            y: py as i32,
        };

        // Top.
        let top = [
            XPoint {
                x: self.x - size,
                y: self.y - self.size - size,
            },
            XPoint {
                x: self.x + size,
                y: self.y - self.size - size,
            },
            XPoint {
                x: self.x,
                y: self.y - self.size,
            },
        ];
        d.win().fill_polygon(&self.ptr_gc, &top);

        // Top right.
        let top_right = [
            p(x - r * 0.85, y - r * 0.8),
            p(x - r * 1.1, y - r * 0.55),
            p(x - r * 0.6, y - r * 0.65),
        ];
        d.win().fill_polygon(&self.ptr_gc, &top_right);

        let dot_gc = &self.discs[0].gc;
        for tri in [
            // Left.
            [
                p(x - r * 1.05, y),
                p(x - r * 1.1, y - r * 0.025),
                p(x - r * 1.1, y + r * 0.025),
            ],
            // Right.
            [
                p(x + r * 1.05, y),
                p(x + r * 1.1, y - r * 0.025),
                p(x + r * 1.1, y + r * 0.025),
            ],
            // Bottom.
            [
                p(x, y + r * 1.05),
                p(x - r * 0.025, y + r * 1.1),
                p(x + r * 0.025, y + r * 1.1),
            ],
            // Bottom left.
            [
                p(x + r * 0.74, y + r * 0.74),
                p(x + r * 0.78, y + r * 0.75),
                p(x + r * 0.75, y + r * 0.78),
            ],
            // Top left.
            [
                p(x + r * 0.74, y - r * 0.74),
                p(x + r * 0.78, y - r * 0.75),
                p(x + r * 0.75, y - r * 0.78),
            ],
            // Bottom right.
            [
                p(x - r * 0.74, y + r * 0.74),
                p(x - r * 0.78, y + r * 0.75),
                p(x - r * 0.75, y + r * 0.78),
            ],
        ] {
            d.win().fill_polygon(dot_gc, &tri);
        }
    }
}

impl Screenhack for Compass {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (w, h) = (self.width, self.height);
        d.win().fill_rectangle(&self.erase_gc, 0, 0, w, h);

        for k in 0..self.discs.len() {
            match self.discs[k].face {
                Face::Ticks => self.draw_ticks(d, k, self.size),
                Face::ThickArrow => self.draw_thick_arrow(d, k, self.size),
                Face::ThinArrow => self.draw_thin_arrow(d, k, self.size),
            }
            Self::roll_disc(&mut self.discs[k]);
        }
        self.draw_pointer(d);

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        // Upstream recomputes the window size and the centre but leaves the
        // compass's own size, and the line widths derived from it, alone.
        self.width = width;
        self.height = height;
        self.size2 = width.min(height);
        self.x = width / 2;
        self.y = height / 2;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: #000000",
    ".foreground: #DDFFFF",
    "*arrow1Foreground: #FFF66A",
    "*arrow2Foreground: #F7D64A",
    "*pointerForeground: #FF0000",
    "*delay: 20000",
    "*doubleBuffer: True",
];

const OPTS: &[Opt] =
    &[Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted()];

pub static DEF: SaverDef = SaverDef {
    slug: "compass",
    label: "Compass",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=IssDEcgB550"),
        blurb: "A compass, with all elements spinning about randomly, for that \"lost and nauseous\" feeling.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
