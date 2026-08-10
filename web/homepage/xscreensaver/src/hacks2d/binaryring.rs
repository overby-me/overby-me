//! Port of `hacks/binaryring.c`.
//!
//! ```text
//! Binary Ring
//! Copyright (c) 2006-2014 Emilio Del Tessandoro <emilio.deltessa@gmail.com>
//!
//!  Directly ported code from complexification.net Binary Ring art
//!  http://www.complexification.net/gallery/machines/binaryRing/appletm/BinaryRing_m.pde
//!
//!  Binary Ring code:
//!  j.tarbell   June, 2004
//!  Albuquerque, New Mexico
//!  complexification.net
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
//! Thousands of particles are emitted from a ring and wander off, each one
//! drawing the faint antialiased line between where it was and where it is.
//! Every particle is drawn twice, mirrored about the vertical axis, which is
//! where the symmetry comes from. Nothing is ever erased: the picture is the
//! accumulation of a hundred thousand fifteen-percent strokes, and it lightens
//! or darkens depending on which epoch the hack is currently in.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{rgb, unrgb};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent, random,
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
    ring_radius: i32,
    max_age: i32,
    curliness: f32,
    particles: Vec<Particle>,
    gc: Gc,
    width: i32,
    height: i32,
    /// Where the picture accumulates. The window is a copy of it.
    buffer: Pixmap,
    colors: [Pixel; 2],
    color: bool,
}

impl State {
    /// Blend the pixel already there towards `myc` by `a`.
    fn draw_point(&mut self, x: i32, y: i32, myc: Pixel, a: f32) {
        let c = self.buffer.get_pixel(x, y);
        let (or, og, ob) = unrgb(c);
        let (r, g, b) = unrgb(myc);
        let mix = |o: u8, n: u8| (o as f32 + (n as f32 - o as f32) * a) as u8;
        self.buffer
            .put_pixel(x, y, rgb(mix(or, r), mix(og, g), mix(ob, b)));
    }

    fn plot(&mut self, x: i32, y: i32, col: Pixel, br: f32) {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            self.draw_point(x, y, col, br.min(1.0));
        }
    }

    /// Xiaolin Wu's line, which is what gives every stroke its soft edge.
    fn draw_line_antialias(
        &mut self,
        mut x1: i32,
        mut y1: i32,
        mut x2: i32,
        mut y2: i32,
        color: Pixel,
        alpha: f32,
    ) {
        let dx = x2 as f32 - x1 as f32;
        let dy = y2 as f32 - y1 as f32;

        // Hard clipping, because this routine has some problems with negative
        // coordinates.
        if x1 < 0
            || x1 > self.width
            || x2 < 0
            || x2 > self.width
            || y1 < 0
            || y1 > self.height
            || y2 < 0
            || y2 > self.height
        {
            return;
        }

        let ipart = |v: f32| v as i32;
        let round = |v: f32| (v + 0.5) as i32;
        let fpart = |v: f32| v - (v as i32) as f32;
        let rfpart = |v: f32| 1.0 - fpart(v);

        if dx.abs() > dy.abs() {
            if x2 < x1 {
                std::mem::swap(&mut x1, &mut x2);
                std::mem::swap(&mut y1, &mut y2);
            }
            let gradient = dy / dx;
            let mut xend = round(x1 as f32) as f32;
            let mut yend = y1 as f32 + gradient * (xend - x1 as f32);
            let mut xgap = rfpart(x1 as f32 + 0.5);
            let xpxl1 = xend as i32;
            let ypxl1 = ipart(yend);
            self.plot(xpxl1, ypxl1, color, rfpart(yend) * xgap * alpha);
            self.plot(xpxl1, ypxl1 + 1, color, fpart(yend) * xgap * alpha);
            let mut intery = yend + gradient;

            xend = round(x2 as f32) as f32;
            yend = y2 as f32 + gradient * (xend - x2 as f32);
            xgap = fpart(x2 as f32 + 0.5);
            let xpxl2 = xend as i32;
            let ypxl2 = ipart(yend);
            self.plot(xpxl2, ypxl2, color, rfpart(yend) * xgap * alpha);
            self.plot(xpxl2, ypxl2 + 1, color, fpart(yend) * xgap * alpha);

            for x in (xpxl1 + 1)..=(xpxl2 - 1) {
                self.plot(x, ipart(intery), color, rfpart(intery) * alpha);
                self.plot(x, ipart(intery) + 1, color, fpart(intery) * alpha);
                intery += gradient;
            }
        } else {
            if y2 < y1 {
                std::mem::swap(&mut x1, &mut x2);
                std::mem::swap(&mut y1, &mut y2);
            }
            let gradient = dx / dy;
            let mut yend = round(y1 as f32) as f32;
            let mut xend = x1 as f32 + gradient * (yend - y1 as f32);
            let mut ygap = rfpart(y1 as f32 + 0.5);
            let ypxl1 = yend as i32;
            let xpxl1 = ipart(xend);
            self.plot(xpxl1, ypxl1, color, rfpart(xend) * ygap * alpha);
            self.plot(xpxl1, ypxl1 + 1, color, fpart(xend) * ygap * alpha);
            let mut interx = xend + gradient;

            yend = round(y2 as f32) as f32;
            xend = x2 as f32 + gradient * (yend - y2 as f32);
            ygap = fpart(y2 as f32 + 0.5);
            let ypxl2 = yend as i32;
            let xpxl2 = ipart(xend);
            self.plot(xpxl2, ypxl2, color, rfpart(xend) * ygap * alpha);
            self.plot(xpxl2, ypxl2 + 1, color, fpart(xend) * ygap * alpha);

            for y in (ypxl1 + 1)..=(ypxl2 - 1) {
                self.plot(ipart(interx), y, color, rfpart(interx) * alpha);
                self.plot(ipart(interx) + 1, y, color, fpart(interx) * alpha);
                interx += gradient;
            }
        }
    }

    fn create_buffers(&mut self) {
        self.buffer = Pixmap::new(self.width, self.height);
        self.buffer.clear(self.colors[BLACK]);
    }

    /// Nudge each channel of the current colour by up to two steps, so the
    /// palette drifts rather than jumping.
    fn next_color(&self, current: Pixel) -> Pixel {
        let (r, g, b) = unrgb(current);
        let step = |v: u8| (v as i32 + (random() % 5) as i32 - 2).clamp(0, 255) as u8;
        rgb(step(r), step(g), step(b))
    }

    fn create_particles(&mut self) {
        let n = self.particles.len();
        for i in 0..n {
            let t = i as f32 / n as f32;
            let emitx = self.ring_radius as f32 * (std::f32::consts::PI * 2.0 * t).sin();
            let emity = self.ring_radius as f32 * (std::f32::consts::PI * 2.0 * t).cos();
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

        self.draw_line_antialias(
            w + p.xx as i32,
            h + p.yy as i32,
            w + p.x as i32,
            h + p.y as i32,
            p.color,
            0.15,
        );
        self.draw_line_antialias(
            w - p.xx as i32,
            h + p.yy as i32,
            w - p.x as i32,
            h + p.y as i32,
            p.color,
            0.15,
        );

        self.particles[i].age += 1;
        // If this is too old, die and be reborn.
        if self.particles[i].age > self.max_age {
            let dir = frand1() * 2.0 * std::f32::consts::PI;
            let p = &mut self.particles[i];
            p.x = self.ring_radius as f32 * dir.sin();
            p.y = self.ring_radius as f32 * dir.cos();
            p.xx = 0.0;
            p.yy = 0.0;
            p.vx = 0.0;
            p.vy = 0.0;
            p.age = 0;

            if self.epoch == WHITE && self.color {
                self.colors[WHITE] = self.next_color(self.colors[WHITE]);
            }
            self.particles[i].color = self.colors[self.epoch];
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let n = d.res.int("particles_number").clamp(1, 50_000) as usize;
    let mut st = State {
        epoch: WHITE,
        growth_delay: d.res.int("growth_delay").max(0) as u32,
        ring_radius: d.res.int("ring_radius").max(0),
        max_age: d.res.int("max_age").max(1),
        curliness: 0.5,
        particles: vec![Particle::default(); n],
        gc: Gc::default(),
        width: d.width(),
        height: d.height(),
        buffer: Pixmap::new(1, 1),
        colors: [rgb(0, 0, 0), rgb(255, 255, 255)],
        color: d.res.bool("color"),
    };
    st.create_particles();
    st.create_buffers();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for i in 0..self.particles.len() {
            self.move_particle(i);
        }

        let (w, h) = (self.width, self.height);
        d.win().copy_area(&self.gc, &self.buffer, 0, 0, w, h, 0, 0);

        // Randomly switch age-colour periods.
        if random() % 10000 > 9950 {
            self.epoch = if self.epoch == WHITE { BLACK } else { WHITE };
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
    "*ring_radius: 40",
    "*max_age: 400",
    "*color: True",
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
    Opt::slider("ring_radius", "Ring Radius", 0.0, 400.0, 1.0, 0, "40"),
    Opt::slider(
        "particles_number",
        "Number of particles",
        500.0,
        20_000.0,
        100.0,
        0,
        "5000",
    ),
    Opt::boolean("color", "Fade with colors", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "binaryring",
    label: "Binary Ring",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "J. Tarbell and Emilio Del Tessandoro",
        year: "2014",
        video: Some("https://www.youtube.com/watch?v=KPiJb0Qm1SE"),
        blurb: "A system of path tracing particles evolves continuously from an initial creation.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
