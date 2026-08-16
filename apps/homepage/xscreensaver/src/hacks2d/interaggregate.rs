//! Port of `hacks/interaggregate.c`.
//!
//! ```text
//!  InterAggregate (dagraz@gmail.com)
//!  Based on code from complexification.net Intersection Aggregate
//!  http://www.complexification.net/gallery/machines/interAggregate/index.php
//!
//!  Intersection Aggregate code:
//!  j.tarbell   May, 2004
//!  Albuquerque, New Mexico
//!  complexification.net
//!
//!  Also based on substrate, a port of j.tarbell's Substrate Art done
//!  by dragorn@kismetwireless.net
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
//! A hundred circles drift across the screen, and none of them is ever drawn.
//! What is drawn is where two of them cross: every frame, every pair is tested,
//! and the chord between the two points where they meet gets painted. The
//! circles are the machinery, the marks they leave are the picture, and after a
//! few thousand frames the marks have thickened into something that looks like
//! pencil laid on paper.
//!
//! The marks come from the same sand painter as [substrate]: a fan of grains
//! scattered along the chord, dense in the middle and fading out, each laid
//! down at a tenth of an alpha or less. Painting the same chord a thousand
//! times from slightly different positions is what builds the tone.
//!
//! Alpha needs to know what is already on screen, so a shadow copy of the
//! window is kept to read back from, exactly as substrate does.
//!
//! Four of the knobs here are not in the upstream XML, which offers only the
//! frame rate and the number of discs. They are upstream's own command-line
//! options, and without them most of the file is unreachable: at the default of
//! no orbits every circle drifts in a straight line and the whole orbital half
//! of the code never runs. Turn the orbit percentage up and the circles instead
//! wheel around each other in a hierarchy, which is what the author's own notes
//! at the top of the file are about.
//!
//! [substrate]: super::substrate

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, parse_color, rgb, unrgb};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, frand,
    screenhack_event_helper,
};

/// Extracted from `pollockEFF.gif`: white, black, olive, camel, tan.
const RGB_COLORMAP: &[&str] = &[
    "#FFFFFF", "#000000", "#000000", "#4e3e2e", "#694d35", "#b0a085", "#e6d3ae",
];

/// Upstream leaves this as a note to make it an option one day.
const NUM_PAINTERS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PathType {
    Linear,
    Orbit,
}

/// What an orbiting circle goes round: the middle of the screen, or another
/// circle earlier in the list.
#[derive(Clone, Copy)]
enum Center {
    Universe,
    Circle(usize),
}

#[derive(Clone, Copy, Default)]
struct SandPainter {
    color: Pixel,
    gain: f64,
    p: f64,
}

#[derive(Clone, Copy)]
struct Circle {
    radius: f64,
    x: f64,
    y: f64,
    path_type: PathType,
    /// For a linear path.
    dx: f64,
    dy: f64,
    /// For an orbital path.
    theta: f64,
    r: f64,
    dtheta: f64,
    center: Option<Center>,
    painters: [SandPainter; NUM_PAINTERS],
}

struct Interaggregate {
    width: i32,
    height: i32,
    num_circles: usize,
    circles: Vec<Circle>,
    percent_orbits: i32,
    base_orbits: i32,
    base_on_center: bool,
    /// The shadow copy of the window, which alpha blending reads back from.
    off_img: Vec<Pixel>,
    parsedcolors: Vec<Pixel>,
    fgcolor: Pixel,
    bgcolor: Pixel,
    cycles: u32,
    max_cycles: u32,
    max_gain: f64,
    draw_centers: bool,
    growth_delay: u32,
    gc: Gc,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let growth_delay = d.res.int("growthDelay").max(0) as u32;
    let max_cycles = d.res.int("maxCycles").max(0) as u32;
    // Upstream bails out below two circles and outside nought to a hundred
    // percent; the panel cannot reach either, so clamp rather than quit.
    let num_circles = d.res.int("numCircles").max(2) as usize;
    let percent_orbits = d.res.int("percentOrbits").clamp(0, 100);
    let base_orbits = d.res.int("baseOrbits").clamp(0, 100);
    let mut base_on_center = d.res.bool("baseOnCenter");
    if percent_orbits == 100 {
        base_on_center = true;
    }

    let fgcolor = d.res.pixel("foreground");
    let bgcolor = d.res.pixel("background");

    let mut st = Interaggregate {
        width: d.width(),
        height: d.height(),
        num_circles,
        circles: Vec::new(),
        percent_orbits,
        base_orbits,
        base_on_center,
        off_img: Vec::new(),
        parsedcolors: RGB_COLORMAP
            .iter()
            .map(|s| parse_color(s).unwrap_or(fgcolor))
            .collect(),
        fgcolor,
        bgcolor,
        cycles: 0,
        max_cycles,
        max_gain: 0.22,
        draw_centers: d.res.bool("drawCenters"),
        growth_delay,
        gc: Gc::new(fgcolor, bgcolor),
    };
    st.build_field();
    Box::new(st)
}

impl Interaggregate {
    /// Upstream fills the shadow copy with `memset`, which writes the low byte
    /// of the background colour into every byte. For the white default that
    /// lands on white anyway, which is what it was reaching for.
    fn build_img(&mut self) {
        self.off_img = vec![self.bgcolor; (self.width * self.height) as usize];
    }

    fn center_pos(&self, c: Center) -> (f64, f64) {
        match c {
            Center::Universe => (self.width as f64 / 2.0, self.height as f64 / 2.0),
            Center::Circle(i) => (self.circles[i].x, self.circles[i].y),
        }
    }

    /// Lay out a fresh set of circles. The first ones drift in straight lines
    /// and the rest, if any were asked for, orbit: some around the middle of
    /// the screen or around a drifting circle, the later ones around an earlier
    /// orbit, so they build up a hierarchy of wheels on wheels.
    fn build_field(&mut self) {
        self.build_img();

        let n = self.num_circles;
        let num_orbits = (self.percent_orbits * n as i32) / 100;
        let orbit_start = n as i32 - num_orbits;
        let base_orbits = orbit_start + (num_orbits * self.base_orbits) / 100;

        let (w, h) = (self.width as f64, self.height as f64);
        let mut circles: Vec<Circle> = Vec::with_capacity(n);

        for i in 0..n as i32 {
            let path_type = if i >= orbit_start {
                PathType::Orbit
            } else {
                PathType::Linear
            };

            let mut c = Circle {
                radius: 0.0,
                x: 0.0,
                y: 0.0,
                path_type,
                dx: 0.0,
                dy: 0.0,
                theta: 0.0,
                r: 0.0,
                dtheta: 0.0,
                center: None,
                painters: [SandPainter::default(); NUM_PAINTERS],
            };

            if path_type == PathType::Linear {
                c.x = frand(w);
                c.y = frand(h);
                c.dx = frand(0.5) - 0.25;
                c.dy = frand(0.5) - 0.25;
                c.radius = 5.0 + frand(55.0);
                // In case we want orbits based on lines.
                c.r = w.min(h) / 2.0;
            } else {
                let center = if i < base_orbits {
                    let center = if self.base_on_center {
                        Center::Universe
                    } else {
                        Center::Circle(frand(orbit_start as f64 - 0.1) as usize)
                    };
                    c.r = 1.0 + frand(w.min(h) / 2.0);
                    center
                } else {
                    // Give a preference for the earlier circles.
                    let p = frand(0.9);
                    let k = (p * i as f64) as usize;
                    c.r = 1.0 + 0.5 * circles[k].r + 0.5 * frand(circles[k].r);
                    Center::Circle(k)
                };
                c.center = Some(center);

                c.radius = 5.0 + frand(55.0f64.min(c.r));
                c.dtheta = (frand(0.5) - 0.25) / c.r;
                c.theta = frand(std::f64::consts::TAU);

                let (cx, cy) = match center {
                    Center::Universe => (w / 2.0, h / 2.0),
                    Center::Circle(k) => (circles[k].x, circles[k].y),
                };
                c.x = c.r * c.theta.cos() + cx;
                c.y = c.r * c.theta.sin() + cy;
            }

            for painter in &mut c.painters {
                painter.gain = frand(0.09) + 0.01;
                painter.p = frand(1.0);
                let k = (frand(0.999) * self.parsedcolors.len() as f64) as usize;
                painter.color = self.parsedcolors[k];
            }

            circles.push(c);
        }

        self.circles = circles;
    }

    /// Blend `myc` into the shadow copy at `a` alpha and return what came out.
    fn trans_point(&mut self, x: i32, y: i32, myc: Pixel, a: f64) -> Pixel {
        let o = (y * self.width + x) as usize;
        if a >= 1.0 {
            self.off_img[o] = myc;
            return myc;
        }
        let (or, og, ob) = unrgb(self.off_img[o]);
        let (r, g, b) = unrgb(myc);
        let mix = |o: u8, n: u8| (o as f64 + (n as i32 - o as i32) as f64 * a) as u8;
        let c = rgb(mix(or, r), mix(og, g), mix(ob, b));
        self.off_img[o] = c;
        c
    }

    fn draw_point(&mut self, d: &mut Dpy, x: i32, y: i32, color: Pixel, intensity: f64) {
        // The canvas is a torus.
        let mut x = x;
        let mut y = y;
        while x >= self.width {
            x -= self.width;
        }
        while x < 0 {
            x += self.width;
        }
        while y >= self.height {
            y -= self.height;
        }
        while y < 0 {
            y += self.height;
        }

        let c = self.trans_point(x, y, color, intensity);
        self.gc.set_foreground(c);
        d.win().draw_point(&self.gc, x, y);
    }

    /// Scatter one painter's grains along the chord from a to b.
    fn paint(&mut self, d: &mut Dpy, ci: usize, pi: usize, a: (f64, f64), b: (f64, f64)) {
        // Jitter the painter's values.
        let painter = &mut self.circles[ci].painters[pi];
        painter.gain += frand(0.05) - 0.025;
        if painter.gain > self.max_gain {
            painter.gain = -self.max_gain;
        } else if painter.gain < -self.max_gain {
            painter.gain = self.max_gain;
        }

        painter.p += frand(0.1) - 0.05;
        // As upstream has it. The first test clamps every positive value to
        // zero, which leaves the second unreachable and keeps p at or below
        // nought forever.
        if 0.0 < painter.p {
            painter.p = 0.0;
        } else if painter.p > 1.0 {
            painter.p = 1.0;
        }

        // Replace 0.1 with 1 / grains.
        let inc = painter.gain * 0.1;
        let (p, color) = (painter.p, painter.color);
        let mut sandp = 0.0;

        for i in 0..=10 {
            let intensity = 0.1 - 0.009 * i as f64;

            let sp = (p + sandp).sin();
            let (x, y) = (a.0 + (b.0 - a.0) * sp, a.1 + (b.1 - a.1) * sp);
            self.draw_point(d, x as i32, y as i32, color, intensity);

            let sm = (p - sandp).sin();
            let (x, y) = (a.0 + (b.0 - a.0) * sm, a.1 + (b.1 - a.1) * sm);
            self.draw_point(d, x as i32, y as i32, color, intensity);

            sandp += inc;
        }
    }

    fn move_circles(&mut self) {
        let (w, h) = (self.width as f64, self.height as f64);
        for i in 0..self.circles.len() {
            if self.circles[i].path_type == PathType::Linear {
                let c = &mut self.circles[i];
                c.x += c.dx;
                c.y += c.dy;
            } else {
                // A centre is always an earlier circle, so it has already moved
                // this frame and the orbit follows it without a frame of lag.
                let center = self.circles[i].center.expect("an orbit has a centre");
                let (cx, cy) = self.center_pos(center);
                let c = &mut self.circles[i];
                c.theta += c.dtheta;
                if c.theta < 0.0 {
                    c.theta += std::f64::consts::TAU;
                } else if c.theta > std::f64::consts::TAU {
                    c.theta -= std::f64::consts::TAU;
                }
                c.x = c.r * c.theta.cos() + cx;
                c.y = c.r * c.theta.sin() + cy;
            }

            let c = &mut self.circles[i];
            if c.x < 0.0 {
                c.x += w;
            } else if c.x >= w {
                c.x -= w;
            }
            if c.y < 0.0 {
                c.y += h;
            } else if c.y >= h {
                c.y -= h;
            }
        }
    }

    /// Every pair, every frame. Upstream's own note says the intersection test
    /// is dwarfed by the cost of painting the ones that do intersect, so there
    /// is nothing to gain from a spatial index.
    fn draw_intersections(&mut self, d: &mut Dpy) {
        for i in 0..self.circles.len() {
            if self.draw_centers {
                let c1 = self.circles[i];
                d.win().draw_point(&self.gc, c1.x as i32, c1.y as i32);
                continue;
            }

            for j in (i + 1)..self.circles.len() {
                let (c1, c2) = (self.circles[i], self.circles[j]);
                let dx = c2.x - c1.x;
                let dy = c2.y - c1.y;
                let dsqr = dx * dx + dy * dy;
                let dist = dsqr.sqrt();

                // Neither outside nor inside one another: they cross.
                if dx.abs() >= c1.radius + c2.radius
                    || dy.abs() >= c1.radius + c2.radius
                    || dist >= c1.radius + c2.radius
                    || dist <= (c1.radius - c2.radius).abs()
                {
                    continue;
                }

                // Unit vector from c1 towards c2.
                let bx = dx / dist;
                let by = dy / dist;
                let r1sqr = c1.radius * c1.radius;

                // How far along that vector the chord's midpoint sits, and how
                // far from there its two ends are.
                let d1 = 0.5 * (r1sqr - c2.radius * c2.radius + dsqr) / dist;
                let midpx = c1.x + d1 * bx;
                let midpy = c1.y + d1 * by;
                let d2 = (r1sqr - d1 * d1).sqrt();

                let int1 = (midpx + d2 * by, midpy - d2 * bx);
                let int2 = (midpx - d2 * by, midpy + d2 * bx);

                for s in 0..NUM_PAINTERS {
                    self.paint(d, i, s, int1, int2);
                }
            }
        }
    }

    fn clear(&mut self, d: &mut Dpy) {
        let (w, h) = (self.width, self.height);
        self.gc.set_foreground(self.bgcolor);
        d.win().fill_rectangle(&self.gc, 0, 0, w, h);
        self.gc.set_foreground(self.fgcolor);
    }
}

impl Screenhack for Interaggregate {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // Restart if the window size changes. Upstream only looks every tenth
        // frame, and the event handler fakes a size change to force a restart.
        if self.cycles.is_multiple_of(10) && (self.height != d.height() || self.width != d.width())
        {
            self.height = d.height();
            self.width = d.width();
            self.build_field();
            self.clear(d);
        }

        self.move_circles();
        self.draw_intersections(d);

        self.cycles += 1;

        if self.cycles >= self.max_cycles && self.max_cycles != 0 {
            self.build_field();
            self.clear(d);
        }

        self.growth_delay
    }

    fn reshape(&mut self, _d: &mut Dpy, _width: i32, _height: i32) {
        // Upstream's reshape hook is empty: the size check in draw is what
        // notices, on its own schedule.
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.height -= 1; // Act like a resize.
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: white",
    ".foreground: black",
    "*fpsSolid: true",
    "*maxCycles: 100000",
    "*growthDelay: 18000",
    "*numCircles: 100",
    "*percentOrbits: 0",
    "*baseOrbits: 75",
    "*baseOnCenter: False",
    "*drawCenters: False",
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
    Opt::slider("numCircles", "Number of discs", 50.0, 400.0, 1.0, 0, "100"),
    Opt::slider("percentOrbits", "Discs that orbit", 0.0, 100.0, 1.0, 0, "0"),
    Opt::slider(
        "baseOrbits",
        "Orbits about a drifting disc",
        0.0,
        100.0,
        1.0,
        0,
        "75",
    ),
    Opt::boolean("baseOnCenter", "Orbit the middle of the screen", "False"),
    Opt::boolean("drawCenters", "Draw the disc centers", "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "interaggregate",
    label: "Interaggregate",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Casey Reas, William Ngan, Robert Hodgin, and Jamie Zawinski",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=wqPOZiuj4RI"),
        blurb: "Pale pencil-like scribbles slowly fill the screen.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
