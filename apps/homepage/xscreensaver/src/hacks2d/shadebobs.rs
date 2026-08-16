//! Port of `hacks/shadebobs.c`.
//!
//! ```text
//! shadebobs, Copyright (c) 1999 Shane Smit <blackend@inconnect.com>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Module - "shadebobs.c"
//!
//! Description:
//!  There are two little shading circles (bobs) that zip around the screen.
//!  one of them shades up towards white, and the other shades down toward
//!  black.
//!  This keeps the screen in color balance at a chosen color.
//!
//!  Its kinda like 'The Force'
//!   There is a light side, a dark side, and it keeps the world in balance.
//!
//! [05/23/99] - Shane Smit: Creation
//! [05/26/99] - Shane Smit: Port to C/screenhack for use with XScreenSaver
//! [06/11/99] - Shane Smit: Stopped trying to rape the palette.
//! [06/20/99] - jwz: cleaned up ximage handling, gave resoources short names,
//!                introduced delay, made it restart after N iterations.
//! [06/21/99] - Shane Smit: Modified default values slightly, color changes
//!                on cycle, and the extents of the sinus pattern change in
//!                real-time.
//! [06/22/99] - Shane Smit: Fixed delay to be fast and use little CPU :).
//! [09/17/99] - Shane Smit: Made all calculations based on the size of the
//!                window. Thus, it'll look the same at 100x100 as it does at
//!                1600x1200 ( Only smaller :).
//! [04/24/00] - Shane Smit: Revamped entire source code:
//!                Shade Bob movement is calculated very differently.
//!                Base color can be any color now.
//! ```
//!
//! The screen holds a palette index per pixel, running black through a base
//! colour to white. Each bob carries a disc of deltas, brightest in the middle,
//! and adds them to whatever it passes over. Half the bobs carry the negative
//! of that disc, so the screen stays in balance no matter how long they run.
//! Each one steers by a slowly turning angle, which is what draws the loops.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{parse_color, rgb, unrgb};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, StartArgs, random,
};

struct ShadeBob {
    /// The disc of brightness steps this bob adds wherever it goes.
    delta_map: Vec<i8>,
    angle: f64,
    angle_delta: f64,
    angle_inc: f64,
    pos_x: f64,
    pos_y: f64,
}

struct State {
    gc: Gc,
    degree_count: i32,
    sin_table: Vec<f64>,
    cos_table: Vec<f64>,
    win_width: i32,
    win_height: i32,
    color: String,
    bob_radius: i32,
    bob_diameter: i32,
    velocity: i32,
    color_vals: Vec<Pixel>,
    color_count: i32,
    cycles: i32,
    /// The whole screen, kept as a second copy so a bob can read back what is
    /// already there.
    image: Pixmap,
    bobs: Vec<ShadeBob>,
    delay: u32,
    draw_i: i32,
    black: Pixel,
}

/// `RANDOM()`: upstream masks off the sign bit before taking a remainder.
fn rnd(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    ((random() & 0x7FFF_FFFF) as i32) % n
}

impl ShadeBob {
    fn reset(&mut self, width: i32, height: i32, degree_count: i32) {
        self.pos_x = rnd(width) as f64;
        self.pos_y = rnd(height) as f64;
        self.angle = rnd(degree_count) as f64;
        self.angle_delta = rnd(degree_count) as f64 - (degree_count as f64 / 2.0);
        self.angle_inc = self.angle_delta / 50.0;
        if self.angle_inc == 0.0 {
            self.angle_inc = if self.angle_delta > 0.0 {
                0.0001
            } else {
                -0.0001
            };
        }
    }

    fn new(radius: i32, diameter: i32, dark: bool) -> Self {
        let mut delta_map = vec![0i8; (diameter * diameter).max(0) as usize];
        for height in -radius..radius {
            for width in -radius..radius {
                let mut delta = 9.0
                    - ((((width as f64 + 0.5).powi(2) + (height as f64 + 0.5).powi(2)).sqrt()
                        / radius as f64)
                        * 8.0);
                if delta < 0.0 {
                    delta = 0.0;
                }
                if dark {
                    delta = -delta;
                }
                let i = (width + radius) * diameter + height + radius;
                delta_map[i as usize] = delta as i8;
            }
        }
        Self {
            delta_map,
            angle: 0.0,
            angle_delta: 0.0,
            angle_inc: 0.0,
            pos_x: 0.0,
            pos_y: 0.0,
        }
    }
}

impl State {
    /// A delta is calculated, and the shadebob turns at an increment. When the
    /// delta falls to 0, a new delta and increment are calculated.
    fn move_bob(&mut self, b: usize) {
        let degree_count = self.degree_count;
        let bob = &mut self.bobs[b];
        bob.angle += bob.angle_inc;
        bob.angle_delta -= bob.angle_inc;

        // Since it can happen that angle < 0 and angle + degree_count >=
        // degree_count on floating point, we set some marginal value.
        if bob.angle + 0.5 >= degree_count as f64 {
            bob.angle -= degree_count as f64;
        } else if bob.angle < -0.5 {
            bob.angle += degree_count as f64;
        }

        if (bob.angle_inc > 0.0 && bob.angle_delta < bob.angle_inc)
            || (bob.angle_inc <= 0.0 && bob.angle_delta > bob.angle_inc)
        {
            bob.angle_delta = rnd(degree_count) as f64 - (degree_count as f64 / 2.0);
            bob.angle_inc = bob.angle_delta / 50.0;
            if bob.angle_inc == 0.0 {
                bob.angle_inc = if bob.angle_delta > 0.0 {
                    0.0001
                } else {
                    -0.0001
                };
            }
        }

        let idx = (bob.angle as i32).clamp(0, degree_count - 1) as usize;
        bob.pos_x += self.sin_table[idx] * self.velocity as f64;
        bob.pos_y += self.cos_table[idx] * self.velocity as f64;

        // This wraps it around the screen.
        if bob.pos_x >= self.win_width as f64 {
            bob.pos_x -= self.win_width as f64;
        } else if bob.pos_x < 0.0 {
            bob.pos_x += self.win_width as f64;
        }
        if bob.pos_y >= self.win_height as f64 {
            bob.pos_y -= self.win_height as f64;
        } else if bob.pos_y < 0.0 {
            bob.pos_y += self.win_height as f64;
        }
    }

    fn execute(&mut self, d: &mut Dpy, b: usize) {
        self.move_bob(b);
        let diameter = self.bob_diameter;
        let (px, py) = (self.bobs[b].pos_x as i32, self.bobs[b].pos_y as i32);

        for height in 0..diameter {
            let mut pixel_y = py + height;
            if pixel_y >= self.win_height {
                pixel_y -= self.win_height;
            }
            for width in 0..diameter {
                let mut pixel_x = px + width;
                if pixel_x >= self.win_width {
                    pixel_x -= self.win_width;
                }

                let color = self.image.get_pixel(pixel_x, pixel_y);
                // Upstream calls this loop the one it would love to take out:
                // the screen stores colours, so the index has to be found by
                // searching the palette for the one that is there.
                let mut color_val = self
                    .color_vals
                    .iter()
                    .position(|c| *c == color)
                    .map(|i| i as i32)
                    .unwrap_or(self.color_count);

                color_val += self.bobs[b].delta_map[(width * diameter + height) as usize] as i32;
                color_val = color_val.clamp(0, self.color_count - 1);

                self.image
                    .put_pixel(pixel_x, pixel_y, self.color_vals[color_val as usize]);
            }
        }

        // Upstream notes this breaks next to the top or left of the screen.
        // However, it is not noticeable.
        d.win()
            .copy_area(&self.gc, &self.image, px, py, diameter, diameter, px, py);
    }

    fn create_tables(&mut self) {
        let n = self.degree_count.max(1) as usize;
        self.sin_table = Vec::with_capacity(n);
        self.cos_table = Vec::with_capacity(n);
        for i in 0..n {
            let radian = (2.0 * i as f64 / n as f64) * std::f64::consts::PI;
            self.sin_table.push(radian.sin());
            self.cos_table.push(radian.cos());
        }
    }

    /// A ramp of `ncolors` from black up through the base colour and on to
    /// white, so a bob adding to a pixel brightens it and one subtracting
    /// darkens it along the same line.
    fn set_palette(&mut self, d: &mut Dpy) {
        let mut base = (rnd(0xFFFF) as f64, rnd(0xFFFF) as f64, rnd(0xFFFF) as f64);
        let named = if self.color.eq_ignore_ascii_case("random") {
            None
        } else {
            parse_color(&self.color)
        };
        if let Some(p) = named {
            let (r, g, b) = unrgb(p);
            base = (
                (((r as u16) << 8) | r as u16) as f64,
                (((g as u16) << 8) | g as u16) as f64,
                (((b as u16) << 8) | b as u16) as f64,
            );
        }

        self.color_count = d.res.int("ncolors").clamp(2, 255);
        let half = self.color_count as f64 / 2.0;
        self.color_vals = (0..self.color_count)
            .map(|i| {
                let f = i as f64;
                let (r, g, b) = if i < self.color_count / 2 {
                    // Black to base colour.
                    (base.0 / half * f, base.1 / half * f, base.2 / half * f)
                } else {
                    // Base colour to white.
                    (
                        ((0xFFFF as f64 - base.0) / half) * (f - half) + base.0,
                        ((0xFFFF as f64 - base.1) / half) * (f - half) + base.1,
                        ((0xFFFF as f64 - base.2) / half) * (f - half) + base.2,
                    )
                };
                rgb(
                    ((r as u16) >> 8) as u8,
                    ((g as u16) >> 8) as u8,
                    ((b as u16) >> 8) as u8,
                )
            })
            .collect();
    }

    fn initialize(&mut self, d: &mut Dpy) {
        self.win_width = d.width();
        self.win_height = d.height();
        self.image = Pixmap::new(self.win_width, self.win_height);

        // These are precalculations used in execute().
        self.bob_diameter = self.win_width.min(self.win_height) / 25;
        self.bob_radius = self.bob_diameter / 2;
        self.velocity = self.win_width.min(self.win_height) / 150;

        // Create the sin and cosine lookup tables.
        self.degree_count = d.res.int("degrees");
        self.degree_count = if self.degree_count == 0 {
            (self.win_width / 6) + 400
        } else {
            self.degree_count.clamp(90, 5400)
        };
        self.create_tables();
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let black = d.res.pixel("background");
    let mut st = State {
        gc: Gc::new(d.res.pixel("foreground"), black),
        degree_count: 0,
        sin_table: Vec::new(),
        cos_table: Vec::new(),
        win_width: 0,
        win_height: 0,
        color: d.res.string("color").to_string(),
        bob_radius: 0,
        bob_diameter: 0,
        velocity: 0,
        color_vals: Vec::new(),
        color_count: 0,
        cycles: 0,
        image: Pixmap::new(1, 1),
        bobs: Vec::new(),
        delay: d.res.int("delay").max(0) as u32,
        // Forces a reset, and with it a palette, on the very first frame.
        draw_i: i32::MAX,
        black,
    };

    let count = d.res.int("count").clamp(1, 64) as usize;
    st.initialize(d);
    st.bobs = (0..count)
        .map(|i| ShadeBob::new(st.bob_radius, st.bob_diameter, i % 2 == 1))
        .collect();
    st.cycles = d.res.int("cycles").max(0) * st.degree_count;

    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let i = self.draw_i;
        self.draw_i = self.draw_i.saturating_add(1);
        if i >= self.cycles {
            self.draw_i = 0;
            // Fill the image with the actual value of the black pixel, not 0.
            self.image.clear(self.black);
            let (w, h, deg) = (self.win_width, self.win_height, self.degree_count);
            for b in &mut self.bobs {
                b.reset(w, h, deg);
            }
            self.set_palette(d);
            d.clear_window();
        }

        for b in 0..self.bobs.len() {
            self.execute(d, b);
        }
        self.delay
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    // Default: automatic degree calculation.
    "*degrees: 0",
    "*color: random",
    "*count: 4",
    "*cycles: 10",
    // Changing this does not work particularly well.
    "*ncolors: 64",
    "*delay: 10000",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 20_000.0, 500.0, 0, "10000").inverted(),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
    Opt::slider("count", "Count", 1.0, 20.0, 1.0, 0, "4"),
    Opt::slider("cycles", "Duration", 0.0, 100.0, 1.0, 0, "10"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "shadebobs",
    label: "Shade Bobs",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Shane Smit",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=gJtxnQ5_8Zk"),
        blurb: "Oscillating oval patterns that look something like vapor trails or neon tubes.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
