//! Port of `hacks/substrate.c`.
//!
//! ```text
//!  Substrate (dragorn@kismetwireless.net)
//!  Directly ported code from complexification.net Substrate art
//!  http://complexification.net/gallery/machines/substrate/applet_s/substrate_s.pde
//!
//!  Substrate code:
//!  j.tarbell   June, 2004
//!  Albuquerque, New Mexico
//!  complexification.net
//!
//!  CHANGES
//!
//!  1.1  dragorn  Jan 04 2005    Fixed some indenting, typo in errors for parsing
//!                                cmdline args
//!  1.1  dagraz   Jan 04 2005    Added option for circular cracks (David Agraz)
//!                               Cleaned up issues with timeouts in start_crack (DA)
//!  1.0  dragorn  Oct 10 2004    First port done
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
//! One rule, applied over and over: a crack travels in a straight line until
//! it meets another crack, and where it stops two new cracks start, at right
//! angles to whatever they landed on. Nothing decides the composition. The
//! city-like plan of rooms and alleys is what a plane looks like after that
//! rule has been applied a few thousand times.
//!
//! Behind each crack a sand painter drags a fan of translucent grains
//! sideways until it reaches the next crack, which is what turns the bare
//! diagram into something that looks painted. The grains are laid down at a
//! tenth of an alpha and fade to nothing across the fan, so the tone of a
//! region is built from hundreds of passes rather than filled in.
//!
//! The palette is a hundred and twenty-two colours lifted from a scan of a
//! Pollock, which is why nothing in it is saturated.
//!
//! Alpha blending needs to read back what is already on screen, and upstream
//! keeps its own shadow copy of the window to read from rather than trusting
//! the server. That copy is kept here too: reading the framebuffer directly
//! would give a different picture, because the crack lines are drawn into the
//! window but into the shadow as a flat foreground colour.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, rgb, unrgb};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, frand, random,
    random_below, screenhack_event_helper,
};

const STEP: f32 = 0.42;

/// Raw colormap extracted from `pollockEFF.gif`.
static RGB_COLORMAP: [Pixel; 122] = [
    rgb(0x20, 0x1F, 0x21),
    rgb(0x26, 0x2C, 0x2E),
    rgb(0x35, 0x26, 0x26),
    rgb(0x37, 0x2B, 0x27),
    rgb(0x30, 0x2C, 0x2E),
    rgb(0x39, 0x2B, 0x2D),
    rgb(0x32, 0x32, 0x29),
    rgb(0x3F, 0x32, 0x29),
    rgb(0x38, 0x32, 0x2E),
    rgb(0x2E, 0x33, 0x3D),
    rgb(0x33, 0x3A, 0x3D),
    rgb(0x47, 0x33, 0x29),
    rgb(0x40, 0x39, 0x2C),
    rgb(0x40, 0x39, 0x2E),
    rgb(0x47, 0x40, 0x2C),
    rgb(0x47, 0x40, 0x2E),
    rgb(0x4E, 0x40, 0x2C),
    rgb(0x4F, 0x40, 0x2E),
    rgb(0x4E, 0x47, 0x38),
    rgb(0x58, 0x40, 0x37),
    rgb(0x65, 0x47, 0x2D),
    rgb(0x6D, 0x5D, 0x3D),
    rgb(0x74, 0x55, 0x30),
    rgb(0x75, 0x55, 0x32),
    rgb(0x74, 0x5D, 0x32),
    rgb(0x74, 0x64, 0x33),
    rgb(0x7C, 0x6C, 0x36),
    rgb(0x52, 0x31, 0x52),
    rgb(0x44, 0x48, 0x42),
    rgb(0x4C, 0x56, 0x47),
    rgb(0x65, 0x5D, 0x45),
    rgb(0x6D, 0x5D, 0x44),
    rgb(0x6C, 0x5D, 0x4E),
    rgb(0x74, 0x6C, 0x43),
    rgb(0x7C, 0x6C, 0x42),
    rgb(0x7C, 0x6C, 0x4B),
    rgb(0x6B, 0x73, 0x4B),
    rgb(0x73, 0x73, 0x4B),
    rgb(0x7B, 0x7B, 0x4A),
    rgb(0x6B, 0x6C, 0x55),
    rgb(0x69, 0x6D, 0x5E),
    rgb(0x7B, 0x6C, 0x5D),
    rgb(0x6B, 0x73, 0x53),
    rgb(0x6A, 0x74, 0x5D),
    rgb(0x72, 0x7B, 0x52),
    rgb(0x7B, 0x7B, 0x52),
    rgb(0x57, 0x74, 0x6E),
    rgb(0x68, 0x74, 0x66),
    rgb(0x9C, 0x54, 0x2B),
    rgb(0x9D, 0x54, 0x32),
    rgb(0x9D, 0x5B, 0x35),
    rgb(0x93, 0x6B, 0x36),
    rgb(0xAA, 0x73, 0x30),
    rgb(0xC4, 0x5A, 0x27),
    rgb(0xD9, 0x52, 0x23),
    rgb(0xD8, 0x5A, 0x20),
    rgb(0xDB, 0x5A, 0x23),
    rgb(0xE5, 0x70, 0x37),
    rgb(0x83, 0x6C, 0x4B),
    rgb(0x8C, 0x6B, 0x4B),
    rgb(0x82, 0x73, 0x5C),
    rgb(0x93, 0x73, 0x52),
    rgb(0x81, 0x7B, 0x63),
    rgb(0x81, 0x7B, 0x6D),
    rgb(0x92, 0x7B, 0x63),
    rgb(0xD9, 0x89, 0x3B),
    rgb(0xE4, 0x98, 0x32),
    rgb(0xDF, 0xA1, 0x33),
    rgb(0xE5, 0xA0, 0x37),
    rgb(0xF0, 0xAB, 0x3B),
    rgb(0x8A, 0x8A, 0x59),
    rgb(0xB2, 0x9A, 0x58),
    rgb(0x89, 0x82, 0x6B),
    rgb(0x9A, 0x82, 0x62),
    rgb(0x88, 0x8B, 0x7C),
    rgb(0x90, 0x9A, 0x7A),
    rgb(0xA2, 0x82, 0x62),
    rgb(0xA1, 0x8A, 0x69),
    rgb(0xA9, 0x99, 0x68),
    rgb(0x99, 0xA1, 0x60),
    rgb(0x99, 0xA1, 0x68),
    rgb(0xCA, 0x81, 0x48),
    rgb(0xEB, 0x8D, 0x43),
    rgb(0xC2, 0x91, 0x60),
    rgb(0xC2, 0x91, 0x68),
    rgb(0xD1, 0xA9, 0x77),
    rgb(0xC9, 0xB9, 0x7F),
    rgb(0xF0, 0xE2, 0x7B),
    rgb(0x9F, 0x92, 0x8B),
    rgb(0xC0, 0xB9, 0x99),
    rgb(0xE6, 0xB8, 0x8F),
    rgb(0xC8, 0xC1, 0x87),
    rgb(0xE0, 0xC8, 0x86),
    rgb(0xF2, 0xCC, 0x85),
    rgb(0xF5, 0xDA, 0x83),
    rgb(0xEC, 0xDE, 0x9D),
    rgb(0xF5, 0xD2, 0x94),
    rgb(0xF5, 0xDA, 0x94),
    rgb(0xF4, 0xE7, 0x84),
    rgb(0xF4, 0xE1, 0x8A),
    rgb(0xF4, 0xE1, 0x93),
    rgb(0xE7, 0xD8, 0xA7),
    rgb(0xF1, 0xD4, 0xA5),
    rgb(0xF1, 0xDC, 0xA5),
    rgb(0xF4, 0xDB, 0xAD),
    rgb(0xF1, 0xDC, 0xAE),
    rgb(0xF4, 0xDB, 0xB5),
    rgb(0xF5, 0xDB, 0xBD),
    rgb(0xF4, 0xE2, 0xAD),
    rgb(0xF5, 0xE9, 0xAD),
    rgb(0xF4, 0xE3, 0xBE),
    rgb(0xF5, 0xEA, 0xBE),
    rgb(0xF7, 0xF0, 0xB6),
    rgb(0xD9, 0xD1, 0xC1),
    rgb(0xE0, 0xD0, 0xC0),
    rgb(0xE7, 0xD8, 0xC0),
    rgb(0xF1, 0xDD, 0xC6),
    rgb(0xE8, 0xE1, 0xC0),
    rgb(0xF3, 0xED, 0xC7),
    rgb(0xF6, 0xEC, 0xCE),
    rgb(0xF8, 0xF2, 0xC7),
    rgb(0xEF, 0xEF, 0xD0),
];

/// A crack, and the sand painter that trails it.
#[derive(Clone, Copy, Default)]
struct Crack {
    x: f32,
    y: f32,
    /// Heading, in degrees. Not reduced to a circle: it is read out of the
    /// crack grid, which holds ten thousand and one for empty, so a fresh
    /// crack sets off at about that many degrees.
    t: f32,
    /// For curvature calculations.
    ys: f32,
    xs: f32,
    t_inc: f32,
    curved: bool,
    sandcolor: Pixel,
    sandp: f32,
    sandg: f32,
    degrees_drawn: f32,
}

/// The value the crack grid holds where no crack has been.
const EMPTY: i32 = 10001;

struct State {
    width: i32,
    height: i32,
    initial_cracks: i32,
    max_num: usize,
    /// Number of grains in the sand painting.
    grains: i32,
    circle_percent: i32,
    cracks: Vec<Crack>,
    /// Grid of actual crack placement, one cell per pixel.
    cgrid: Vec<i32>,
    /// Raw map of pixels we need to keep for alpha blending.
    off_img: Vec<Pixel>,
    fgcolor: Pixel,
    bgcolor: Pixel,
    cycles: u32,
    max_cycles: u32,
    wireframe: bool,
    seamless: bool,
    growth_delay: u32,
    gc: Gc,
}

impl State {
    fn cgrid_at(&self, x: i32, y: i32) -> i32 {
        self.cgrid[(y * self.width + x) as usize]
    }

    /// Synthesis of `Crack::findStart()` and `Crack::startCrack()`.
    fn start_crack(&mut self, i: usize) {
        let mut px = 0;
        let mut py = 0;
        let mut found = false;

        // Shift until a crack is found to grow out of. Early on there is
        // nothing marked at all, so this spins its whole budget and falls
        // through to the crack's own position.
        let mut timeout = 0;
        while !found && timeout < 10000 {
            timeout += 1;
            px = random_below(self.width);
            py = random_below(self.height);
            if self.cgrid_at(px, py) < 10000 {
                found = true;
            }
        }

        if !found {
            // We timed out. Use our default values.
            let cr = self.cracks[i];
            px = (cr.x as i32).clamp(0, self.width - 1);
            py = (cr.y as i32).clamp(0, self.height - 1);
            let t = cr.t;
            self.cgrid[(py * self.width + px) as usize] = t as i32;
        }

        // Start a crack.
        let mut a = self.cgrid_at(px, py) as f32;

        // Conversion of the java int(random(-2, 2.1)).
        if random_below(100) < 50 {
            a -= 90.0 + (frand(4.1) as f32 - 2.0);
        } else {
            a += 90.0 + (frand(4.1) as f32 - 2.0);
        }

        let cr = &mut self.cracks[i];
        if random_below(100) < self.circle_percent {
            cr.curved = true;
            cr.degrees_drawn = 0.0;

            let mut r = (10 + random_below((self.width + self.height) / 2)) as f32;
            if random_below(100) < 50 {
                r *= -1.0;
            }

            // Arc length = r * theta, so theta = arc length / r.
            let radian_inc = STEP / r;
            cr.t_inc = radian_inc * 360.0 / 2.0 / std::f32::consts::PI;
            cr.ys = r * radian_inc.sin();
            cr.xs = r * (1.0 - radian_inc.cos());
        } else {
            cr.curved = false;
        }

        // Condensed from Crack::startCrack.
        cr.x = px as f32 + 0.61 * (a * std::f32::consts::PI / 180.0).cos();
        cr.y = py as f32 + 0.61 * (a * std::f32::consts::PI / 180.0).sin();
        cr.t = a;
    }

    fn make_crack(&mut self) {
        if self.cracks.len() >= self.max_num {
            return;
        }
        let cr = Crack {
            sandp: 0.0,
            sandg: frand(0.2) as f32 - 0.01,
            sandcolor: RGB_COLORMAP[(random() as usize) % RGB_COLORMAP.len()],
            curved: false,
            degrees_drawn: 0.0,
            // We could use these values in the timeout case of start_crack.
            x: random_below(self.width) as f32,
            y: random_below(self.height) as f32,
            t: random_below(360) as f32,
            ..Crack::default()
        };
        self.cracks.push(cr);
        let i = self.cracks.len() - 1;
        self.start_crack(i);
    }

    /// Alpha blended point drawing, against the shadow copy of the window.
    fn trans_point(&mut self, x1: i32, y1: i32, myc: Pixel, a: f32) -> Pixel {
        if x1 >= 0 && x1 < self.width && y1 >= 0 && y1 < self.height {
            let o = (y1 * self.width + x1) as usize;
            if a >= 1.0 {
                self.off_img[o] = myc;
            } else {
                let (or, og, ob) = unrgb(self.off_img[o]);
                let (r, g, b) = unrgb(myc);
                let mix = |o: u8, n: u8| (o as f32 + (n as f32 - o as f32) * a) as u8;
                let c = rgb(mix(or, r), mix(og, g), mix(ob, b));
                self.off_img[o] = c;
                return c;
            }
        }
        // Both the opaque case and the out of bounds case land here.
        self.bgcolor
    }

    /// Synthesis of `Crack::regionColor()` and `SandPainter::render()`.
    fn region_color(&mut self, d: &mut Dpy, i: usize) {
        let cr = self.cracks[i];
        let mut rx = cr.x;
        let mut ry = cr.y;

        // Move perpendicular to the crack until the open space runs out.
        loop {
            rx += 0.81 * (cr.t * std::f32::consts::PI / 180.0).sin();
            ry -= 0.81 * (cr.t * std::f32::consts::PI / 180.0).cos();

            let mut cx = rx as i32;
            let mut cy = ry as i32;
            if self.seamless {
                cx %= self.width;
                cy %= self.height;
            }

            if cx >= 0 && cx < self.width && cy >= 0 && cy < self.height {
                if self.cgrid_at(cx, cy) <= 10000 {
                    break;
                }
            } else {
                break;
            }
        }

        // Modulate gain.
        let mut sandg = cr.sandg + (frand(0.1) as f32 - 0.050);
        sandg = sandg.clamp(0.0, 1.0);
        self.cracks[i].sandg = sandg;

        let grains = self.grains;
        let w = sandg / (grains - 1) as f32;

        // Lay down grains of sand.
        for g in 0..grains {
            let s = (cr.sandp + (g as f32 * w).sin()).sin();
            let mut drawx = cr.x + (rx - cr.x) * s;
            let mut drawy = cr.y + (ry - cr.y) * s;
            if self.seamless {
                drawx = (drawx + self.width as f32) % self.width as f32;
                drawy = (drawy + self.height as f32) % self.height as f32;
            }

            let alpha = 0.1 - g as f32 / (grains as f32 * 10.0);
            let c = self.trans_point(drawx as i32, drawy as i32, cr.sandcolor, alpha);
            self.gc.set_foreground(c);
            d.win().draw_point(&self.gc, drawx as i32, drawy as i32);
            self.gc.set_foreground(self.fgcolor);
        }
    }

    fn build_substrate(&mut self) {
        self.cycles = 0;
        self.cracks.clear();
        self.cgrid = vec![EMPTY; (self.width * self.height) as usize];
        for _ in 0..self.initial_cracks {
            self.make_crack();
        }
    }

    /// Upstream fills the shadow copy with `memset`, which writes the low byte
    /// of the background colour into every byte. For the white default that
    /// lands on white anyway, which is what it was reaching for.
    fn build_img(&mut self) {
        self.off_img = vec![self.bgcolor; (self.width * self.height) as usize];
    }

    fn restart(&mut self, d: &mut Dpy) {
        self.build_substrate();
        self.build_img();
        let bg = self.bgcolor;
        self.gc.set_foreground(bg);
        let (w, h) = (self.width, self.height);
        d.win().fill_rectangle(&self.gc, 0, 0, w, h);
        let fg = self.fgcolor;
        self.gc.set_foreground(fg);
    }

    /// Basically `Crack::move()`.
    fn movedrawcrack(&mut self, d: &mut Dpy, cracknum: usize) {
        let mut cr = self.cracks[cracknum];

        // Continue cracking.
        let rad = cr.t * std::f32::consts::PI / 180.0;
        if !cr.curved {
            cr.x += STEP * rad.cos();
            cr.y += STEP * rad.sin();
        } else {
            cr.x += cr.ys * rad.cos();
            cr.y += cr.ys * rad.sin();
            cr.x += cr.xs * (rad - std::f32::consts::FRAC_PI_2).cos();
            cr.y += cr.xs * (rad - std::f32::consts::FRAC_PI_2).sin();
            cr.t += cr.t_inc;
            cr.degrees_drawn += cr.t_inc.abs();
        }
        if self.seamless {
            cr.x = (cr.x + self.width as f32) % self.width as f32;
            cr.y = (cr.y + self.height as f32) % self.height as f32;
        }
        self.cracks[cracknum] = cr;

        // Bounds check. Modification of random(-0.33, 0.33).
        let mut cx = (cr.x + (frand(0.66) as f32 - 0.33)) as i32;
        let mut cy = (cr.y + (frand(0.66) as f32 - 0.33)) as i32;
        if self.seamless {
            cx %= self.width;
            cy %= self.height;
        }

        if cx >= 0 && cx < self.width && cy >= 0 && cy < self.height {
            // Draw the sand painter if we are not wireframe.
            if !self.wireframe {
                self.region_color(d, cracknum);
            }

            // Draw the fgcolor crack.
            let o = (cy * self.width + cx) as usize;
            self.off_img[o] = self.fgcolor;
            let fg = self.fgcolor;
            self.gc.set_foreground(fg);
            d.win().draw_point(&self.gc, cx, cy);

            let t = self.cracks[cracknum].t;
            if self.cracks[cracknum].curved && self.cracks[cracknum].degrees_drawn > 360.0 {
                // Completed the circle, stop cracking.
                self.start_crack(cracknum);
                self.make_crack();
            } else if self.cgrid[o] > 10000 || (self.cgrid[o] as f32 - t).abs() < 5.0 {
                // Continue cracking.
                self.cgrid[o] = t as i32;
            } else if (self.cgrid[o] as f32 - t).abs() > 2.0 {
                // Crack encountered (not self), stop cracking.
                self.start_crack(cracknum);
                self.make_crack();
            }
        } else {
            // Out of bounds, stop cracking. These are needed in case of a
            // timeout in start_crack.
            let cr = &mut self.cracks[cracknum];
            cr.x = random_below(self.width) as f32;
            cr.y = random_below(self.height) as f32;
            cr.t = random_below(360) as f32;
            self.start_crack(cracknum);
            self.make_crack();
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.width(), d.height());
    let fgcolor = d.res.pixel("foreground");
    let bgcolor = d.res.pixel("background");

    // Upstream exits with a message on each of these; clamp instead, since a
    // screensaver that refuses to run is worse than one that rounds a knob.
    let mut st = State {
        width,
        height,
        initial_cracks: d.res.int("initialCracks").max(3),
        max_num: d.res.int("maxCracks").max(11) as usize,
        grains: d.res.int("sandGrains").max(2),
        circle_percent: d.res.int("circlePercent").clamp(0, 100),
        cracks: Vec::new(),
        cgrid: Vec::new(),
        off_img: Vec::new(),
        fgcolor,
        bgcolor,
        cycles: 0,
        max_cycles: d.res.int("maxCycles").max(0) as u32,
        wireframe: d.res.bool("wireFrame"),
        seamless: d.res.bool("seamless"),
        growth_delay: d.res.int("growthDelay").max(0) as u32,
        gc: Gc::new(fgcolor, bgcolor),
    };
    st.build_img();
    st.build_substrate();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // Upstream re-reads the window size every ten cycles and starts over
        // when it has changed; here the reshape hook does that instead.

        // `make_crack` can lengthen the list while this runs, and upstream
        // re-reads the count each time round, so a crack born this frame is
        // moved this frame.
        let mut i = 0;
        while i < self.cracks.len() {
            self.movedrawcrack(d, i);
            i += 1;
        }

        self.cycles += 1;
        if self.cycles >= self.max_cycles && self.max_cycles != 0 {
            self.restart(d);
        }

        self.growth_delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        self.restart(d);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.cycles = self.max_cycles;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: white",
    ".foreground: black",
    "*fpsSolid: true",
    "*wireFrame: false",
    "*seamless: false",
    "*maxCycles: 10000",
    "*growthDelay: 18000",
    "*initialCracks: 3",
    "*maxCracks: 100",
    "*sandGrains: 64",
    "*circlePercent: 33",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "growthDelay",
        "Frame rate",
        0.0,
        100_000.0,
        1000.0,
        0,
        "18000",
    )
    .inverted(),
    Opt::slider("maxCycles", "Duration", 2000.0, 25000.0, 500.0, 0, "10000"),
    Opt::slider("sandGrains", "Sand grains", 16.0, 128.0, 1.0, 0, "64"),
    Opt::slider(
        "circlePercent",
        "Circle percentage",
        0.0,
        100.0,
        1.0,
        0,
        "33",
    ),
    Opt::spin("initialCracks", "Initial cracks", 3.0, 15.0, "3"),
    Opt::boolean("wireFrame", "Wireframe only", "false"),
    Opt::boolean("seamless", "Seamless mode", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "substrate",
    label: "Substrate",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "J. Tarbell and Mike Kershaw",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=dCCVgBOVD0E"),
        blurb: "Crystalline lines grow on a computational substrate.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
