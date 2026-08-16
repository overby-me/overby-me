//! Port of `hacks/deluxe.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1999-2018 Jamie Zawinski <jwz@jwz.org>
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
//! Pulsing stars, circles and lines, all centred on the middle of the screen
//! and all breathing in and out at their own rate. A shape that has bounced off
//! its outer limit a few times burns out and is replaced by a fresh one of a
//! random kind.
//!
//! Upstream's transparency is left out: it works by allocating separate colour
//! planes and drawing into them with a plane mask, so that overlapping shapes
//! blend. That is a PseudoColor trick with no counterpart on a TrueColor
//! canvas.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_random_colormap;
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, XPoint, random,
    random_below,
};

/// The ratio of a pentagram's outer radius to its inner one.
const STAR_RATIO: f64 = 2.618_033_988_749_895;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Star,
    Circle,
    HLines,
    VLines,
    Corners,
}

struct Throbber {
    x: i32,
    y: i32,
    size: i32,
    max_size: i32,
    thickness: i32,
    speed: i32,
    /// How many more times it may bounce before it burns out.
    fuse: i32,
    gc: Gc,
    kind: Kind,
}

struct Deluxe {
    count: usize,
    delay: u32,
    colors: Vec<XColor>,
    erase_gc: Gc,
    throbbers: Vec<Throbber>,
    width: i32,
    height: i32,
    thickness: i32,
    speed: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let count = d.res.int("count").max(1) as usize;
    let ncolors = d.res.int("ncolors").max(1) as usize;
    let colors = make_random_colormap(ncolors, true);

    let mut thickness = d.res.int("thickness").max(1);
    if d.width() > 2560 || d.height() > 2560 {
        thickness *= 3; // Retina displays
    }

    let mut st = Deluxe {
        count,
        delay: d.res.int("delay").max(0) as u32,
        colors,
        erase_gc: Gc::new(d.res.pixel("background"), d.res.pixel("background")),
        throbbers: Vec::with_capacity(count),
        width: d.width(),
        height: d.height(),
        thickness,
        speed: d.res.int("speed").max(1),
    };
    for _ in 0..count {
        let t = st.make_throbber();
        st.throbbers.push(t);
    }
    Box::new(st)
}

impl Deluxe {
    fn make_throbber(&self) -> Throbber {
        let (w, h) = (self.width, self.height);
        let mut max_size = w.max(h);
        let thickness = self.thickness;

        // Bounce inwards to start with, at a rate that varies a little.
        let mut speed = self.speed.abs().max(1);
        speed += (random_below(speed) / 2) - (speed / 2);
        if speed > 0 {
            speed = -speed;
        }

        let kind = match random() % 11 {
            0..=3 => Kind::Star,
            4..=7 => Kind::Circle,
            8 => Kind::HLines,
            9 => Kind::VLines,
            _ => Kind::Corners,
        };
        if kind == Kind::Circle {
            max_size = (max_size as f64 * 1.5) as i32;
        }

        let (size, speed) = if !random().is_multiple_of(4) {
            (max_size, speed)
        } else {
            // One in four starts small and grows instead.
            (thickness, -speed)
        };

        let pixel = self.colors[(random() as usize) % self.colors.len()].pixel;
        let mut gc = Gc::new(pixel, pixel);
        gc.set_line_width(thickness);

        Throbber {
            x: w / 2,
            y: h / 2,
            size,
            max_size,
            thickness,
            speed,
            fuse: 1 + random_below(4),
            gc,
            kind,
        }
    }
}

impl Throbber {
    fn draw_star(&self, d: &mut Dpy) {
        let s = self.size as f64 * STAR_RATIO;
        let s2 = self.size as f64;
        let c = std::f64::consts::PI * 2.0;
        let o = -std::f64::consts::PI / 2.0;

        let mut points = [XPoint::default(); 11];
        for (i, p) in points.iter_mut().take(10).enumerate() {
            // Alternate between the outer and inner radius.
            let r = if i % 2 == 0 { s } else { s2 };
            let a = o + (i as f64 / 10.0) * c;
            p.x = self.x + (r * a.cos()) as i32;
            p.y = self.y + (r * a.sin()) as i32;
        }
        points[10] = points[0];
        d.win().draw_lines(&self.gc, &points);
    }

    fn draw(&self, d: &mut Dpy) {
        match self.kind {
            Kind::Star => self.draw_star(d),
            Kind::Circle => {
                d.win().draw_arc(
                    &self.gc,
                    self.x - self.size / 2,
                    self.y - self.size / 2,
                    self.size,
                    self.size,
                    0,
                    FULL_CIRCLE,
                );
            }
            Kind::HLines => {
                let m = self.max_size;
                d.win()
                    .draw_line(&self.gc, 0, self.y - self.size, m, self.y - self.size);
                d.win()
                    .draw_line(&self.gc, 0, self.y + self.size, m, self.y + self.size);
            }
            Kind::VLines => {
                let m = self.max_size;
                d.win()
                    .draw_line(&self.gc, self.x - self.size, 0, self.x - self.size, m);
                d.win()
                    .draw_line(&self.gc, self.x + self.size, 0, self.x + self.size, m);
            }
            Kind::Corners => {
                let s = (self.size + self.thickness) / 2;
                let m = self.max_size;
                if self.y > s {
                    let up = [
                        XPoint {
                            x: 0,
                            y: self.y - s,
                        },
                        XPoint {
                            x: self.x - s,
                            y: self.y - s,
                        },
                        XPoint {
                            x: self.x - s,
                            y: 0,
                        },
                    ];
                    d.win().draw_lines(&self.gc, &up);
                    let up = [
                        XPoint {
                            x: self.x + s,
                            y: 0,
                        },
                        XPoint {
                            x: self.x + s,
                            y: self.y - s,
                        },
                        XPoint {
                            x: m,
                            y: self.y - s,
                        },
                    ];
                    d.win().draw_lines(&self.gc, &up);
                }
                if self.x > s {
                    let down = [
                        XPoint {
                            x: 0,
                            y: self.y + s,
                        },
                        XPoint {
                            x: self.x - s,
                            y: self.y + s,
                        },
                        XPoint {
                            x: self.x - s,
                            y: m,
                        },
                    ];
                    d.win().draw_lines(&self.gc, &down);
                    let down = [
                        XPoint {
                            x: self.x + s,
                            y: m,
                        },
                        XPoint {
                            x: self.x + s,
                            y: self.y + s,
                        },
                        XPoint {
                            x: m,
                            y: self.y + s,
                        },
                    ];
                    d.win().draw_lines(&self.gc, &down);
                }
            }
        }
    }

    /// Step one pulse. Returns false once the shape has burnt out.
    fn throb(&mut self, d: &mut Dpy) -> bool {
        self.size += self.speed;
        if self.size <= self.thickness / 2 {
            self.speed = -self.speed;
            self.size += self.speed * 2;
        } else if self.size > self.max_size {
            self.speed = -self.speed;
            self.size += self.speed * 2;
            self.fuse -= 1;
        }

        if self.fuse <= 0 {
            return false;
        }
        self.draw(d);
        true
    }
}

impl Screenhack for Deluxe {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (w, h) = (self.width, self.height);
        d.win().fill_rectangle(&self.erase_gc, 0, 0, w, h);

        for i in 0..self.count {
            if !self.throbbers[i].throb(d) {
                self.throbbers[i] = self.make_throbber();
            }
        }

        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        d.clear_window();
        for t in self.throbbers.iter_mut() {
            t.fuse = 0;
        }
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 10000",
    "*count: 5",
    "*thickness: 50",
    "*speed: 15",
    "*ncolors: 20",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 50000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("thickness", "Lines", 1.0, 150.0, 1.0, 0, "50"),
    Opt::slider("count", "Shapes", 1.0, 20.0, 1.0, 0, "5"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "20"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "deluxe",
    label: "Deluxe",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=2CsKEVR3ecs"),
        blurb: "Pulsing stars, circles, and lines.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
