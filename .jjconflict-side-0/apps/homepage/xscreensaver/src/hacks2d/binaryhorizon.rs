//! Port of `hacks/binaryhorizon.c`.
//!
//! ```text
//! Binary Horizon
//! Copyright (c) 2020 Patrick Leiser <emilio.deltessa@gmail.com>
//!
//!  Directly ported code from complexification.net Binary Ring art
//!  http://www.complexification.net/gallery/machines/binaryRing/appletm/BinaryRing_m.pde
//!
//!  Directly Based on:
//!  Binary Ring code:
//!    j.tarbell   June, 2004
//!    Albuquerque, New Mexico
//!    complexification.net
//!
//! Directly based the hacks of:
//!
//! xscreensaver, Copyright (c) 1997, 1998, 2002 Jamie Zawinski <jwz@jwz.org>
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
//! Binary Ring rearranged: the particles are emitted along a horizontal line
//! rather than off a circle, and every time the epoch flips, that line moves
//! to a new height and the strokes change from lightening the picture to
//! darkening it. What builds up is a landscape of fine paths hanging above and
//! below a horizon that keeps being redrawn somewhere else.
//!
//! See [`super::binaryring`], which this is a variant of, and
//! [`crate::runtime::Fb::draw_line_antialias`], which both of them are built
//! on.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{rgb, unrgb};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent, frand,
    random,
};

const BLACK: usize = 0;
const WHITE: usize = 1;

/// `frand1()`: upstream doubles a signed random word and scales by 2^-31, so
/// the result runs from -1 up to (just under) 1. The doubling drops the top
/// bit, which is deliberate.
fn frand1() -> f32 {
    (((random() as i32) << 1) as f64 * 4.656_612_875_245_797e-10) as f32
}

#[derive(Clone, Copy, Default)]
struct Particle {
    x: f32,
    y: f32,
    xx: f32,
    yy: f32,
    vx: f32,
    vy: f32,
    color: Pixel,
    /// Age from 0 to max_age.
    age: i32,
}

struct State {
    epoch: usize,
    growth_delay: u32,
    max_age: i32,
    duration: f64,
    start_time: f64,
    curliness: f32,
    particles: Vec<Particle>,
    gc: Gc,
    width: i32,
    height: i32,
    /// Where the next horizon sits, relative to the middle of the screen.
    line_height: i32,
    /// Where the picture accumulates. The window is a copy of it.
    buffer: Pixmap,
    colors: [Pixel; 2],
    color: bool,
    bicolor: bool,
    fade: bool,
}

impl State {
    fn create_buffers(&mut self) {
        self.buffer = Pixmap::new(self.width, self.height);
        self.buffer.clear(self.colors[BLACK]);
    }

    fn next_color(&self, current: Pixel) -> Pixel {
        if self.fade {
            // Nudge each channel a couple of steps, so the palette drifts.
            let (r, g, b) = unrgb(current);
            let step = |v: u8| (v as i32 + (random() % 5) as i32 - 2).clamp(0, 255) as u8;
            rgb(step(r), step(g), step(b))
        } else {
            // The no-fade option: a fresh colour for every particle.
            let chan = || (random() % 255) as u8;
            rgb(chan(), chan(), chan())
        }
    }

    fn create_particles(&mut self) {
        let n = self.particles.len();
        for i in 0..n {
            // Emitted along the top edge rather than around a ring.
            let emitx = self.width as f32 * (i as f32 / n as f32);
            let emity = 0.0f32;
            let direction = (std::f32::consts::PI * i as f32) / n as f32;

            if self.epoch == WHITE && self.color {
                self.colors[WHITE] = self.next_color(self.colors[WHITE]);
            }

            let max_initial_velocity = 2.0f32;
            self.particles[i] = Particle {
                x: -emitx,
                y: -emity,
                xx: 0.0,
                yy: 0.0,
                vx: max_initial_velocity * direction.cos(),
                vy: max_initial_velocity * direction.sin(),
                age: (random() % self.max_age.max(1) as u32) as i32,
                color: self.colors[WHITE],
            };
        }
    }

    /// Randomly move one particle and draw it.
    fn move_particle(&mut self, i: usize) {
        let w = self.width / 2;
        let h = self.height / 2;
        let max_dv = 1.0f32;

        let p = &mut self.particles[i];
        p.xx = p.x;
        p.yy = p.y;
        p.x += p.vx;
        p.y += p.vy;
        p.vx += frand1() * self.curliness * max_dv;
        p.vy += frand1() * self.curliness * max_dv;
        let p = *p;

        self.buffer.draw_line_antialias(
            w + p.xx as i32,
            h + p.yy as i32,
            w + p.x as i32,
            h + p.y as i32,
            p.color,
            0.15,
        );
        self.buffer.draw_line_antialias(
            w - p.xx as i32,
            h + p.yy as i32,
            w - p.x as i32,
            h + p.y as i32,
            p.color,
            0.15,
        );

        self.particles[i].age += 1;
        // If this is too old, die and be reborn, back on the horizon.
        if self.particles[i].age > self.max_age {
            let dir = frand1() * 2.0 * std::f32::consts::PI;
            let line_height = self.line_height as f32;
            let width = self.width as f32;
            let p = &mut self.particles[i];
            p.x = width * dir.sin();
            p.y = line_height;
            p.xx = 0.0;
            p.yy = 0.0;
            p.vx = 0.0;
            p.vy = 0.0;
            p.age = 0;

            if self.epoch == WHITE && self.color {
                self.colors[WHITE] = self.next_color(self.colors[WHITE]);
            }
            if self.epoch == BLACK && self.color && self.bicolor {
                self.colors[BLACK] = self.next_color(self.colors[BLACK]);
            }
            self.particles[i].color = self.colors[self.epoch];
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let n = d.res.int("particles_number").clamp(1, 50_000) as usize;
    let duration = d.res.int("duration").max(0) as f64;
    let mut st = State {
        epoch: WHITE,
        growth_delay: d.res.int("growth_delay").max(0) as u32,
        max_age: d.res.int("max_age").max(1),
        // Dual screens are not in lockstep, so the reset is jittered.
        duration: if duration <= 0.0 {
            0.0
        } else {
            duration * (1.0 + frand(0.3))
        },
        start_time: 0.0,
        curliness: 0.5,
        particles: vec![Particle::default(); n],
        gc: Gc::default(),
        width: d.width(),
        height: d.height(),
        line_height: 0,
        buffer: Pixmap::new(1, 1),
        colors: [rgb(0, 0, 0), rgb(255, 255, 255)],
        color: d.res.bool("color"),
        bicolor: d.res.bool("bicolor"),
        fade: d.res.bool("fade"),
    };
    st.create_particles();
    st.create_buffers();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // Full reset every N seconds.
        if self.duration > 0.0 && d.time > self.start_time + self.duration {
            self.start_time = d.time;
            self.epoch = WHITE;
            self.create_particles();
            self.create_buffers();
        }

        for i in 0..self.particles.len() {
            self.move_particle(i);
        }

        let (w, h) = (self.width, self.height);
        d.win().copy_area(&self.gc, &self.buffer, 0, 0, w, h, 0, 0);

        // Randomly switch age-colour periods. Slower to change than the ring's.
        if random() % 10000 > 9975 {
            self.epoch = if self.epoch == WHITE { BLACK } else { WHITE };
            self.line_height = -((frand1() * self.height as f32 / 2.0) as i32).abs();
            if self.epoch == WHITE {
                self.line_height = -self.line_height;
            }
        }

        self.growth_delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
            self.epoch = WHITE;
            self.create_particles();
            self.create_buffers();
        }
    }

    /// If someone presses a key, switch the colour.
    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if matches!(event, XEvent::KeyPress { .. }) {
            self.epoch = if self.epoch == WHITE { BLACK } else { WHITE };
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*growth_delay: 10000",
    "*particles_number: 5000",
    "*max_age: 400",
    "*duration: 30",
    "*color: True",
    "*bicolor: True",
    "*fade: True",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "growth_delay",
        "Growth delay",
        0.0,
        100_000.0,
        1000.0,
        0,
        "10000",
    ),
    Opt::slider(
        "particles_number",
        "Number of particles",
        100.0,
        20_000.0,
        100.0,
        0,
        "5000",
    ),
    Opt::slider("duration", "Duration", 1.0, 120.0, 1.0, 0, "30"),
    Opt::boolean("color", "Random colors", "True"),
    Opt::boolean("bicolor", "Two contrasting colors", "True"),
    Opt::boolean("fade", "Fade between colors", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "binaryhorizon",
    label: "Binary Horizon",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Patrick Leiser, J. Tarbell and Emilio Del Tessandoro",
        year: "2021",
        video: Some("https://www.youtube.com/watch?v=upB7CSoxNTs"),
        blurb: "A system of path tracing particles evolves continuously from an initial horizon.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
