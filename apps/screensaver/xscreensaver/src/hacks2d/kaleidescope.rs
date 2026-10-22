//! Port of `hacks/kaleidescope.c`.
//!
//! ```text
//! Copyright (c) 1997 by Ron Tapia <tapia@nmia.com>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! The above, for lack of a better copyright statement in easy reach
//! was just lifted from the xscreensaver source.
//! ```
//!
//! Line segments rotated into k-fold symmetry, each leaving a trail of its own
//! recent positions. From the header: "One of the odd things about this hack is
//! that the radial motion of the segments depends on roundoff error alone."
//! That is not a figure of speech: the endpoints are 16-bit, each symmetry step
//! rotates the previous step's truncated result, and the accumulated error is
//! what makes the figures drift outward. The port keeps the 16-bit arithmetic
//! for exactly that reason.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XColor, XSegment,
    random,
};

/// How the trail is coloured.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorMode {
    /// Random colour per segment, darkening along the trail.
    Greedy,
    /// Random colour per segment, constant along the trail.
    Nice,
    /// Just the foreground colour.
    Plain,
}

/// One position in a segment's trail: the endpoints in the natural coordinate
/// system, its colour, and the rotated copies actually drawn.
#[derive(Clone)]
struct Ksegment {
    color: XColor,
    drawn: bool,
    x1: i16,
    y1: i16,
    x2: i16,
    y2: i16,
    xsegments: Vec<XSegment>,
}

/// One moving line, and the ring of its recent positions.
struct Object {
    time: i32,
    trail: Vec<Ksegment>,
    cur: usize,
}

struct Kaleidescope {
    xoff: i32,
    yoff: i32,
    costheta: f32,
    sintheta: f32,
    symmetry: usize,
    ntrails: usize,
    local_rotation: i32,
    global_rotation: i32,
    draw_gc: Gc,
    erase_gc: Gc,
    default_fg_pixel: crate::runtime::Pixel,
    delay: u32,
    redmin: u16,
    redrange: u16,
    greenmin: u16,
    greenrange: u16,
    bluemin: u16,
    bluerange: u16,
    color_mode: ColorMode,
    objects: Vec<Object>,
    done_once: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let symmetry = d.res.int("symmetry").max(1) as usize;
    let ntrails = d.res.int("ntrails").max(1) as usize;
    let nsegments = d.res.int("nsegments").max(1) as usize;
    let fg = d.res.pixel("foreground");

    let mut draw_gc = Gc::new(fg, d.res.pixel("background"));
    let mut erase_gc = Gc::new(d.res.pixel("background"), fg);
    // Retina displays.
    let lw = if d.width() > 2560 || d.height() > 2560 {
        3
    } else {
        1
    };
    draw_gc.set_line_width(lw);
    erase_gc.set_line_width(lw);

    let mut st = Kaleidescope {
        xoff: d.width() / 2,
        yoff: d.height() / 2,
        costheta: (std::f64::consts::TAU / symmetry as f64).cos() as f32,
        sintheta: (std::f64::consts::TAU / symmetry as f64).sin() as f32,
        symmetry,
        ntrails,
        local_rotation: d.res.int("local_rotation"),
        global_rotation: d.res.int("global_rotation"),
        draw_gc,
        erase_gc,
        default_fg_pixel: fg,
        delay: d.res.int("delay").max(0) as u32,
        redmin: d.res.int("redmin") as u16,
        redrange: d.res.int("redrange").max(1) as u16,
        greenmin: d.res.int("greenmin") as u16,
        greenrange: d.res.int("greenrange").max(1) as u16,
        bluemin: d.res.int("bluemin") as u16,
        bluerange: d.res.int("bluerange").max(1) as u16,
        color_mode: match d.res.string("color_mode") {
            "greedy" => ColorMode::Greedy,
            "nice" => ColorMode::Nice,
            _ => ColorMode::Plain,
        },
        objects: Vec::new(),
        done_once: false,
    };

    st.objects = (0..nsegments).map(|_| st.create_object()).collect();
    for obj in st.objects.iter_mut() {
        // `init_ksegment`: give the current position random endpoints.
        Kaleidescope::randomise(&mut obj.trail[obj.cur], st.xoff, st.yoff);
    }
    Box::new(st)
}

impl Kaleidescope {
    fn random_color(&self) -> XColor {
        match self.color_mode {
            ColorMode::Greedy | ColorMode::Nice => {
                let mut c = XColor::from_rgb16(
                    (random() % self.redrange as u32) as u16 + self.redmin,
                    (random() % self.greenrange as u32) as u16 + self.greenmin,
                    (random() % self.bluerange as u32) as u16 + self.bluemin,
                );
                c.alloc();
                c
            }
            ColorMode::Plain => XColor::from_pixel(self.default_fg_pixel),
        }
    }

    /// `kcycle_color`: only greedy mode fades along the trail.
    fn cycle_color(&self, color: &mut XColor, steps: (u16, u16, u16)) {
        if self.color_mode != ColorMode::Greedy {
            return;
        }
        color.red = color.red.wrapping_sub(steps.0);
        color.green = color.green.wrapping_sub(steps.1);
        color.blue = color.blue.wrapping_sub(steps.2);
        color.alloc();
    }

    fn create_object(&self) -> Object {
        let mut color = self.random_color();
        let steps = (
            color.red / (2 * self.ntrails as u16).max(1),
            color.green / (2 * self.ntrails as u16).max(1),
            color.blue / (2 * self.ntrails as u16).max(1),
        );

        let mut trail = Vec::with_capacity(self.ntrails);
        for i in 0..self.ntrails {
            if i > 0 {
                self.cycle_color(&mut color, steps);
            }
            trail.push(Ksegment {
                color,
                drawn: false,
                x1: 0,
                y1: 0,
                x2: 0,
                y2: 0,
                xsegments: vec![XSegment::default(); self.symmetry],
            });
        }
        Object {
            time: 0,
            trail,
            cur: 0,
        }
    }

    fn randomise(seg: &mut Ksegment, xoff: i32, yoff: i32) {
        let r = |n: i32| {
            if n != 0 {
                (random() % n as u32) as i16
            } else {
                0
            }
        };
        seg.x1 = r(xoff);
        seg.y1 = r(yoff);
        seg.x2 = r(xoff);
        seg.y2 = r(yoff);
    }

    fn draw_object(&mut self, d: &mut Dpy, i: usize) {
        let (xoff, yoff) = (self.xoff, self.yoff);
        let (costheta, sintheta) = (self.costheta, self.sintheta);
        let symmetry = self.symmetry;

        let cur = self.objects[i].cur;
        {
            let seg = &mut self.objects[i].trail[cur];
            let (mut x1, mut y1, mut x2, mut y2) = (seg.x1, seg.y1, seg.x2, seg.y2);

            // Maybe throw the values away and start over.
            let (dx, dy) = ((x2 - x1) as i32, (y2 - y1) as i32);
            if (dx * dx) + (dy * dy) < 100 {
                Self::randomise(seg, xoff, yoff);
                x1 = seg.x1;
                y1 = seg.y1;
                x2 = seg.x2;
                y2 = seg.y2;
            }

            // Each step rotates the previous step's truncated result, which is
            // where the drift comes from.
            for k in 0..symmetry {
                let nx1 = (x1 as f32 * costheta + y1 as f32 * sintheta) as i16;
                let ny1 = (y1 as f32 * costheta - x1 as f32 * sintheta) as i16;
                let nx2 = (x2 as f32 * costheta + y2 as f32 * sintheta) as i16;
                let ny2 = (y2 as f32 * costheta - x2 as f32 * sintheta) as i16;
                x1 = nx1;
                y1 = ny1;
                x2 = nx2;
                y2 = ny2;
                seg.xsegments[k] = XSegment {
                    x1: nx1 as i32 + xoff,
                    y1: ny1 as i32 + yoff,
                    x2: nx2 as i32 + xoff,
                    y2: ny2 as i32 + yoff,
                };
            }
        }

        let (color, segments) = {
            let seg = &self.objects[i].trail[cur];
            (seg.color.pixel, seg.xsegments.clone())
        };
        self.draw_gc.set_foreground(color);
        d.win().draw_segments(&self.draw_gc, &segments);
        self.objects[i].trail[cur].drawn = true;

        // Erase the oldest position in the trail.
        let next = (cur + 1) % self.ntrails;
        if self.objects[i].trail[next].drawn {
            let old = self.objects[i].trail[next].xsegments.clone();
            d.win().draw_segments(&self.erase_gc, &old);
        }
    }

    fn propagate_object(&mut self, i: usize) {
        let two_pi_10k = std::f64::consts::TAU / 10000.0;
        let lsin = (two_pi_10k * self.local_rotation as f64).sin() as f32;
        let lcos = (two_pi_10k * self.local_rotation as f64).cos() as f32;
        let gsin = (two_pi_10k * self.global_rotation as f64).sin() as f32;
        let gcos = (two_pi_10k * self.global_rotation as f64).cos() as f32;

        self.objects[i].time += 1;

        let cur = self.objects[i].cur;
        let (x1, y1, x2, y2) = {
            let s = &self.objects[i].trail[cur];
            (s.x1, s.y1, s.x2, s.y2)
        };

        let midx = (x1 + x2) / 2;
        let midy = (y1 + y2) / 2;

        // The midpoint orbits the centre; the segment spins about its midpoint.
        let nmidx = (midx as f32 * gcos + midy as f32 * gsin) as i16;
        let nmidy = (midy as f32 * gcos - midx as f32 * gsin) as i16;

        let x1 = x1 - midx;
        let x2 = x2 - midx;
        let y1 = y1 - midy;
        let y2 = y2 - midy;

        let next = (cur + 1) % self.ntrails;
        self.objects[i].cur = next;
        let s = &mut self.objects[i].trail[next];
        s.x1 = (x1 as f32 * lcos + y1 as f32 * lsin) as i16 + nmidx;
        s.y1 = (y1 as f32 * lcos - x1 as f32 * lsin) as i16 + nmidy;
        s.x2 = (x2 as f32 * lcos + y2 as f32 * lsin) as i16 + nmidx;
        s.y2 = (y2 as f32 * lcos - x2 as f32 * lsin) as i16 + nmidy;
    }
}

impl Screenhack for Kaleidescope {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.done_once {
            for i in 0..self.objects.len() {
                self.propagate_object(i);
            }
        } else {
            self.done_once = true;
        }
        for i in 0..self.objects.len() {
            self.draw_object(d, i);
        }
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.xoff = width / 2;
        self.yoff = height / 2;
        let lw = if width > 2560 || height > 2560 { 3 } else { 1 };
        self.draw_gc.set_line_width(lw);
        self.erase_gc.set_line_width(lw);
    }
}

const DEFAULTS: &[&str] = &[
    ".background:	     black",
    ".foreground:	     white",
    "*fpsSolid:	     true",
    "*color_mode:      nice",
    "*symmetry:	       11",
    "*ntrails:	      100",
    "*nsegments:          7",
    "*narcs:              0",
    "*local_rotation:   -59",
    "*global_rotation:    1",
    "*spring_constant:    5",
    "*delay:          20000",
    "*redmin:         30000",
    "*redrange:       20000",
    "*greenmin:       30000",
    "*greenrange:     20000",
    "*bluemin:        30000",
    "*bluerange:      20000",
];

const COLOR_MODES: &[SelectItem] = &[
    SelectItem {
        value: "nice",
        label: "One color per trail",
    },
    SelectItem {
        value: "greedy",
        label: "Fading trails",
    },
    SelectItem {
        value: "plain",
        label: "Foreground only",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("nsegments", "Segments", 1.0, 100.0, 1.0, 0, "7"),
    Opt::slider("symmetry", "Symmetry", 3.0, 32.0, 1.0, 0, "11"),
    Opt::slider("ntrails", "Trails", 1.0, 1000.0, 1.0, 0, "100"),
    Opt::select("color_mode", "Colors", COLOR_MODES, "nice"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "kaleidescope",
    label: "Kaleidescope",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ron Tapia",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=mGplFlx1y3M"),
        blurb: "Line segments rotated into k-fold symmetry, leaving trails.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
