//! Port of `hacks/halftone.c`.
//!
//! ```text
//! halftone, Copyright (c) 2002 by Peter Jaric <peter@jaric.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Description:
//! Draws the gravitational force in each point on the screen seen
//! through a halftone dot pattern. The force is calculated from a set
//! of moving mass points. View it from a distance for best effect.
//! ```
//!
//! A field of dots on a fixed grid, each sized by how strongly a handful of
//! drifting point masses pull on it. Where two masses pass near each other the
//! dots swell and the pattern blooms; where they are far apart it fades to
//! nothing. The two colours cycle slowly through a smooth map, so the whole
//! field drifts in hue as well as in shape.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::{About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, frand};

const DEFAULT_DELAY: i32 = 10000;
const DEFAULT_SPACING: i32 = 14;
const DEFAULT_SIZE_FACTOR: f64 = 1.5;
const DEFAULT_COUNT: i32 = 10;
const DEFAULT_MIN_MASS: f64 = 0.001;
const DEFAULT_MAX_MASS: f64 = 0.02;
const DEFAULT_MIN_SPEED: f64 = 0.001;
const DEFAULT_MAX_SPEED: f64 = 0.02;

struct Mass {
    x: f64,
    y: f64,
    mass: f64,
    x_inc: f64,
    y_inc: f64,
}

struct Halftone {
    /// How hard each grid point is being pulled, zero to one.
    dots: Vec<f64>,
    dots_width: i32,
    dots_height: i32,
    spacing: i32,
    max_dot_size: f64,
    masses: Vec<Mass>,
    gc: Gc,
    ncolors: usize,
    colors: Vec<XColor>,
    color0: usize,
    color1: usize,
    color_tick: i32,
    cycle_speed: i32,
    width: i32,
    height: i32,
    delay: u32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let delay = d.res.int("delay");
    let delay = if delay < 0 { DEFAULT_DELAY } else { delay };

    let count = d.res.int("count");
    let count = if count < 1 { DEFAULT_COUNT } else { count };

    let spacing = d.res.int("spacing");
    let mut spacing = if spacing < 1 {
        DEFAULT_SPACING
    } else {
        spacing
    };

    let min_mass = d.res.float("minMass");
    let min_mass = if min_mass < 0.0 {
        DEFAULT_MIN_MASS
    } else {
        min_mass
    };
    let max_mass = d.res.float("maxMass");
    let max_mass = if max_mass < 0.0 {
        DEFAULT_MAX_MASS
    } else {
        max_mass
    };
    let max_mass = max_mass.max(min_mass);

    let min_speed = d.res.float("minSpeed");
    let min_speed = if min_speed < 0.0 {
        DEFAULT_MIN_SPEED
    } else {
        min_speed
    };
    let max_speed = d.res.float("maxSpeed");
    let max_speed = if max_speed < 0.0 {
        DEFAULT_MAX_SPEED
    } else {
        max_speed
    };
    let max_speed = max_speed.max(min_speed);

    let masses = (0..count)
        .map(|_| Mass {
            x: frand(1.0),
            y: frand(1.0),
            mass: min_mass + (max_mass - min_mass) * frand(1.0),
            x_inc: min_speed + (max_speed - min_speed) * frand(1.0),
            y_inc: min_speed + (max_speed - min_speed) * frand(1.0),
        })
        .collect();

    if d.width() > 2560 || d.height() > 2560 {
        spacing *= 3; // Retina displays
    }

    let factor = d.res.float("sizeFactor");
    let factor = if factor < 0.0 {
        DEFAULT_SIZE_FACTOR
    } else {
        factor
    };

    let ncolors = d.res.int("colors").max(4) as usize;
    let colors = make_smooth_colormap(ncolors);

    let mut st = Halftone {
        dots: Vec::new(),
        dots_width: 0,
        dots_height: 0,
        spacing,
        max_dot_size: factor * spacing as f64,
        masses,
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        ncolors: colors.len(),
        color0: 0,
        color1: colors.len() / 2,
        colors,
        color_tick: 0,
        cycle_speed: d.res.int("cycleSpeed"),
        width: d.width(),
        height: d.height(),
        delay: delay.max(0) as u32,
    };
    st.update_dot_attributes();
    Box::new(st)
}

impl Halftone {
    fn update_dot_attributes(&mut self) {
        let w = self.width / self.spacing + 1;
        let h = self.height / self.spacing + 1;
        if self.dots.is_empty() || w != self.dots_width || h != self.dots_height {
            self.dots_width = w;
            self.dots_height = h;
            self.dots = vec![0.0; (w * h) as usize];
        }
    }

    /// The pull at a grid point, as the length of the summed force vector.
    fn gravity_at(&self, x: i32, y: i32) -> f64 {
        let mut gx = 0.0;
        let mut gy = 0.0;
        for m in &self.masses {
            let dx = x as f64 - m.x * self.dots_width as f64;
            let dy = y as f64 - m.y * self.dots_height as f64;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance != 0.0 {
                let gravity = m.mass
                    / (distance * distance / (self.dots_width as f64 * self.dots_height as f64));
                gx += (dx / distance) * gravity;
                gy += (dy / distance) * gravity;
            }
        }
        (gx * gx + gy * gy).sqrt()
    }

    fn repaint(&mut self, d: &mut Dpy) {
        let c = self.colors[self.color0].pixel;
        self.gc.set_foreground(c);
        let (w, h) = (self.width, self.height);
        d.win().fill_rectangle(&self.gc, 0, 0, w, h);

        let c = self.colors[self.color1].pixel;
        self.gc.set_foreground(c);

        self.color_tick += 1;
        if self.color_tick >= self.cycle_speed {
            self.color_tick = 0;
            self.color0 = (self.color0 + 1) % self.ncolors;
            self.color1 = (self.color1 + 1) % self.ncolors;
        }

        for x in 0..self.dots_width {
            for y in 0..self.dots_height {
                let size =
                    (self.max_dot_size * self.dots[(x + y * self.dots_width) as usize]) as i32;
                if size <= 0 {
                    continue;
                }
                d.win().fill_arc(
                    &self.gc,
                    x * self.spacing - size / 2,
                    y * self.spacing - size / 2,
                    size,
                    size,
                    0,
                    FULL_CIRCLE,
                );
            }
        }
    }

    fn update(&mut self) {
        self.update_dot_attributes();

        for m in self.masses.iter_mut() {
            if m.x >= 1.0 || m.x <= 0.0 {
                m.x_inc = -m.x_inc;
            }
            if m.y >= 1.0 || m.y <= 0.0 {
                m.y_inc = -m.y_inc;
            }
            m.x += m.x_inc;
            m.y += m.y_inc;
        }

        for x in 0..self.dots_width {
            for y in 0..self.dots_height {
                let gravity = self.gravity_at(x, y);
                self.dots[(x + y * self.dots_width) as usize] = gravity.clamp(0.0, 1.0);
            }
        }
    }
}

impl Screenhack for Halftone {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.repaint(d);
        self.update();
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.update_dot_attributes();
    }
}

const DEFAULTS: &[&str] = &[
    ".background: Black",
    ".foreground: White",
    "*delay: 10000",
    "*count: 10",
    "*minMass: 0.001",
    "*maxMass: 0.02",
    "*minSpeed: 0.001",
    "*maxSpeed: 0.02",
    "*spacing: 14",
    "*sizeFactor: 1.5",
    "*colors: 200",
    "*cycleSpeed: 10",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Gravity points", 1.0, 50.0, 1.0, 0, "10"),
    Opt::slider("spacing", "Dot size", 2.0, 50.0, 1.0, 0, "14"),
    Opt::slider("sizeFactor", "Dot fill factor", 0.1, 3.0, 0.1, 1, "1.5"),
    Opt::slider("minSpeed", "Minimum speed", 0.001, 0.09, 0.001, 3, "0.001"),
    Opt::slider("maxSpeed", "Maximum speed", 0.001, 0.09, 0.001, 3, "0.02"),
    Opt::slider("minMass", "Minimum mass", 0.001, 0.09, 0.001, 3, "0.001"),
    Opt::slider("maxMass", "Maximum mass", 0.001, 0.09, 0.001, 3, "0.02"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "halftone",
    label: "Halftone",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Peter Jaric",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=K2lqgBPde4o"),
        blurb: "A halftone dot pattern in motion.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
