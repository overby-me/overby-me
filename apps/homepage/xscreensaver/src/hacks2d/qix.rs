//! Port of `hacks/qix.c`.
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
//! ```
//!
//! The arcade-cabinet one. A polygon's corners bounce around the screen, and
//! the last few hundred positions it held are kept and drawn, so it trails a
//! ribbon behind itself. Each step the oldest is erased and a new one drawn,
//! its colour a few degrees of hue further round than the one before.
//!
//! Several of these run at once, and by default they are translucent: rather
//! than a colour each, they get a plane each, so where two overlap the bits
//! add instead of one painting over the other.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{hsv_to_rgb, rgb, rgb_to_hsv};
use crate::runtime::{
    About, Dpy, GXFunc, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, SelectItem, StartArgs,
    XPoint, frand, random, random_below,
};

const MAXPOLY: usize = 16;
/// Coordinates are kept this many bits below the pixel, so a corner can drift
/// slowly rather than jumping.
const SCALE: i32 = 6;

#[derive(Clone, Copy, Default)]
struct QPoint {
    x: i64,
    y: i64,
    dx: i64,
    dy: i64,
}

#[derive(Clone)]
struct QLine {
    p: Vec<QPoint>,
    /// The colour as hue-saturation-value components, kept alongside the pixel
    /// so the rotation does not drift as it round-trips.
    red: u16,
    green: u16,
    blue: u16,
    pixel: Pixel,
    dead: bool,
}

struct Qix {
    fp: usize,
    npoly: usize,
    lines: Vec<QLine>,
}

struct State {
    draw_gc: Gc,
    erase_gc: Gc,
    /// Per-qix draw and erase contexts, used only in transparent mode.
    plane_gcs: Vec<(Gc, Gc)>,
    default_fg_pixel: Pixel,
    maxx: i64,
    maxy: i64,
    max_spread: i64,
    max_size: i64,
    color_shift: i32,
    random_p: bool,
    solid_p: bool,
    transparent_p: bool,
    gravity_p: bool,
    delay: u32,
    count: usize,
    npoly: usize,
    qixes: Vec<Qix>,
}

impl State {
    fn get_geom(&mut self, w: i32, h: i32) {
        self.maxx = (((w + 1) as i64) << SCALE) - 1;
        self.maxy = (((h + 1) as i64) << SCALE) - 1;
    }

    fn init_one_qix(&mut self, nlines: usize) -> Qix {
        let npoly = self.npoly;
        let mut first = QLine {
            p: vec![QPoint::default(); npoly],
            red: 0,
            green: 0,
            blue: 0,
            pixel: self.default_fg_pixel,
            dead: true,
        };

        if !self.transparent_p {
            let (r, g, b) = hsv_to_rgb(random_below(360), frand(1.0), frand(0.5) + 0.5);
            first.red = r;
            first.green = g;
            first.blue = b;
            first.pixel = rgb((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8);
        }

        if self.max_size == 0 {
            for p in first.p.iter_mut() {
                p.x = rand_below(self.maxx);
                p.y = rand_below(self.maxy);
            }
        } else {
            first.p[0].x = rand_below(self.maxx);
            first.p[0].y = rand_below(self.maxy);
            first.p[1].x = first.p[0].x + rand_below(self.max_size / 2);
            first.p[1].y = first.p[0].y + rand_below(self.max_size / 2);
            first.p[1].x = first.p[1].x.min(self.maxx);
            first.p[1].y = first.p[1].y.min(self.maxy);
        }

        for p in first.p.iter_mut() {
            p.dx = rand_below(self.max_spread + 1) - self.max_spread / 2;
            p.dy = rand_below(self.max_spread + 1) - self.max_spread / 2;
        }

        Qix {
            fp: 0,
            npoly,
            lines: vec![first; nlines],
        }
    }

    fn points_of(line: &QLine, npoly: usize) -> Vec<XPoint> {
        let mut pts: Vec<XPoint> = (0..npoly)
            .map(|i| XPoint {
                x: (line.p[i].x >> SCALE) as i32,
                y: (line.p[i].y >> SCALE) as i32,
            })
            .collect();
        pts.push(pts[0]);
        pts
    }

    fn quad(a: &QLine, b: &QLine) -> [XPoint; 4] {
        let at = |p: &QPoint| XPoint {
            x: (p.x >> SCALE) as i32,
            y: (p.y >> SCALE) as i32,
        };
        [at(&a.p[0]), at(&a.p[1]), at(&b.p[1]), at(&b.p[0])]
    }

    fn free_qline(&mut self, d: &mut Dpy, qid: usize, cur: usize, prev: usize) {
        if self.qixes[qid].lines[cur].dead {
            return;
        }
        let gc = if self.transparent_p {
            self.plane_gcs[qid].1.clone()
        } else {
            self.erase_gc.clone()
        };
        let npoly = self.qixes[qid].npoly;
        if self.solid_p {
            let pts = Self::quad(&self.qixes[qid].lines[cur], &self.qixes[qid].lines[prev]);
            d.win().fill_polygon(&gc, &pts);
        } else {
            let pts = Self::points_of(&self.qixes[qid].lines[cur], npoly);
            d.win().draw_lines(&gc, &pts);
        }
        self.qixes[qid].lines[cur].dead = true;
    }

    /// Nudge one coordinate, bouncing it off the wall it runs into.
    fn wiggle(&self, point: &mut i64, delta: &mut i64, max: i64) {
        if self.random_p {
            *delta += rand_below(1 << (SCALE + 1)) - (1 << SCALE);
        }
        *delta = (*delta).clamp(-self.max_spread, self.max_spread);
        *point += *delta;
        if *point < 0 {
            *point = 0;
            *delta = -*delta;
            *point += *delta << 1;
        } else if *point > max {
            *point = max;
            *delta = -*delta;
            *point += *delta << 1;
        }
    }

    fn add_qline(&mut self, d: &mut Dpy, qid: usize, cur: usize, prev: usize) {
        let npoly = self.qixes[qid].npoly;
        let prev_line = self.qixes[qid].lines[prev].clone();
        self.qixes[qid].lines[cur] = prev_line.clone();

        if self.gravity_p {
            for p in self.qixes[qid].lines[cur].p.iter_mut() {
                p.dy += 3;
            }
        }

        let (maxx, maxy) = (self.maxx, self.maxy);
        for i in 0..npoly {
            let mut p = self.qixes[qid].lines[cur].p[i];
            self.wiggle(&mut p.x, &mut p.dx, maxx);
            self.wiggle(&mut p.y, &mut p.dy, maxy);
            self.qixes[qid].lines[cur].p[i] = p;
        }

        if self.max_size != 0 {
            let jitter = || {
                if self.random_p {
                    rand_below(self.max_spread)
                } else {
                    0
                }
            };
            let line = &mut self.qixes[qid].lines[cur];
            if line.p[0].x - line.p[1].x > self.max_size {
                line.p[0].x = line.p[1].x + self.max_size - jitter();
            } else if line.p[1].x - line.p[0].x > self.max_size {
                line.p[1].x = line.p[0].x + self.max_size - jitter();
            }
            if line.p[0].y - line.p[1].y > self.max_size {
                line.p[0].y = line.p[1].y + self.max_size - jitter();
            } else if line.p[1].y - line.p[0].y > self.max_size {
                line.p[1].y = line.p[0].y + self.max_size - jitter();
            }
        }

        if !self.transparent_p {
            let line = &mut self.qixes[qid].lines[cur];
            let (h, s, v) = rgb_to_hsv(line.red, line.green, line.blue);
            let h = (h + self.color_shift) % 360;
            let (r, g, b) = hsv_to_rgb(h, s, v);
            line.red = r;
            line.green = g;
            line.blue = b;
            line.pixel = rgb((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8);
            let p = line.pixel;
            self.draw_gc.set_foreground(p);
        }

        let gc = if self.transparent_p {
            self.plane_gcs[qid].0.clone()
        } else {
            self.draw_gc.clone()
        };

        if !self.solid_p {
            let pts = Self::points_of(&self.qixes[qid].lines[cur], npoly);
            d.win().draw_lines(&gc, &pts);
        } else if !prev_line.dead {
            let pts = Self::quad(&self.qixes[qid].lines[cur], &prev_line);
            d.win().fill_polygon(&gc, &pts);
        }

        self.qixes[qid].lines[cur].dead = false;
    }

    fn qix1(&mut self, d: &mut Dpy, qid: usize) {
        let nlines = self.qixes[qid].lines.len();
        let fp = self.qixes[qid].fp;
        let ofp = if fp == 0 { nlines - 1 } else { fp - 1 };
        self.free_qline(d, qid, fp, (fp + 1) % nlines);
        self.add_qline(d, qid, fp, ofp);
        self.qixes[qid].fp = (fp + 1) % nlines;
    }
}

/// `random() % n` over the wide fixed-point range the coordinates use.
fn rand_below(n: i64) -> i64 {
    if n <= 1 {
        return 0;
    }
    ((random() as i64) << 15 | random() as i64 & 0x7fff) % n
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let bg = d.res.pixel("background");
    let fg = d.res.pixel("foreground");

    let mut count = d.res.int("count").max(1) as usize;
    let nlines = {
        let n = d.res.int("segments");
        if n <= 0 { 20 } else { n as usize }
    };
    let mut npoly = d.res.int("poly").clamp(2, MAXPOLY as i32) as usize;

    let max_spread = {
        let s = d.res.int("spread");
        (if s <= 0 { 10 } else { s } as i64) << SCALE
    };
    let mut max_size = (d.res.int("size").max(0) as i64) << SCALE;
    let solid_p = d.res.bool("solid");
    let xor_p = d.res.bool("xor");
    let mut transparent_p = d.res.bool("transparent");
    let additive_p = d.res.bool("additive");
    let color_shift = {
        let c = d.res.int("colorShift");
        if !(0..360).contains(&c) { 5 } else { c }
    };

    // Clear up ambiguities regarding npoly.
    if solid_p {
        npoly = 2;
    }
    if npoly > 2 {
        max_size = 0;
    }
    if count == 1 {
        // Transparency between one thing and itself is a no-op.
        transparent_p = false;
    }

    // Upstream asks X for one colour plane per qix. Here the planes are cut
    // out of the framebuffer by hand: qix i takes the same bit from each of
    // red, green and blue, so drawing sets a grey level and two overlapping
    // qixes add rather than overwrite. Eight bits per channel means eight
    // qixes at most.
    if transparent_p {
        count = count.min(8);
    }
    let plane_gcs: Vec<(Gc, Gc)> = (0..count)
        .map(|i| {
            let mask: Pixel = ((0x80u32 >> i) as Pixel) * 0x0001_0101;
            let mut draw = Gc::new(if additive_p { !0 } else { 0 }, bg);
            draw.set_plane_mask(mask);
            let mut erase = Gc::new(if additive_p { 0 } else { !0 }, bg);
            erase.set_plane_mask(mask);
            if xor_p {
                draw.set_function(GXFunc::Xor);
                erase = draw.clone();
            }
            (draw, erase)
        })
        .collect();

    let mut draw_gc = Gc::new(fg, bg);
    let mut erase_gc = Gc::new(bg, bg);
    if xor_p && !transparent_p {
        draw_gc.set_function(GXFunc::Xor);
        erase_gc = draw_gc.clone();
    }

    let mut st = State {
        draw_gc,
        erase_gc,
        plane_gcs,
        default_fg_pixel: fg,
        maxx: 0,
        maxy: 0,
        max_spread,
        max_size,
        color_shift,
        random_p: d.res.bool("random"),
        solid_p,
        transparent_p,
        gravity_p: d.res.bool("gravity"),
        delay: d.res.int("delay").max(0) as u32,
        count,
        npoly,
        qixes: Vec::new(),
    };
    st.get_geom(d.width(), d.height());

    // Subtractive mode starts from white and clears bits as it draws, which is
    // what upstream's colormap does on a screen with writable cells; its
    // TrueColor path leaves the option unimplemented and says so.
    if st.transparent_p && !additive_p {
        let w = rgb(255, 255, 255);
        d.win().clear(w);
    }

    for _ in 0..st.count {
        let q = st.init_one_qix(nlines);
        st.qixes.push(q);
    }
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for i in 0..self.qixes.len() {
            self.qix1(d, i);
        }
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.get_geom(width, height);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*count: 4",
    "*segments: 250",
    "*poly: 2",
    "*spread: 8",
    "*size: 200",
    "*colorShift: 3",
    "*solid: true",
    "*delay: 10000",
    "*random: false",
    "*xor: false",
    "*transparent: true",
    "*gravity: false",
    "*additive: true",
];

const FILL: &[SelectItem] = &[
    SelectItem {
        value: "true",
        label: "Solid objects",
    },
    SelectItem {
        value: "false",
        label: "Line segments",
    },
];

const MOTION: &[SelectItem] = &[
    SelectItem {
        value: "false",
        label: "Linear motion",
    },
    SelectItem {
        value: "true",
        label: "Random motion",
    },
];

const COLOR_MODE: &[SelectItem] = &[
    SelectItem {
        value: "true",
        label: "Additive colors",
    },
    SelectItem {
        value: "false",
        label: "Subtractive colors",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("segments", "Segments", 10.0, 500.0, 10.0, 0, "250"),
    Opt::slider("spread", "Density", 1.0, 50.0, 1.0, 0, "8").inverted(),
    Opt::slider("colorShift", "Color contrast", 0.0, 25.0, 1.0, 0, "3"),
    Opt::select("solid", "Fill", FILL, "true"),
    Opt::select("random", "Motion", MOTION, "false"),
    Opt::select("additive", "Color mode", COLOR_MODE, "true"),
    Opt::spin("count", "Count", 1.0, 20.0, "4"),
    Opt::spin("size", "Max size", 200.0, 1000.0, "200"),
    Opt::spin("poly", "Poly corners", 2.0, 100.0, "2"),
    Opt::boolean("transparent", "Transparent", "true"),
    Opt::boolean("xor", "XOR", "false"),
    Opt::boolean("gravity", "Gravity", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "qix",
    label: "Qix",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=GPqDtJ0vF8U"),
        blurb: "Bounces a series of line segments around the screen with various presentations.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
