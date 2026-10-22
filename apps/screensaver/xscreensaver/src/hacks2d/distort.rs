//! Port of `hacks/distort.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2018 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! distort
//! by Jonas Munsin (jmunsin@iki.fi) and Jamie Zawinski <jwz@jwz.org>
//!
//! 28 Sep 1999 Jonas Munsin (jmunsin@iki.fi)
//!    Added about 10x faster algortim for 8, 16 and 32 bpp (modifies pixels
//!    directly avoiding costly XPutPixle(XGetPixel()) calls, inspired by
//!    xwhirl made by horvai@clipper.ens.fr (Peter Horvai) and the XFree86
//!    Xlib sources.
//! 08 Oct 1999 Jonas Munsin (jmunsin@iki.fi)
//!    Corrected several bugs causing references beyond allocated memory.
//! 09 Oct 2016 Dave Odell (dmo2118@gmail.com)
//!  Updated for new xshm.c.
//! ```
//!
//! Lenses wander over a picture. A lens is not code: it is a square table of
//! "where each pixel should be read from instead", built once when the lens is
//! made and then used as a lookup for every frame. Everything the hack can do,
//! it does by choosing what goes in that table.
//!
//! Inside the lens, a pixel at distance `r` from the centre reads from `r`
//! scaled by the sine of how far out it is, which bulges the middle towards
//! you. Take the reciprocal of that scale and the bulge inverts into a black
//! hole. Add a rotation that falls off with distance and it becomes a vortex,
//! the formula for which was borrowed from the whirl plugin for the Gimp. The
//! reflect mode is the odd one out: it is not a table at all but a circular
//! inversion computed per pixel, which is what makes it look like a glass ball
//! rather than a lens.
//!
//! One upstream detail is not reproduced. In swamp mode the lens grows and
//! shrinks, and upstream builds one table per radius to animate it, but the
//! fast drawing path it takes on any modern display reads a single table that
//! the last of those overwrote, so the lens never actually changes size. Here
//! each radius keeps its own table, and swamp swamps.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::BLACK;
use crate::runtime::{
    About, Dpy, ImageLoad, Opt, Pixel, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent,
    XImage, random, screenhack_event_helper,
};

/// A lens: where it is, how big, and which way it is going.
#[derive(Clone, Copy, Default)]
struct Coo {
    x: i32,
    y: i32,
    r: i32,
    r_change: i32,
    xmove: i32,
    ymove: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Effect {
    /// Bounce the lens around the picture.
    MoveLense,
    /// Grow and shrink it in place.
    SwampThing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawKind {
    Plain,
    Reflect,
}

struct State {
    xy_coo: [Coo; 10],
    delay: u32,
    radius: i32,
    speed: i32,
    number: usize,
    blackhole: bool,
    vortex: bool,
    magnify: bool,
    reflect: bool,
    duration: f64,
    start_time: f64,
    width: i32,
    height: i32,
    black_pixel: Pixel,
    /// The picture the lenses look at.
    orig: Option<XImage>,
    /// One lookup table per lens radius. Only swamp mode uses more than one.
    fast_from: Vec<Vec<i32>>,
    effect: Effect,
    draw_kind: DrawKind,
    img_loader: Option<ImageLoad>,
    loading: bool,
}

impl State {
    /// The side of a lens table.
    fn side(&self) -> i32 {
        2 * self.radius + self.speed + 2
    }

    /// Build the table for a lens of the given radius, magnifying out to
    /// `loop_r`. Everything outside that just reads straight through.
    fn make_round_lense(&self, loop_r: i32) -> Vec<i32> {
        let n = self.side();
        let radius = self.radius as f64;
        let mut out = vec![0i32; (n * n) as usize];

        for i in 0..n {
            for j in 0..n {
                let mut r = (((i - self.radius) * (i - self.radius)
                    + (j - self.radius) * (j - self.radius)) as f64)
                    .sqrt();
                let d = if loop_r == 0 { 0.0 } else { r / loop_r as f64 };

                let (fx, fy);
                if r < (loop_r - 1) as f64 {
                    if self.vortex {
                        // This one-line formula for getting a nice rotation
                        // angle is borrowed (with permission) from the whirl
                        // plugin for the Gimp, Copyright (C) 1996 Federico
                        // Mena Quintero. Five is a constant used because it
                        // looks good.
                        let angle = 5.0 * (1.0 - d) * (1.0 - d);
                        let (mut ax, mut ay);
                        // Avoid atan2's DOMAIN error message.
                        if radius - j as f64 == 0.0 && radius - i as f64 == 0.0 {
                            ax = (radius + angle.cos() * r) as i32;
                            ay = (radius + angle.sin() * r) as i32;
                        } else {
                            let t = (radius - j as f64).atan2(-(radius - i as f64));
                            ax = (radius + (angle - t).cos() * r) as i32;
                            ay = (radius + (angle - t).sin() * r) as i32;
                        }
                        if self.magnify {
                            r = (d * std::f64::consts::FRAC_PI_2).sin();
                            if self.blackhole && r != 0.0 {
                                r = 1.0 / r;
                            }
                            // Upstream reads back the truncated integers here,
                            // so the rounding happens twice.
                            ax = (radius + (ax as f64 - radius) * r) as i32;
                            ay = (radius + (ay as f64 - radius) * r) as i32;
                        }
                        fx = ax;
                        fy = ay;
                    } else {
                        // The default is to magnify. Raising r to a different
                        // power gives different amounts of distortion; a
                        // reciprocal sucks everything into a black hole.
                        r = (d * std::f64::consts::FRAC_PI_2).sin();
                        if self.blackhole && r != 0.0 {
                            r = 1.0 / r;
                        }
                        fx = (radius + (i as f64 - radius) * r) as i32;
                        fy = (radius + (j as f64 - radius) * r) as i32;
                    }
                } else {
                    fx = i;
                    fy = j;
                }

                let flat = fx + self.width * fy;
                out[(i + j * n) as usize] = if flat < 0 || flat >= self.width * self.height {
                    0
                } else {
                    flat
                };
            }
        }
        out
    }

    fn init_round_lense(&mut self) {
        self.fast_from = if self.effect == Effect::SwampThing {
            (0..=self.radius)
                .map(|k| self.make_round_lense(k))
                .collect()
        } else {
            vec![self.make_round_lense(self.radius)]
        };
    }

    /// Look every pixel of the lens box up in the table and copy it over.
    fn plain_draw(&mut self, d: &mut Dpy, k: usize) {
        let Some(orig) = self.orig.take() else { return };
        let n = self.side();
        let (x, y) = (self.xy_coo[k].x, self.xy_coo[k].y);
        if x + n <= orig.width() && y + n <= orig.height() {
            let table = if self.fast_from.len() > 1 {
                &self.fast_from[(self.xy_coo[k].r.clamp(0, self.radius)) as usize]
            } else {
                &self.fast_from[0]
            };
            let base = x + y * self.width;
            let limit = self.width * self.height;
            let src = orig.pixels();
            for row in 0..n {
                for col in 0..n {
                    let o = (base + table[(col + row * n) as usize]).clamp(0, limit - 1);
                    d.win().put_pixel(x + col, y + row, src[o as usize]);
                }
            }
        }
        self.orig = Some(orig);
    }

    /// A circular inversion rather than a table: everything inside the circle
    /// is read from the reciprocal of its distance to the centre, which is
    /// what makes it look like a glass ball.
    ///
    /// From the reflect algorithm submitted by Randy Zack <randy@acucorp.com>.
    fn reflect_draw(&mut self, d: &mut Dpy, k: usize) {
        let Some(orig) = self.orig.take() else { return };
        let n = self.side();
        let rsq = self.radius * self.radius;
        let mut cx = self.radius;
        let mut cy = self.radius;
        if self.xy_coo[k].ymove > 0 {
            cy += self.speed;
        }
        if self.xy_coo[k].xmove > 0 {
            cx += self.speed;
        }
        let (ox, oy) = (self.xy_coo[k].x, self.xy_coo[k].y);

        for i in 0..n {
            let ly = i - cy;
            let lysq = ly * ly;
            let ny = (oy + i).min(orig.height() - 1);
            for j in 0..n {
                let lx = j - cx;
                let dist = lx * lx + lysq;
                let p = if dist > rsq
                    || ly < -self.radius
                    || ly > self.radius
                    || lx < -self.radius
                    || lx > self.radius
                {
                    orig.get_pixel(ox + j, ny)
                } else if dist == 0 {
                    self.black_pixel
                } else {
                    let x = ox + cx + (lx * rsq / dist);
                    let y = oy + cy + (ly * rsq / dist);
                    if x < 0 || x >= self.width || y < 0 || y >= self.height {
                        self.black_pixel
                    } else {
                        orig.get_pixel(x, y)
                    }
                };
                d.win().put_pixel(ox + j, oy + i, p);
            }
        }
        self.orig = Some(orig);
    }

    /// A new position that does not overlap any other lens, because the
    /// drawing routines would be much slower if they had to handle several
    /// layers of distortion.
    fn new_rnd_coo(&mut self, k: usize) {
        let sx = (self.width - 2 * self.radius).max(1) as u32;
        let sy = (self.height - 2 * self.radius).max(1) as u32;
        self.xy_coo[k].x = (random() % sx) as i32;
        self.xy_coo[k].y = (random() % sy) as i32;

        let mut loop_guard = 0;
        let mut i = 0;
        while i < self.number {
            if i != k
                && (self.xy_coo[k].x - self.xy_coo[i].x).abs() <= self.side()
                && (self.xy_coo[k].y - self.xy_coo[i].y).abs() <= self.side()
            {
                self.xy_coo[k].x = (random() % sx) as i32;
                self.xy_coo[k].y = (random() % sy) as i32;
                i = 0;
                loop_guard += 1;
                if loop_guard > 1000 {
                    return; // Let us not get stuck.
                }
                continue;
            }
            i += 1;
            loop_guard += 1;
            if loop_guard > 1000 {
                return;
            }
        }
    }

    /// Move a lens, and handle bounces with the walls and the other lenses.
    fn move_lense(&mut self, k: usize) {
        if self.xy_coo[k].x + self.side() >= self.width {
            self.xy_coo[k].xmove = -self.xy_coo[k].xmove.abs();
        }
        if self.xy_coo[k].x <= self.speed {
            self.xy_coo[k].xmove = self.xy_coo[k].xmove.abs();
        }
        if self.xy_coo[k].y + self.side() >= self.height {
            self.xy_coo[k].ymove = -self.xy_coo[k].ymove.abs();
        }
        if self.xy_coo[k].y <= self.speed {
            self.xy_coo[k].ymove = self.xy_coo[k].ymove.abs();
        }

        self.xy_coo[k].x += self.xy_coo[k].xmove;
        self.xy_coo[k].y += self.xy_coo[k].ymove;

        // Bounce against other lenses. The test is for circular lenses; the
        // rectangular one upstream keeps commented out beside it.
        for i in 0..self.number {
            let dx = self.xy_coo[k].x - self.xy_coo[i].x;
            let dy = self.xy_coo[k].y - self.xy_coo[i].y;
            if i != k && dx * dx + dy * dy <= 4 * self.radius * self.radius {
                let (x, y) = (self.xy_coo[k].xmove, self.xy_coo[k].ymove);
                self.xy_coo[k].xmove = self.xy_coo[i].xmove;
                self.xy_coo[k].ymove = self.xy_coo[i].ymove;
                self.xy_coo[i].xmove = x;
                self.xy_coo[i].ymove = y;
            }
        }
    }

    /// Make a lens grow and shrink in place, moving on when it vanishes.
    fn swamp_thing(&mut self, d: &mut Dpy, k: usize) {
        if self.xy_coo[k].r >= self.radius {
            self.xy_coo[k].r_change = -self.xy_coo[k].r_change.abs();
        }
        if self.xy_coo[k].r <= 0 {
            self.xy_coo[k].r = 0;
            self.plain_draw(d, k);
            self.xy_coo[k].r_change = self.xy_coo[k].r_change.abs();
            self.new_rnd_coo(k);
            self.xy_coo[k].r = self.xy_coo[k].r_change;
            return;
        }
        self.xy_coo[k].r += self.xy_coo[k].r_change;
        self.xy_coo[k].r = self.xy_coo[k].r.clamp(0, self.radius);
    }

    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        self.start_time = d.time;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    fn image_arrived(&mut self, d: &mut Dpy) {
        let (w, h) = (self.width, self.height);
        self.orig = Some(d.win_ref().sub_image(0, 0, w, h));
        self.start_time = d.time;
        self.loading = false;

        self.init_round_lense();
        for i in 0..self.number {
            self.new_rnd_coo(i);
            // "Randomize" the initial values a bit.
            self.xy_coo[i].r = if self.number != 1 {
                (i as i32 * self.radius) / (self.number as i32 - 1)
            } else {
                0
            };
            let s = self.speed - (i % 2) as i32 * 2 * self.speed;
            self.xy_coo[i].r_change = s;
            self.xy_coo[i].xmove = s;
            self.xy_coo[i].ymove = s;
        }
    }

    /// Read the knobs, and if none of them were touched pick one of the
    /// twelve settings upstream lists as looking good.
    fn reset(&mut self, d: &Dpy) {
        self.delay = d.res.int("delay").max(0) as u32;
        self.duration = d.res.int("duration").max(1) as f64;
        self.radius = d.res.int("radius");
        self.speed = d.res.int("speed");
        self.number = d.res.int("number").max(0) as usize;
        self.blackhole = d.res.bool("blackhole");
        self.vortex = d.res.bool("vortex");
        self.magnify = d.res.bool("magnify");
        self.reflect = d.res.bool("reflect");

        let s = d.res.string("effect");
        let mut effect = match s {
            "swamp" => Some(Effect::SwampThing),
            "bounce" => Some(Effect::MoveLense),
            _ => None,
        };
        self.draw_kind = DrawKind::Plain;

        if effect.is_none()
            && self.radius == 0
            && self.speed == 0
            && self.number == 0
            && !self.blackhole
            && !self.vortex
            && !self.magnify
            && !self.reflect
        {
            effect = Some(Effect::MoveLense);
            match random() % 12 {
                0 => {
                    self.radius = 125;
                    self.number = 4;
                    self.speed = 1;
                }
                1 => {
                    self.radius = 125;
                    self.number = 4;
                    self.speed = 1;
                    self.blackhole = true;
                }
                2 => {
                    self.radius = 125;
                    self.number = 4;
                    self.speed = 1;
                    self.vortex = true;
                }
                3 => {
                    self.radius = 125;
                    self.number = 4;
                    self.speed = 1;
                    self.vortex = true;
                    self.magnify = true;
                }
                4 => {
                    self.radius = 125;
                    self.number = 4;
                    self.speed = 1;
                    self.vortex = true;
                    self.magnify = true;
                    self.blackhole = true;
                }
                5 => {
                    self.radius = 250;
                    self.number = 1;
                    self.speed = 2;
                }
                6 => {
                    self.radius = 250;
                    self.number = 1;
                    self.speed = 2;
                    self.blackhole = true;
                }
                7 => {
                    self.radius = 250;
                    self.number = 1;
                    self.speed = 2;
                    self.vortex = true;
                }
                8 => {
                    self.radius = 250;
                    self.number = 1;
                    self.speed = 2;
                    self.vortex = true;
                    self.magnify = true;
                }
                9 => {
                    self.radius = 250;
                    self.number = 1;
                    self.speed = 2;
                    self.vortex = true;
                    self.magnify = true;
                    self.blackhole = true;
                }
                10 => {
                    self.radius = 80;
                    self.number = 1;
                    self.speed = 2;
                    self.reflect = true;
                    self.draw_kind = DrawKind::Reflect;
                }
                _ => {
                    self.radius = 125;
                    self.number = 4;
                    self.speed = 2;
                    self.reflect = true;
                    self.draw_kind = DrawKind::Reflect;
                }
            }
        }

        if self.width > 2560 || self.height > 2560 {
            self.radius *= 2; // Retina displays.
        }

        // Swamp mode keeps one table per radius, so throttle the radius.
        if effect == Some(Effect::SwampThing) && self.radius > 60 {
            self.radius = 60;
        }

        // Never allow the radius to be too close to the min window dimension.
        {
            let max = self.width.min(self.height) * 2 / (5 * self.number.max(1) as i32);
            if self.radius > max {
                self.radius = max;
            }
        }

        if self.radius <= 0 {
            self.radius = 60;
        }
        if self.speed <= 0 {
            self.speed = 2;
        }
        if self.number == 0 || self.number >= 10 {
            self.number = 1;
        }
        self.effect = effect.unwrap_or(Effect::MoveLense);
        if self.reflect {
            self.draw_kind = DrawKind::Reflect;
            self.effect = Effect::MoveLense;
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut st = State {
        xy_coo: [Coo::default(); 10],
        delay: 20000,
        radius: 0,
        speed: 0,
        number: 0,
        blackhole: false,
        vortex: false,
        magnify: false,
        reflect: false,
        duration: 120.0,
        start_time: 0.0,
        width: d.width(),
        height: d.height(),
        black_pixel: BLACK,
        orig: None,
        fast_from: Vec::new(),
        effect: Effect::MoveLense,
        draw_kind: DrawKind::Plain,
        img_loader: None,
        loading: false,
    };
    st.reset(d);
    st.start_load(d);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.loading {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return self.delay;
        }

        if self.start_time + self.duration < d.time {
            self.start_load(d);
            return self.delay;
        }

        for k in 0..self.number {
            match self.effect {
                Effect::MoveLense => self.move_lense(k),
                Effect::SwampThing => self.swamp_thing(d, k),
            }
            match self.draw_kind {
                DrawKind::Plain => self.plain_draw(d, k),
                DrawKind::Reflect => self.reflect_draw(d, k),
            }
        }
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.start_load(d);
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.reset(d);
            self.start_load(d);
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    "*dontClearRoot: True",
    "*background: Black",
    "*fpsSolid: true",
    "*delay: 20000",
    "*duration: 120",
    "*radius: 0",
    "*speed: 0",
    "*number: 0",
    "*slow: False",
    "*vortex: False",
    "*magnify: False",
    "*reflect: False",
    "*blackhole: False",
    "*effect: none",
];

const EFFECTS: &[SelectItem] = &[
    SelectItem {
        value: "none",
        label: "Normal",
    },
    SelectItem {
        value: "swamp",
        label: "Swamp thing",
    },
    SelectItem {
        value: "bounce",
        label: "Bounce",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 200_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::slider("radius", "Lens size", 0.0, 1000.0, 10.0, 0, "0"),
    Opt::spin("number", "Lens count", 0.0, 10.0, "0"),
    Opt::select("effect", "Effect", EFFECTS, "none"),
    Opt::boolean("reflect", "Reflect", "false"),
    Opt::boolean("magnify", "Magnify", "false"),
    Opt::boolean("blackhole", "Black hole", "false"),
    Opt::boolean("vortex", "Vortex", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "distort",
    label: "Distort",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jonas Munsin",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=ENaG3gwtukM"),
        blurb: "Wandering lenses distort an image in various ways.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
