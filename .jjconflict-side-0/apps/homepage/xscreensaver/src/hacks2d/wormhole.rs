//! Port of `hacks/wormhole.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2011 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! wormhole:
//! Animation of moving through a wormhole. Based on my own code written
//! a few years ago.
//! author: Jon Rafkind <jon@rafkind.com>
//! date: 1/19/04
//! ```
//!
//! Short streaks are born on a small circle around a moving centre and pushed
//! outward by perspective as their depth counts down, so the mouth of the
//! tunnel appears to rush at you. The mouth itself wanders towards a target
//! point, picks a new one when it arrives or bumps an edge, and now and then
//! spirals instead. Colour comes from a two-thousand-entry ramp built out of
//! random shades; a window of a hundred and twenty-eight of them slides along
//! it, so the tunnel changes hue without any two frames disagreeing.
//!
//! Two upstream quirks are the look of the thing rather than slips to tidy up.
//! `Cos` and `Sine` multiply their argument by 180/pi instead of dividing, so
//! an "angle" in degrees is scrambled to somewhere else on the circle: the
//! streaks land in an order that has nothing to do with counting round. And the
//! streaks are drawn but never erased between frames only because the whole
//! frame is painted black first.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::XColor;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XEvent, random,
};

/// `rnd`. Upstream guards a non-positive modulus, which a small window reaches.
fn rnd(q: i32) -> i32 {
    let q = q.max(1);
    (random() % q as u32) as i32
}

/// Upstream's `Cos`, scale factor and all: this is not the cosine of an angle
/// in degrees, and that is the point.
fn cos_(a: i32) -> f64 {
    (a as f64 * 180.0 / std::f64::consts::PI).cos()
}

fn sine(a: i32) -> f64 {
    (a as f64 * 180.0 / std::f64::consts::PI).sin()
}

/// The angle from one point to another, in degrees, counting anticlockwise.
fn gang(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let mut tang = if x1 == x2 {
        if y1 < y2 { 90 } else { 270 }
    } else if y1 == y2 {
        if x1 < x2 { 0 } else { 180 }
    } else {
        (0.5 + ((-(y2 - y1)) as f64).atan2((x2 - x1) as f64) * 180.0 / std::f64::consts::PI) as i32
    };
    while tang < 0 {
        tang += 360;
    }
    tang % 360
}

fn dist(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let xs = x1 - x2;
    let ys = y1 - y2;
    (((xs * xs + ys * ys) as f64).sqrt()) as i32
}

#[derive(Clone, Copy, Default)]
struct Star {
    x: i32,
    y: i32,
    calc_x: i32,
    calc_y: i32,
    z: i32,
    center_x: i32,
    center_y: i32,
}

impl Star {
    /// Perspective: the further away, the closer to the centre of the mouth.
    fn calc(&mut self) {
        if self.center_x == 0 || self.center_y == 0 {
            self.z = 0;
            return;
        }
        if self.z <= 0 {
            self.calc_x = (self.x << 10) / self.center_x;
            self.calc_y = (self.y << 10) / self.center_y;
        } else {
            self.calc_x = (self.x << 10) / self.z + self.center_x;
            self.calc_y = (self.y << 10) / self.z + self.center_y;
        }
    }

    fn init(z: i32, ang: i32, worm: &Wormhole) -> Self {
        let mut s = Star {
            x: (cos_(ang) * worm.diameter as f64) as i32,
            y: (sine(ang) * worm.diameter as f64) as i32,
            calc_x: 0,
            calc_y: 0,
            center_x: worm.actual_x,
            center_y: worm.actual_y,
            z,
        };
        s.calc();
        s
    }
}

/// One streak: two points at the same angle but different depths.
#[derive(Clone, Copy, Default)]
struct StarLine {
    begin: Star,
    end: Star,
}

/// A long ramp of colours, and a window that slides along it.
struct ColorChanger {
    shade: Vec<XColor>,
    min: i32,
    max: i32,
    shade_use: i32,
    shade_max: i32,
    min_want: i32,
}

/// A random colour, kept clear of both black and white.
fn init_xcolor() -> XColor {
    XColor::from_rgb16(
        (rnd(50000) + 10000) as u16,
        (rnd(50000) + 10000) as u16,
        (rnd(50000) + 10000) as u16,
    )
}

/// A linear fade from one colour to another, over `max` entries.
fn blend_palette(pal: &mut [XColor], max: i32, sc: &XColor, ec: &XColor) {
    for (q, slot) in pal.iter_mut().enumerate().take(max as usize) {
        let j = q as f32 / max as f32;
        let mix = |s: u16, e: u16| -> u16 {
            (0.5 + s as f32 + (e as i32 - s as i32) as f32 * j) as i32 as u16
        };
        *slot = XColor::from_rgb16(
            mix(sc.red, ec.red),
            mix(sc.green, ec.green),
            mix(sc.blue, ec.blue),
        );
    }
}

impl ColorChanger {
    fn new() -> Self {
        let shade_max = 2048;
        let shade_use = 128;
        let mut ch = ColorChanger {
            shade: vec![XColor::default(); shade_max as usize],
            min: 0,
            max: shade_use,
            shade_use,
            shade_max,
            min_want: rnd(shade_max - shade_use),
        };

        let mut old_color = init_xcolor();
        let mut new_color = init_xcolor();
        let mut q = 0;
        while q < ch.shade_max {
            blend_palette(
                &mut ch.shade[q as usize..],
                ch.shade_use,
                &old_color,
                &new_color,
            );
            old_color = new_color;
            new_color = init_xcolor();
            q += ch.shade_use;
        }
        ch
    }

    /// Walk the window one step towards where it wants to be, and pick a new
    /// destination once it arrives.
    fn move_(&mut self) {
        if self.min < self.min_want {
            self.min += 1;
            self.max += 1;
        }
        if self.min > self.min_want {
            self.min -= 1;
            self.max -= 1;
        }
        if self.min == self.min_want {
            self.min_want = rnd(self.shade_max - self.shade_use);
        }
    }
}

struct Wormhole {
    diameter: i32,
    diameter_change: i32,
    actual_x: i32,
    actual_y: i32,
    virtual_x: f64,
    virtual_y: f64,
    speed: f64,
    ang: i32,
    want_x: i32,
    want_y: i32,
    max_z: i32,
    add_star: i32,
    spiral: i32,
    changer: ColorChanger,
    /// A slot per streak, holes and all: upstream keeps the array sparse and
    /// doubles it when it fills up, and where a streak lands in it is part of
    /// the drawing order.
    stars: Vec<Option<StarLine>>,
    black: Pixel,
}

impl Wormhole {
    fn new(screen_x: i32, screen_y: i32, make_stars: i32) -> Self {
        let actual_x = screen_x / 2;
        let actual_y = screen_y / 2;
        let want_x = rnd(screen_x - 50) + 25;
        let want_y = rnd(screen_y - 50) + 25;
        Wormhole {
            diameter: rnd(10) + 15,
            diameter_change: rnd(10) + 15,
            actual_x,
            actual_y,
            virtual_x: actual_x as f64,
            virtual_y: actual_y as f64,
            speed: screen_x as f64 / 180.0,
            ang: gang(actual_x, actual_y, want_x, want_y),
            want_x,
            want_y,
            max_z: 600,
            add_star: make_stars,
            spiral: 0,
            changer: ColorChanger::new(),
            stars: vec![None; 64],
            black: XColor::from_rgb16(0, 0, 0).pixel,
        }
    }

    fn spawn_star(&mut self) {
        let ang = rnd(360);
        let star_new = StarLine {
            begin: Star::init(self.max_z, ang, self),
            end: Star::init(self.max_z + rnd(6) + 4, ang, self),
        };

        if let Some(slot) = self.stars.iter_mut().find(|s| s.is_none()) {
            *slot = Some(star_new);
            return;
        }

        let old_stars = self.stars.len();
        self.stars.resize(old_stars * 2, None);
        self.stars[old_stars] = Some(star_new);
    }

    /// One step of the mouth, the streaks and the colour window.
    fn move_(&mut self, screen_x: i32, screen_y: i32, z_speed: i32) {
        const MIN_DIST: i32 = 100;
        let mut find = false;

        let dx = cos_(self.ang) * self.speed;
        let dy = sine(self.ang) * self.speed;
        self.virtual_x += dx;
        self.virtual_y += dy;
        self.actual_x = self.virtual_x as i32;
        self.actual_y = self.virtual_y as i32;

        if self.spiral != 0 {
            if self.spiral % 5 == 0 {
                self.ang = (self.ang + 1) % 360;
            }
            self.spiral -= 1;
            if self.spiral <= 0 {
                find = true;
            }
        } else {
            if dist(self.actual_x, self.actual_y, self.want_x, self.want_y) < 20 {
                find = true;
            }
            // Two independent rolls, so this fires one time in twenty.
            if rnd(20) == rnd(20) {
                find = true;
            }

            if self.actual_x < MIN_DIST {
                self.actual_x = MIN_DIST;
                self.virtual_x = self.actual_x as f64;
                find = true;
            }
            if self.actual_y < MIN_DIST {
                self.actual_y = MIN_DIST;
                self.virtual_y = self.actual_y as f64;
                find = true;
            }
            if self.actual_x > screen_x - MIN_DIST {
                self.actual_x = screen_x - MIN_DIST;
                self.virtual_x = self.actual_x as f64;
                find = true;
            }
            if self.actual_y > screen_y - MIN_DIST {
                self.actual_y = screen_y - MIN_DIST;
                self.virtual_y = self.actual_y as f64;
                find = true;
            }

            if rnd(500) == rnd(500) {
                self.spiral = rnd(30) + 50;
            }
        }

        if find {
            self.want_x = rnd(screen_x - MIN_DIST * 2) + MIN_DIST;
            self.want_y = rnd(screen_y - MIN_DIST * 2) + MIN_DIST;
            self.ang = gang(self.actual_x, self.actual_y, self.want_x, self.want_y);
        }

        for slot in &mut self.stars {
            let dead = match slot {
                Some(stl) => {
                    stl.begin.z -= z_speed;
                    stl.end.z -= z_speed;
                    stl.begin.calc();
                    stl.end.calc();
                    stl.begin.z <= 0 || stl.end.z <= 0
                }
                None => false,
            };
            if dead {
                *slot = None;
            }
        }

        self.changer.move_();

        if self.diameter < self.diameter_change {
            self.diameter += 1;
        }
        if self.diameter > self.diameter_change {
            self.diameter -= 1;
        }
        if rnd(30) == rnd(30) {
            self.diameter_change = rnd(35) + 5;
        }

        for _ in 0..self.add_star {
            self.spawn_star();
        }
    }
}

struct State {
    screen_x: i32,
    screen_y: i32,
    z_speed: i32,
    delay: u32,
    worm: Wormhole,
    gc: Gc,
}

impl State {
    /// Upstream draws into an off-screen pixmap and copies it over, except on
    /// the platforms that double-buffer already. This runtime is one of those,
    /// so paint the window straight.
    fn draw_wormhole(&mut self, d: &mut Dpy) {
        let worm = &self.worm;
        self.gc.set_foreground(worm.black);
        d.win()
            .fill_rectangle(&self.gc, 0, 0, self.screen_x, self.screen_y);

        for stl in worm.stars.iter().flatten() {
            let z = stl.begin.z;
            let color = z * worm.changer.shade_use / worm.max_z;
            let i = (worm.changer.min + color).clamp(0, worm.changer.shade_max - 1) as usize;
            self.gc.set_foreground(worm.changer.shade[i].pixel);
            d.win().draw_line(
                &self.gc,
                stl.begin.calc_x,
                stl.begin.calc_y,
                stl.end.calc_x,
                stl.end.calc_y,
            );
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (screen_x, screen_y) = (d.width(), d.height());
    let make_stars = d.res.int("stars").max(0);
    let mut gc = Gc::default();
    if screen_x > 2560 || screen_y > 2560 {
        gc.line_width = 3; // Retina displays.
    }
    Box::new(State {
        screen_x,
        screen_y,
        z_speed: d.res.int("zspeed").max(1),
        delay: d.res.int("delay").max(0) as u32,
        worm: Wormhole::new(screen_x, screen_y, make_stars),
        gc,
    })
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (sx, sy, zs) = (self.screen_x, self.screen_y, self.z_speed);
        self.worm.move_(sx, sy, zs);
        self.draw_wormhole(d);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        // Upstream only swaps the work pixmap for one of the new size; the
        // tunnel keeps its position and finds its way back on screen.
        self.screen_x = width;
        self.screen_y = height;
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: Black",
    ".foreground: #E9967A",
    "*delay: 10000",
    "*zspeed: 10",
    "*stars: 20",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("zspeed", "Star speed", 1.0, 30.0, 1.0, 0, "10"),
    Opt::slider("stars", "Stars created", 1.0, 100.0, 1.0, 0, "20"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "wormhole",
    label: "Wormhole",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jon Rafkind",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=jGuJU8JKxlI"),
        blurb: "Flying through a colored wormhole in space.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
