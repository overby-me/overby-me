//! Port of `hacks/eruption.c`.
//!
//! ```text
//! Eruption, Copyright (c) 2002-2003 W.P. van Paassen <peter@paassen.tmfweb.nl>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Module - "eruption.c"
//!
//! [01-2015] - Dave Odell <dmo2118@gmail.com>: Performance tweaks. Also, click-for-explosion.
//! [02-2003] - W.P. van Paassen: Improvements, added some code of jwz from the pyro hack for a spherical distribution of the particles
//! [01-2003] - W.P. van Paassen: Port to X for use with XScreenSaver, the shadebob hack by Shane Smit was used as a template
//! [04-2002] - W.P. van Paassen: Creation for the Demo Effects Collection (http://demo-effects.sourceforge.net)
//! ```
//!
//! A few hundred particles are thrown out of one point, bounce off the walls
//! and cool as they go. They are not drawn: they stamp their heat into a byte
//! per pixel, and then the whole field is run through a blur that also takes a
//! constant off every cell. Reading the result through a black-blue-red-yellow
//! -white palette is what makes it look like fire rather than confetti.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::rgb;
use crate::runtime::{
    About, Dpy, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XEvent, frand, random,
    random_below, screenhack_event_helper,
};

/// Slightly whacked, for better explosions.
const PI_2000: usize = 6284;
const SPREAD: i32 = 15;

/// Could be more if anybody wanted the blur to fall off the left and right
/// edges.
const X_PAD: usize = 1;
const Y_PAD: usize = 1;

#[derive(Clone, Copy, Default)]
struct Particle {
    xpos: i16,
    ypos: i16,
    xdir: i16,
    ydir: i16,
    colorindex: u8,
    dead: bool,
}

struct State {
    sin_cache: Vec<i32>,
    cos_cache: Vec<i32>,
    particles: Vec<Particle>,
    win_width: i32,
    win_height: i32,
    /// A byte of heat per pixel, padded by one all round so the blur can read
    /// off the edge.
    fire: Vec<u8>,
    xdelta: u8,
    ydelta: u8,
    decay: i32,
    gravity: i16,
    heat: i32,

    cycles: i32,
    delay: u32,
    color_count: i32,
    color_vals: Vec<Pixel>,
    draw_i: i32,
}

impl State {
    /// Needs to be run once. Could easily be reimplemented to run and cache at
    /// compile time.
    fn cache(&mut self) {
        self.sin_cache = vec![0; PI_2000];
        self.cos_cache = vec![0; PI_2000];
        for i in 0..PI_2000 {
            let mut da = ((random_below(PI_2000 as i32 / 2) as f64) / 1000.0).sin();
            // Emulation of spherical distribution.
            da += frand(1.0).asin() / std::f64::consts::FRAC_PI_2 * 0.1;
            // Approximating the integration of the binomial, for
            // well-distributed randomness.
            self.cos_cache[i] =
                -(((i as f64 / 1000.0).cos() * da * self.ydelta as f64) as i32).abs();
            self.sin_cache[i] = ((i as f64 / 1000.0).sin() * da * self.xdelta as f64) as i32;
        }
    }

    fn init_particle(&mut self, i: usize, xcenter: i32, ycenter: i32) {
        let v = (random() as usize) % PI_2000;
        let p = &mut self.particles[i];
        p.xpos = (xcenter - SPREAD + random_below(SPREAD * 2)) as i16;
        p.ypos = (ycenter - SPREAD + random_below(SPREAD * 2)) as i16;
        p.xdir = self.sin_cache[v] as i16;
        p.ydir = self.cos_cache[v] as i16;
        p.colorindex = (self.color_count - 1) as u8;
        p.dead = false;
    }

    fn new_eruption(&mut self, xcenter: i32, ycenter: i32) {
        for i in 0..self.particles.len() {
            self.init_particle(i, xcenter, ycenter);
        }
        self.draw_i = 0;
    }

    fn random_eruption(&mut self) {
        let (x, y) = (
            random_below(self.win_width.max(1)),
            random_below(self.win_height.max(1)),
        );
        self.new_eruption(x, y);
    }

    fn move_particles(&mut self) {
        let (w, h) = (self.win_width as i16, self.win_height as i16);
        let count = self.color_count as u8;
        for p in &mut self.particles {
            if p.dead {
                continue;
            }
            p.xpos = p.xpos.wrapping_add(p.xdir);
            p.ypos = p.ypos.wrapping_add(p.ydir);

            // Is the particle dead?
            if p.colorindex == 0 {
                p.dead = true;
                continue;
            }

            if p.xpos < 1 {
                p.xpos = 1;
                p.xdir = p.xdir.wrapping_neg().wrapping_sub(4);
                p.colorindex = count;
            } else if p.xpos >= w - 2 {
                p.xpos = (w - 2).max(1);
                p.xdir = p.xdir.wrapping_neg().wrapping_add(4);
                p.colorindex = count;
            }

            if p.ypos < 1 {
                p.ypos = 1;
                p.ydir = p.ydir.wrapping_neg();
                p.colorindex = count;
            } else if p.ypos >= h - 3 {
                p.ypos = (h - 3).max(1);
                p.ydir = (p.ydir.wrapping_neg() >> 2).wrapping_sub(random_below(2) as i16);
                p.colorindex = count;
            }

            // Gravity kicks in, then the particle cools off.
            p.ydir = p.ydir.wrapping_add(self.gravity);
            // A bounce sets the index to the colour count, which is 256 by
            // default and so truncates to zero in the byte it lives in. The
            // decrement then wraps it round to the hottest colour, which is
            // what re-lights a particle that has hit a wall.
            p.colorindex = p.colorindex.wrapping_sub(1);
        }
    }

    fn stamp_particles(&mut self) {
        let pitch = self.win_width as usize + X_PAD * 2;
        let h = self.win_height;
        for p in &self.particles {
            let y = p.ypos as i32;
            if p.dead || y < -(Y_PAD as i32) + 1 || y >= h + Y_PAD as i32 - 1 {
                continue;
            }
            let x = p.xpos as i32;
            if x < -(X_PAD as i32) || x >= self.win_width + X_PAD as i32 - 1 {
                continue;
            }
            // Draw the particle as a five-cell plus.
            let center = (y + Y_PAD as i32) as usize * pitch + (x + X_PAD as i32) as usize;
            let color = p.colorindex;
            self.fire[center] = color;
            self.fire[center - 1] = color;
            self.fire[center + 1] = color;
            if y < h + Y_PAD as i32 - 2 {
                self.fire[center + pitch] = color;
            }
            if y >= -(Y_PAD as i32) + 2 {
                self.fire[center - pitch] = color;
            }
        }
    }

    /// The blur, and the paint.
    ///
    /// This is basically the GIMP's convolution matrix filter with a ring of
    /// ones, a divisor of eight and an offset of minus the cooling factor,
    /// except that each cell is written as it is computed, left to right and
    /// top to bottom, so the result smears rightwards and downwards.
    fn burn_and_paint(&mut self, d: &mut Dpy) {
        let pitch = self.win_width as usize + X_PAD * 2;
        let w = self.win_width as usize;

        for i in 0..self.win_height as usize {
            let l0 = i * pitch + X_PAD;
            let l1 = l0 + pitch;
            let l2 = l1 + pitch;

            let mut t0 =
                self.fire[l0 - 1] as i32 + self.fire[l1 - 1] as i32 + self.fire[l2 - 1] as i32;
            let mut t1 = self.fire[l0] as i32 + self.fire[l1] as i32 + self.fire[l2] as i32;

            for j in 1..w + X_PAD {
                let t2 =
                    self.fire[l0 + j] as i32 + self.fire[l1 + j] as i32 + self.fire[l2 + j] as i32;
                let px = l1 + j - 1;
                t1 -= self.fire[px] as i32;
                let temp = t0 + t1 + t2 - self.decay;
                let temp = if temp >= 0 { temp >> 3 } else { 0 };
                self.fire[px] = temp as u8;
                t0 = t1 + temp;
                t1 = t2;
            }

            // Draw the fire array to the screen.
            for j in 0..w {
                let c = self.color_vals[self.fire[l1 + j] as usize];
                d.win().put_pixel(j as i32, i as i32, c);
            }
        }
    }

    /// Black to blue to red to yellow to white, which is what turns a field of
    /// cooling numbers into flame.
    fn set_palette(&mut self, ncolors: i32) {
        let count = ncolors.clamp(16, 256);
        let step = (65535 / count) as u16;
        self.color_vals = (0..count)
            .map(|i| {
                let (r, g, b): (u16, u16, u16) = if i < count >> 3 {
                    // Black to blue.
                    (0, 0, step.wrapping_mul((i << 1) as u16))
                } else if i < count >> 2 {
                    // Blue to red.
                    let t = (i - (count >> 3)) as u16;
                    (
                        step.wrapping_mul(t << 3),
                        0,
                        16383u16.wrapping_sub(step.wrapping_mul(t << 1)),
                    )
                } else if i < (count >> 2) + (count >> 3) {
                    // Red to yellow.
                    let t = ((i - (count >> 2)) << 3) as u16;
                    (65535, step.wrapping_mul(t), 0)
                } else if i < count >> 1 {
                    // Yellow to white.
                    let t = ((i - ((count >> 2) + (count >> 3))) << 3) as u16;
                    (65535, 65535, step.wrapping_mul(t))
                } else {
                    (65535, 65535, 65535)
                };
                rgb((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8)
            })
            .collect();

        // The palette keeps all its entries, but particles only start as hot
        // as this, so a lower heat never reaches the white end.
        self.color_count = if self.heat < count { self.heat } else { count };
    }

    fn create_image(&mut self, width: i32, height: i32) {
        self.win_width = width;
        self.win_height = height;
        self.fire = vec![0; (height as usize + Y_PAD * 2) * (width as usize + X_PAD * 2)];
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let n = d.res.int("particles").clamp(100, 2000) as usize;
    let decay = d.res.int("cooloff").clamp(0, 10) << 3;
    let gravity = d.res.int("gravity").clamp(-5, 5) as i16;
    let heat = d.res.int("heat").clamp(64, 256);

    let mut st = State {
        sin_cache: Vec::new(),
        cos_cache: Vec::new(),
        particles: vec![Particle::default(); n],
        win_width: 0,
        win_height: 0,
        fire: Vec::new(),
        xdelta: 0,
        ydelta: 0,
        decay,
        gravity,
        heat,
        cycles: d.res.int("cycles").max(1),
        delay: d.res.int("delay").max(0) as u32,
        color_count: 256,
        color_vals: Vec::new(),
        draw_i: -1,
    };
    st.create_image(d.width(), d.height());

    // How far a particle may be thrown: enough to reach the middle of the
    // window vertically, and an eighth of it sideways.
    let mut sum: i32 = 0;
    while sum < (st.win_height >> 1) - SPREAD {
        st.ydelta = st.ydelta.saturating_add(1);
        sum += st.ydelta as i32;
        if st.ydelta == u8::MAX {
            break;
        }
    }
    let mut sum: i32 = 0;
    while sum < st.win_width >> 3 {
        st.xdelta = st.xdelta.saturating_add(1);
        sum += st.xdelta as i32;
        if st.xdelta == u8::MAX {
            break;
        }
    }

    st.cache();
    st.set_palette(d.res.int("ncolors"));
    d.clear_window();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.draw_i < 0 {
            self.random_eruption();
        } else {
            let i = self.draw_i;
            self.draw_i += 1;
            if i >= self.cycles {
                self.random_eruption();
            }
        }

        self.move_particles();
        self.stamp_particles();
        self.burn_and_paint(d);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.create_image(width, height);
        self.draw_i = -1;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if let XEvent::ButtonPress { x, y, .. } = event {
            self.new_eruption(*x, *y);
            return true;
        }
        if screenhack_event_helper(event) {
            self.random_eruption();
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsTop: true",
    "*cycles: 80",
    "*ncolors: 256",
    "*delay: 10000",
    "*particles: 300",
    "*cooloff: 2",
    "*gravity: 1",
    "*heat: 256",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("ncolors", "Number of colors", 16.0, 256.0, 1.0, 0, "256"),
    Opt::slider(
        "particles",
        "Number of particles",
        100.0,
        2000.0,
        10.0,
        0,
        "300",
    ),
    Opt::slider("cooloff", "Cooling factor", 0.0, 10.0, 1.0, 0, "2"),
    Opt::slider("heat", "Heat", 64.0, 256.0, 1.0, 0, "256"),
    Opt::slider("gravity", "Gravity", -5.0, 5.0, 1.0, 0, "1"),
    Opt::slider("cycles", "Duration", 10.0, 3000.0, 10.0, 0, "80"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "eruption",
    label: "Eruption",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "W.P. van Paassen",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=CQ6jDBnumT8"),
        blurb: "Exploding fireworks.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
