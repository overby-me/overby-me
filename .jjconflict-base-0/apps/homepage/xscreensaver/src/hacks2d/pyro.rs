//! Port of `hacks/pyro.c`.
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
//! Draw some fireworks.  Inspired from TI Explorer Lisp code by
//! John S. Pezaris <pz@hx.lcs.mit.edu>
//! ```
//!
//! Fireworks. A rocket rises white until its fuse burns out, then bursts into a
//! shower of coloured shrapnel that shrinks as it falls. Everything runs in
//! fixed point, and gravity is proportional to a particle's size, so the big
//! sparks fall fastest. The burst directions come from a sine table whose
//! amplitude is randomised per entry, which is what keeps the shells from
//! looking like perfect circles.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::hsv_to_rgb;
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XColor, frand,
    random_below,
};

/// Two pi, in the units the burst-direction tables are indexed by.
const PI_2000: usize = 6284;

#[derive(Clone, Copy)]
struct Projectile {
    /// Position and velocity, in 1/1024 pixel.
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    decay: i32,
    size: i32,
    fuse: i32,
    /// True while this is the rising rocket rather than its shrapnel.
    primary: bool,
    dead: bool,
    color: Pixel,
}

impl Default for Projectile {
    /// Every slot starts on the free list, which upstream arranges by handing
    /// the whole array to its free routine.
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            dx: 0,
            dy: 0,
            decay: 0,
            size: 0,
            fuse: 0,
            primary: false,
            dead: true,
            color: 0,
        }
    }
}

struct Pyro {
    draw_gc: Gc,
    erase_gc: Gc,
    default_fg: Pixel,
    mono: bool,
    how_many: usize,
    frequency: i32,
    scatter: i32,
    delay: u32,
    projectiles: Vec<Projectile>,
    /// Free slots, most recently freed first.
    free: Vec<usize>,
    /// Visit order, kept sorted by colour so the GC changes colour rarely.
    order: Vec<usize>,
    sin_cache: Vec<i32>,
    cos_cache: Vec<i32>,
    draw_xlim: i32,
    draw_ylim: i32,
    real_draw_xlim: i32,
    real_draw_ylim: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    let mut how_many = d.res.int("count");
    let mut frequency = d.res.int("frequency");
    let mut scatter = d.res.int("scatter");
    if how_many <= 0 {
        how_many = 100;
    }
    if frequency <= 0 {
        frequency = 30;
    }
    if scatter <= 0 {
        scatter = 20;
    }
    let how_many = how_many as usize;

    // The burst directions, with a randomised amplitude per entry: upstream
    // calls this "slightly whacked, for better explosions".
    let mut sin_cache = vec![0; PI_2000];
    let mut cos_cache = vec![0; PI_2000];
    for i in 0..PI_2000 {
        // Emulation of spherical distribution.
        let mut da = ((random_below((PI_2000 / 2) as i32)) as f64 / 1000.0).sin();
        // Approximating the integration of the binomial, for well-distributed
        // randomness.
        da += frand(1.0).asin() / std::f64::consts::FRAC_PI_2 * 0.1;
        cos_cache[i] = ((i as f64 / 1000.0).cos() * da * 2500.0) as i32;
        sin_cache[i] = ((i as f64 / 1000.0).sin() * da * 2500.0) as i32;
    }

    d.clear_window();

    Box::new(Pyro {
        draw_gc: Gc::new(fg, d.res.pixel("background")),
        erase_gc: Gc::new(d.res.pixel("background"), d.res.pixel("background")),
        default_fg: fg,
        mono: d.mono_p,
        how_many,
        frequency,
        scatter,
        delay: d.res.int("delay").max(0) as u32,
        projectiles: vec![Projectile::default(); how_many],
        free: (0..how_many).collect(),
        order: (0..how_many).collect(),
        sin_cache,
        cos_cache,
        draw_xlim: 0,
        draw_ylim: 0,
        real_draw_xlim: 0,
        real_draw_ylim: 0,
    })
}

impl Pyro {
    fn get_projectile(&mut self) -> Option<usize> {
        let at = self.free.pop()?;
        self.projectiles[at].dead = false;
        Some(at)
    }

    fn free_projectile(&mut self, at: usize) {
        self.projectiles[at].dead = true;
        self.free.push(at);
    }

    /// Send up a rocket, aimed so that it will still be on screen when its
    /// fuse burns out.
    fn launch(&mut self, xlim: i32, ylim: i32, g: i32) {
        let Some(at) = self.get_projectile() else {
            return;
        };

        let (x, dx) = loop {
            let x = random_below(xlim);
            let dx = 30000 - random_below(60000);
            let xxx = x + dx * 200;
            if xxx > 0 && xxx < xlim {
                break (x, dx);
            }
        };

        let color = if self.mono {
            self.default_fg
        } else {
            let (r, gg, b) = hsv_to_rgb(random_below(360), 1.0, 1.0);
            XColor::from_rgb16(r, gg, b).pixel
        };

        let dy = random_below(4000) - 13000;
        let mut fuse = ((random_below(500) + 500) * (dy / g).abs()) / 1000;
        // Cope with small windows: those constants assume big windows.
        let dd = 1000000 / ylim.max(1);
        if dd > 1 {
            fuse /= dd;
        }

        let p = &mut self.projectiles[at];
        p.x = x;
        p.y = ylim;
        p.dx = dx;
        p.size = 8000;
        p.decay = 0;
        p.dy = dy;
        p.fuse = fuse;
        p.primary = true;
        p.color = color;
    }

    fn shrapnel(&mut self, parent: usize) {
        let Some(at) = self.get_projectile() else {
            return;
        };
        let v = random_below(PI_2000 as i32) as usize;
        let par = self.projectiles[parent];
        let p = &mut self.projectiles[at];
        p.x = par.x;
        p.y = par.y;
        p.dx = self.sin_cache[v] + par.dx;
        p.dy = self.cos_cache[v] + par.dy;
        p.decay = random_below(50) - 60;
        p.size = (par.size * 2) / 3;
        p.fuse = 0;
        p.primary = false;
        p.color = par.color;
    }
}

impl Screenhack for Pyro {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let g = 100;
        let mut resort = false;
        let mut last_pixel: Option<Pixel> = None;

        for i in 0..self.how_many {
            let at = self.order[i];
            if self.projectiles[at].dead {
                continue;
            }

            let p = &mut self.projectiles[at];
            let old_x = p.x >> 10;
            let old_y = p.y >> 10;
            let old_size = p.size >> 10;
            p.size += p.decay;
            let size = p.size >> 10;
            p.x += p.dx;
            let x = p.x >> 10;
            p.y += p.dy;
            let y = p.y >> 10;
            // Gravity goes with size, so the big sparks fall fastest.
            p.dy += p.size >> 6;
            if p.primary {
                p.fuse -= 1;
            }
            let (primary, fuse, size_now, color) = (p.primary, p.fuse, p.size, p.color);

            if old_size > 0 {
                if old_size == 1 {
                    d.win().draw_point(&self.erase_gc, old_x, old_y);
                } else {
                    d.win()
                        .fill_rectangle(&self.erase_gc, old_x, old_y, old_size, old_size);
                }
            }

            let alive = if primary { fuse > 0 } else { size_now > 0 };
            if alive && x < self.real_draw_xlim && y < self.real_draw_ylim && x > 0 && y > 0 {
                if size > 0 {
                    let pixel = if self.mono || primary {
                        self.default_fg
                    } else {
                        color
                    };
                    if last_pixel != Some(pixel) {
                        last_pixel = Some(pixel);
                        self.draw_gc.set_foreground(pixel);
                    }

                    if size == 1 {
                        d.win().draw_point(&self.draw_gc, x, y);
                    } else if size < 4 {
                        d.win().fill_rectangle(&self.draw_gc, x, y, size, size);
                    } else {
                        d.win()
                            .fill_arc(&self.draw_gc, x, y, size, size, 0, FULL_CIRCLE);
                    }
                }
            } else {
                self.free_projectile(at);
            }

            if primary && fuse <= 0 {
                let mut j = random_below(self.scatter) + (self.scatter / 2);
                while j > 0 {
                    self.shrapnel(at);
                    j -= 1;
                }
                resort = true;
            }
        }

        if random_below(self.frequency) == 0 {
            self.real_draw_xlim = d.width();
            self.real_draw_ylim = d.height();
            self.draw_xlim = self.real_draw_xlim * 1000;
            self.draw_ylim = self.real_draw_ylim * 1000;
            let (xlim, ylim) = (self.draw_xlim, self.draw_ylim);
            self.launch(xlim, ylim, g);
            resort = true;
        }

        // Being sorted lets us avoid changing the colour as often.
        if resort {
            let projectiles = &self.projectiles;
            self.order.sort_by_key(|&i| projectiles[i].color);
        }

        self.delay
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*count: 600",
    "*delay: 10000",
    "*frequency: 30",
    "*scatter: 100",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Particle density", 10.0, 2000.0, 10.0, 0, "600"),
    Opt::slider("frequency", "Launch frequency", 1.0, 100.0, 1.0, 0, "30").inverted(),
    Opt::slider("scatter", "Explosive yield", 1.0, 400.0, 1.0, 0, "100"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "pyro",
    label: "Pyro",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=JJqVfnMstuw"),
        blurb: "Exploding fireworks.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
