//! Port of `hacks/intermomentary.c`.
//!
//! ```text
//!  InterMomentary (dragorn@kismetwireless.net)
//!  Directly ported code from complexification.net InterMomentary art
//!  http://www.complexification.net/gallery/machines/interMomentary/applet_l/interMomentary_l.pde
//!
//! Intersecting Circles, Instantaneous
//! J. Tarbell                              + complexification.net
//! Albuquerque, New Mexico
//! May, 2004
//!
//! a REAS collaboration for the            + groupc.net
//! Whitney Museum of American Art ARTPORT  + artport.whitney.org
//! Robert Hodgin                           + flight404.com
//! William Ngan                            + metaphorical.net
//!
//! 1.0  Oct 10 2004  dragorn  Completed first port
//!
//! Based, of course, on other hacks in:
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
//! Eighty-odd discs drift about, each growing from nothing to its own size.
//! Wherever two of them cross, the two intersection points are stamped into a
//! brightness buffer that is wiped every frame. Points ride round each disc's
//! rim, and when one passes over a stamp it flares into a five-by-five glow.
//! So the circles themselves are never drawn: all you see is where they cross
//! and who happened to be there at that moment.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{make_color_ramp, rgb_to_hsv, unrgb};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, frand, random,
};

/// A point riding round a disc's rim.
#[derive(Clone, Copy, Default)]
struct PxRider {
    t: f32,
    vt: f32,
    mycharge: f32,
}

/// A disc of light.
struct Disc {
    x: f32,
    y: f32,
    /// Current radius, and the one it grows towards.
    r: f32,
    dr: f32,
    vx: f32,
    vy: f32,
    numr: usize,
    px_riders: Vec<PxRider>,
}

struct State {
    height: i32,
    width: i32,
    discs: Vec<Disc>,
    maxrider: usize,
    maxradius: f64,
    /// How many discs to lay out at the start.
    initial: usize,
    fgcolor: Pixel,
    bgcolor: Pixel,
    cycles: u32,
    /// The brightness buffer, wiped every frame. Nothing reads the screen.
    off_alpha: Vec<u8>,

    gc: Gc,
    draw_delay: u32,
    colors: Vec<Pixel>,
    ncolors: usize,
    pscale: i32,
}

impl State {
    #[inline]
    fn ref_pixel(&self, x: i32, y: i32) -> u8 {
        self.off_alpha[(y * self.width + x) as usize]
    }

    /// Alpha blended point drawing. Returns the value left behind, which the
    /// caller turns into a screen colour; upstream returns zero for the
    /// fully-opaque case, and nothing asks for that.
    fn trans_point(&mut self, x1: i32, y1: i32, myc: u8, a: f32) -> u8 {
        if x1 >= 0 && x1 < self.width && y1 >= 0 && y1 < self.height {
            let i = (y1 * self.width + x1) as usize;
            if a >= 1.0 {
                self.off_alpha[i] = myc;
            } else {
                let c = self.off_alpha[i] as f32;
                let c = (c + (myc as f32 - c) * a) as u8;
                self.off_alpha[i] = c;
                return c;
            }
        }
        0
    }

    fn get_pixel(&self, v: u8) -> Pixel {
        self.colors[(v as usize * (self.ncolors - 1)) / 255]
    }

    fn make_disc(&mut self, x: f32, y: f32, vx: f32, vy: f32, r: f32) {
        let numr = ((frand(r as f64) / 2.62) as usize).min(self.maxrider);
        let px_riders = (0..self.maxrider)
            .map(|_| PxRider {
                vt: 0.0,
                t: frand(std::f64::consts::PI * 2.0) as f32,
                mycharge: 0.0,
            })
            .collect();
        self.discs.push(Disc {
            x,
            y,
            vx,
            vy,
            dr: r,
            r: frand(r as f64) as f32 / 3.0,
            numr,
            px_riders,
        });
    }

    fn move_disc(&mut self, dnum: usize) {
        let (w, h) = (self.width as f32, self.height as f32);
        let d = &mut self.discs[dnum];
        d.x += d.vx;
        d.y += d.vy;

        // Bound check: a disc that has left one edge comes back at the other.
        if d.x + d.r < 0.0 {
            d.x += w + d.r + d.r;
        }
        if d.x - d.r > w {
            d.x -= w + d.r + d.r;
        }
        if d.y + d.r < 0.0 {
            d.y += h + d.r + d.r;
        }
        if d.y - d.r > h {
            d.y -= h + d.r + d.r;
        }

        // Increase to destination radius.
        if d.r < d.dr {
            d.r += 0.1;
        }
    }

    fn draw_glowpoint(&mut self, d: &mut Dpy, px: f32, py: f32) {
        for i in -2..3 {
            for j in -2..3 {
                let a = 0.8 - (i * i) as f32 * 0.1 - (j * j) as f32 * 0.1;
                let c = self.trans_point(px as i32 + i, py as i32 + j, 255, a);
                let p = self.get_pixel(c);
                self.gc.set_foreground(p);
                let s = self.pscale;
                d.win()
                    .fill_rectangle(&self.gc, px as i32 + i, py as i32 + j, s, s);
            }
        }
    }

    fn moverender_rider(&mut self, d: &mut Dpy, dnum: usize, m: usize) {
        let (x, y, r) = {
            let di = &self.discs[dnum];
            (di.x, di.y, di.r)
        };
        let (px, py);
        {
            let rid = &mut self.discs[dnum].px_riders[m];
            // Add velocity to theta.
            rid.t = (rid.t + rid.vt + std::f32::consts::PI).rem_euclid(2.0 * std::f32::consts::PI)
                - std::f32::consts::PI;
            rid.vt += frand(0.002) as f32 - 0.001;
            // Apply friction brakes.
            if rid.vt.abs() > 0.02 {
                rid.vt *= 0.9;
            }
            px = x + r * rid.t.cos();
            py = y + r * rid.t.sin();
        }

        if px < 0.0 || px >= self.width as f32 || py < 0.0 || py >= self.height as f32 {
            return;
        }

        // Max brightness seems to be 0.003845. Guestimated: 40 is 18% of 255,
        // so this scales to the same range. In practice any mark at all in the
        // buffer is enough to set a rider glowing.
        let c = self.ref_pixel(px as i32, py as i32);
        let cv = c as f32 / 255.0;

        if cv > 0.0006921 {
            self.draw_glowpoint(d, px, py);
            self.discs[dnum].px_riders[m].mycharge = 0.003845;
        } else {
            let rid = &mut self.discs[dnum].px_riders[m];
            rid.mycharge *= 0.98;
            let c = (255.0 * rid.mycharge) as u8;
            self.trans_point(px as i32, py as i32, c, 0.5);
            let p = self.get_pixel(c);
            self.gc.set_foreground(p);
            let s = self.pscale;
            d.win().fill_rectangle(&self.gc, px as i32, py as i32, s, s);
        }
    }

    fn render_disc(&mut self, d: &mut Dpy, dnum: usize) {
        let (dix, diy, dir) = {
            let di = &self.discs[dnum];
            (di.x, di.y, di.r)
        };

        // Find intersecting points with all ascending discs.
        for n in dnum + 1..self.discs.len() {
            let (nx, ny, nr) = {
                let o = &self.discs[n];
                (o.x, o.y, o.r)
            };
            let dx = nx - dix;
            let dy = ny - diy;
            let dist = (dx * dx + dy * dy).sqrt();

            // Intersection test, then a complete containment test.
            if dist >= nr + dir || dist <= (nr - dir).abs() {
                continue;
            }

            // Find solutions.
            let a = (dir * dir - nr * nr + dist * dist) / (2.0 * dist);
            let p2x = dix + a * (nx - dix) / dist;
            let p2y = diy + a * (ny - diy) / dist;
            let h = (dir * dir - a * a).sqrt();

            let p3ax = p2x + h * (ny - diy) / dist;
            let p3ay = p2y - h * (nx - dix) / dist;
            let p3bx = p2x - h * (ny - diy) / dist;
            let p3by = p2y + h * (nx - dix) / dist;

            let (w, hh) = (self.width as f32, self.height as f32);
            if p3ax < 0.0
                || p3ax >= w
                || p3ay < 0.0
                || p3ay >= hh
                || p3bx < 0.0
                || p3bx >= w
                || p3by < 0.0
                || p3by >= hh
            {
                continue;
            }

            // The two points might be identical; upstream ignores that case.
            for (px, py) in [(p3ax, p3ay), (p3bx, p3by)] {
                let c = self.trans_point(px as i32, py as i32, 255, 0.75);
                let p = self.get_pixel(c);
                self.gc.set_foreground(p);
                let s = self.pscale;
                d.win().fill_rectangle(&self.gc, px as i32, py as i32, s, s);
            }
        }

        // Render all the pixel riders.
        for m in 0..self.discs[dnum].numr {
            self.moverender_rider(d, dnum, m);
        }
    }

    fn build_img(&mut self) {
        self.off_alpha = vec![0; (self.width * self.height).max(1) as usize];
    }

    fn blank_img(&mut self, d: &mut Dpy) {
        self.off_alpha.fill(0);
        self.gc.set_foreground(self.bgcolor);
        let (w, h) = (self.width, self.height);
        d.win().fill_rectangle(&self.gc, 0, 0, w, h);
        self.gc.set_foreground(self.fgcolor);
    }

    fn seed_discs(&mut self) {
        self.discs.clear();
        let n = self.initial;
        for tempx in 0..n {
            // Arrange in an anti-collapsing circle.
            let t = (2.0 * std::f64::consts::PI) * tempx as f64 / n as f64;
            let fx = (0.4 * self.width as f64 * t.cos()) as f32;
            let fy = (0.4 * self.height as f64 * t.sin()) as f32;
            let x = frand(self.width as f64 / 2.0) as f32 + fx;
            let y = frand(self.height as f64 / 2.0) as f32 + fy;
            let r = 5.0 + frand(self.maxradius) as f32;
            let bt = if random() % 100 < 50 { -1.0 } else { 1.0 };
            self.make_disc(x, y, bt * fx / 1000.0, bt * fy / 1000.0, r);
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fgcolor = d.res.pixel("foreground");
    let bgcolor = d.res.pixel("background");

    let mut ncolors = d.res.int("colors").clamp(2, 4096) as usize;
    ncolors += 1;

    let (fr, fg, fb) = unrgb(fgcolor);
    let (br, bg, bb) = unrgb(bgcolor);
    let to16 = |v: u8| ((v as u16) << 8) | v as u16;
    let (fgh, fgs, fgv) = rgb_to_hsv(to16(fr), to16(fg), to16(fb));
    let (bgh, bgs, bgv) = rgb_to_hsv(to16(br), to16(bg), to16(bb));
    let colors: Vec<Pixel> = make_color_ramp(bgh, bgs, bgv, fgh, fgs, fgv, ncolors, false)
        .iter()
        .map(|c| c.pixel)
        .collect();
    let ncolors = colors.len();

    let mut pscale = 1;
    if d.width() > 2560 || d.height() > 2560 {
        pscale *= 3; // Retina displays.
    }

    let mut st = State {
        height: d.height(),
        width: d.width(),
        discs: Vec::new(),
        // Upstream refuses to start below these, printing an error and exiting.
        maxrider: d.res.int("maxRiders").max(11) as usize,
        maxradius: d.res.int("maxRadius").max(31) as f64,
        initial: d.res.int("numDiscs").max(11) as usize,
        fgcolor,
        bgcolor,
        cycles: 0,
        off_alpha: Vec::new(),
        gc: Gc::new(fgcolor, bgcolor),
        draw_delay: d.res.int("drawDelay").max(0) as u32,
        colors,
        ncolors,
        pscale,
    };
    st.build_img();
    st.seed_discs();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.cycles.is_multiple_of(10) && (self.height != d.height() || self.width != d.width())
        {
            // Restart if the window size changes.
            self.height = d.height();
            self.width = d.width();
            self.build_img();
        }

        self.blank_img(d);
        for i in 0..self.discs.len() {
            self.move_disc(i);
            self.render_disc(d, i);
        }

        self.cycles += 1;
        self.draw_delay
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: yellow",
    "*drawDelay: 30000",
    "*numDiscs: 85",
    "*maxRiders: 40",
    "*maxRadius: 100",
    "*colors: 256",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "drawDelay",
        "Frame rate",
        0.0,
        100_000.0,
        1000.0,
        0,
        "30000",
    )
    .inverted(),
    Opt::slider("numDiscs", "Number of discs", 50.0, 400.0, 1.0, 0, "85"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "intermomentary",
    label: "Intermomentary",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Casey Reas, William Ngan, Robert Hodgin, and Jamie Zawinski",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=pH-ykepPopw"),
        blurb: "Blinking dots interact with each other circularly.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
