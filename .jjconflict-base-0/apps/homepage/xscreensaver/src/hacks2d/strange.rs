//! Port of `hacks/strange.c`.
//!
//! ```text
//! strange --- strange attractors
//!
//! Copyright (c) 1997 by Massimino Pascal <Pascal.Massimon@ens.fr>
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! Revision History:
//! 10-Apr-2017: dmo2118@gmail.com: Enhancements for accumulator mode:
//!              Performance tuning, varying colors, fixed wobbliness.
//!              New options: point size, zoom, brightness, motion blur.
//! 22-Dec-2004: TDA: Replace Gauss_Rand with a real Gaussian.
//! 30-Jul-1998: sineswiper@resonatorsoft.com: added curve factor (discovered
//!              while experimenting with the Gauss_Rand function).
//! 10-May-1997: jwz AT jwz.org: turned into a standalone program.
//!              Made it render into an offscreen bitmap and then copy
//!              that onto the screen, to reduce flicker.
//!
//! strange attractors are not so hard to find...
//! ```
//!
//! A point is fed through a fixed pair of polynomials over and over, and the
//! set of places it visits is drawn. Nothing about the shape is chosen: fifteen
//! coefficients are rolled at random, and whatever the resulting map does with
//! a point is the picture. Most such maps fly off to infinity or collapse to
//! nothing; this one cannot, because every coordinate goes through a sine
//! before it comes back, which folds the plane into itself and keeps the orbit
//! in a bounded region no matter how large the intermediate arithmetic gets.
//!
//! All of it runs in fixed point, twelve bits of fraction, and the sine is a
//! sixteen-thousand-entry lookup table indexed by the low bits of whatever
//! integer arrives. That is the fold. Two maps are on offer, one quadratic and
//! one that also divides by a third polynomial, which is the one that makes the
//! swirling sheets rather than the closed loops.
//!
//! The coefficients drift. Each frame interpolates between the current set and
//! the next, and when it arrives the next becomes the current and a fresh set
//! is rolled, so the attractor is always melting into a different one. If the
//! points cluster into something small and dull, the drift speeds up until it
//! is not dull any more, which is a rule upstream calls varying speed to avoid
//! boredom.
//!
//! There are two renderers and the point count picks between them. Up to six
//! thousand points a frame, the orbit is drawn as bare pixels into a one-bit
//! mask, and the mask is copied onto the window in a colour that changes every
//! frame, so the attractor is a single flat colour and the whole window is
//! repainted each time.
//!
//! Past that it switches to an accumulator, which is the mode the modern
//! options are for. Rather than plotting points, it counts how many times each
//! pixel is hit, box-blurs those counts by the point size, mixes in a fading
//! share of the previous frame for motion blur, and maps the result through a
//! logarithmic ramp of one hue. Dense regions of the attractor come out bright
//! and the thin outer filaments stay visible, which they do not when every
//! point is drawn at the same brightness.
//!
//! Three notes on the port. Upstream splits the accumulator across a thread
//! per few rows and gives each thread its own orbit; this runs one orbit on one
//! thread, which is the same picture drawn from one starting point instead of
//! several, and upstream's own picture already varies with the machine's core
//! count. The shared-memory image it rasterises into is written straight to the
//! window here. And the second point buffer is gone: it is only read on a
//! display where the one-bit mask could not be allocated, which cannot happen
//! here.
//!
//! The fixed-point arithmetic overflows on purpose, or at least upstream leaves
//! it to, so every multiply that can is written as a wrapping one.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, XColor};
use crate::runtime::xlockmore::{ColorScheme, MAXRAND, ModeInfo, lrand, nrand};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XPoint, XRectangle,
    random,
};

const MAX_PRM: usize = 15;
const UNIT_BITS: u32 = 12;
const UNIT: i32 = 1 << UNIT_BITS;
const UNIT2: i32 = 1 << 14;
const COLOR_BITS: u32 = 16;
const SKIP_FIRST: i32 = 100;
/// Above this many points a frame the accumulator renderer takes over.
const ACC_THRESHOLD: usize = 6000;
const ACC_GAMMA: f32 = 10.0;
const DEF_NUM_COLS: usize = 150;

fn dbl_to_prm(x: f32) -> i32 {
    (UNIT as f32 * x) as i32
}

/// The spread and the centre of each of the fifteen coefficients.
const AMP_PRM: [f32; MAX_PRM] = [
    1.0, 3.5, 3.5, 2.5, 4.7, //
    1.0, 3.5, 3.6, 2.5, 4.7, //
    1.0, 1.5, 2.2, 2.1, 3.5,
];
const MID_PRM: [f32; MAX_PRM] = [
    0.0, 1.5, 0.0, 0.5, 1.5, //
    0.0, 1.5, 0.0, 0.5, 1.5, //
    0.0, 1.5, -1.0, -0.5, 2.5,
];

/// The generator upstream uses inside the orbit, where the global one would
/// not be thread-safe.
fn goodrnd(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
    *seed
}

/// Extremely cheap entropy: a single multiply.
fn cheaprnd(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(5);
    *seed
}

struct Strange {
    mi: ModeInfo,
    prm1: [f32; MAX_PRM],
    prm2: [f32; MAX_PRM],
    prm: [i32; MAX_PRM],
    /// The sine table that folds the plane back onto itself.
    fold: Vec<i32>,
    /// Whether to use the map that divides by a third polynomial.
    iterate_x3: bool,
    points: Vec<XPoint>,
    max_pt: usize,
    col: u32,
    count: i32,
    speed: i32,
    width: i32,
    height: i32,
    /// The one-bit mask the simple renderer draws into.
    dbuf: Pixmap,
    dbuf_gc: Gc,
    gc: Gc,

    curve: i32,
    point_size: i32,
    zoom: f32,
    brightness: f32,
    motion_blur: f32,

    acc: Option<Acc>,
}

/// The accumulator renderer's buffers.
struct Acc {
    /// One row's worth of padding past the window, for the horizontal blur.
    aligned_width: usize,
    /// How much of the previous frame survives, as a sixteen-bit fraction.
    blur_fac: u16,
    color_fac: f64,
    num_cols: usize,
    cols: Vec<Pixel>,
    palette: Vec<XColor>,
    /// Hits per pixel this frame.
    acc_map: Vec<u16>,
    /// The box blur's row ring, then its running vertical sum.
    bloom_rows: Vec<u16>,
    motion_blur: Vec<u16>,
    rnd: u64,
}

impl Strange {
    fn new(d: &mut Dpy) -> Self {
        let mi = ModeInfo::new(d, ColorScheme::Smooth);
        let (width, height) = (mi.width, mi.height);

        let mut point_size = d.res.int("pointSize").max(1);
        if width > 2560 || height > 2560 {
            // Retina displays.
            point_size *= 3;
        }
        let mut curve = d.res.int("curve");
        if curve <= 0 {
            curve = 10;
        }

        // The fold: a sine table, indexed by the low bits of whatever the
        // arithmetic produced.
        let mut fold = vec![0; UNIT2 as usize + 1];
        for (i, f) in fold.iter_mut().enumerate() {
            *f = dbl_to_prm(((i as f32) / UNIT as f32).sin());
        }

        let max_pt = d.res.int("points").max(1) as usize;
        let white = mi.white;
        let black = mi.black;
        let mut st = Self {
            mi,
            prm1: [0.0; MAX_PRM],
            prm2: [0.0; MAX_PRM],
            prm: [0; MAX_PRM],
            fold,
            iterate_x3: false,
            points: Vec::with_capacity(max_pt),
            max_pt,
            col: 0,
            count: 0,
            speed: 4,
            width,
            height,
            dbuf: Pixmap::new_bitmap(width, height),
            dbuf_gc: Gc::new(1, 0),
            gc: Gc::new(white, black),
            curve,
            point_size,
            zoom: d.res.float("zoom") as f32,
            brightness: d.res.float("brightness") as f32,
            motion_blur: d.res.float("motionBlur") as f32,
            acc: None,
        };
        st.restart(d);
        st
    }

    /// `init_strange`, minus what only has to happen once.
    fn restart(&mut self, d: &mut Dpy) {
        self.width = self.mi.width;
        self.height = self.mi.height;
        self.count = 0;
        self.col = nrand(self.mi.npixels().max(1)) as u32;
        self.speed = 4;

        self.iterate_x3 = nrand(2) != 0;
        if self.curve < 10 {
            // Avoid the boring quadratic map.
            self.iterate_x3 = true;
        }

        self.random_prm_1();
        self.random_prm_2();

        if self.max_pt > ACC_THRESHOLD {
            self.acc = Some(self.new_acc());
        } else {
            self.acc = None;
            self.dbuf = Pixmap::new_bitmap(self.width, self.height);
        }
        self.mi.clear_window(d);
    }

    fn new_acc(&self) -> Acc {
        let npixels = self.mi.npixels().max(1);
        let num_cols = if npixels <= 2 { 2 } else { DEF_NUM_COLS };
        let palette: Vec<XColor> = (0..npixels)
            .map(|i| XColor::from_pixel(self.mi.pixel(i as usize)))
            .collect();
        // Add slack for the horizontal blur.
        let aligned_width = (self.width + self.point_size) as usize;
        let h = self.height.max(1) as usize;
        Acc {
            aligned_width,
            blur_fac: (65536.0 * (self.motion_blur - 1.0) / (self.motion_blur + 1.0)) as u16,
            color_fac: 2.0 / (self.motion_blur as f64 + 1.0),
            num_cols,
            cols: vec![0; num_cols],
            palette,
            acc_map: vec![0; aligned_width * h],
            bloom_rows: vec![0; aligned_width * (self.point_size as usize + 2)],
            motion_blur: vec![0; aligned_width * h],
            rnd: random() as u64,
        }
    }

    /// `Old_Gauss_Rand`: not actually a Gaussian, and the curve knob is the
    /// shape of it.
    fn old_gauss_rand(&self, c: f32, a: f32, s: f32) -> f32 {
        let y = lrand() as f32 / MAXRAND as f32;
        // Upstream divides two integers here, so the curve knob only takes
        // effect in steps of ten, and below ten it flattens the shape to zero.
        let z = (self.curve / 10) as f32;
        let y = a * (z - (-y * y * s).exp()) / (z - (-s).exp());
        if nrand(2) != 0 { c + y } else { c - y }
    }

    fn random_prm_1(&mut self) {
        for i in 0..MAX_PRM {
            self.prm1[i] = self.old_gauss_rand(MID_PRM[i], AMP_PRM[i], 4.0);
        }
    }

    fn random_prm_2(&mut self) {
        for i in 0..MAX_PRM {
            self.prm2[i] = self.old_gauss_rand(MID_PRM[i], AMP_PRM[i], 4.0);
        }
    }

    /// `DO_FOLD`: the sine table, which is what keeps the orbit bounded.
    fn do_fold(&self, a: i32) -> i32 {
        if a < 0 {
            -self.fold[(a.wrapping_neg() & (UNIT2 - 1)) as usize]
        } else {
            self.fold[(a & (UNIT2 - 1)) as usize]
        }
    }

    /// One step of the map. Every product here can overflow, and upstream
    /// lets it, so they all wrap.
    fn iterate(&self, x: i32, y: i32) -> (i32, i32) {
        let p = &self.prm;
        let xx = x.wrapping_mul(x) >> UNIT_BITS;
        let x2y = xx.wrapping_mul(y) >> UNIT_BITS;
        let yy = y.wrapping_mul(y) >> UNIT_BITS;
        let y2x = yy.wrapping_mul(x) >> UNIT_BITS;
        let xy = x.wrapping_mul(y) >> UNIT_BITS;

        let mut tmp_x = p[1]
            .wrapping_mul(xx)
            .wrapping_add(p[2].wrapping_mul(xy))
            .wrapping_add(p[3].wrapping_mul(yy))
            .wrapping_add(p[4].wrapping_mul(x2y));
        tmp_x = p[0].wrapping_sub(y).wrapping_add(tmp_x >> UNIT_BITS);
        tmp_x = self.do_fold(tmp_x);

        let mut tmp_y = p[6]
            .wrapping_mul(xx)
            .wrapping_add(p[7].wrapping_mul(xy))
            .wrapping_add(p[8].wrapping_mul(yy))
            .wrapping_add(p[9].wrapping_mul(y2x));
        tmp_y = p[5].wrapping_add(x).wrapping_add(tmp_y >> UNIT_BITS);
        tmp_y = self.do_fold(tmp_y);

        if !self.iterate_x3 {
            return (tmp_x, tmp_y);
        }

        let mut tmp_z = p[11]
            .wrapping_mul(xx)
            .wrapping_add(p[12].wrapping_mul(xy))
            .wrapping_add(p[13].wrapping_mul(yy))
            .wrapping_add(p[14].wrapping_mul(y2x));
        tmp_z = p[10].wrapping_add(x).wrapping_add(tmp_z >> UNIT_BITS);
        let mut tmp_z0 = UNIT.wrapping_add(tmp_z.wrapping_mul(tmp_z) >> UNIT_BITS);
        // Can happen with a curve of nine.
        if tmp_z0 == 0 {
            tmp_z0 = 1;
        }
        let tmp_z1 = ((1i64 << 30) / tmp_z0 as i64) as u64;
        let xo = ((tmp_x as i64 as u64).wrapping_mul(tmp_z1) >> (30 - UNIT_BITS)) as u32 as i32;
        let yo = ((tmp_y as i64 as u64).wrapping_mul(tmp_z1) >> (30 - UNIT_BITS)) as u32 as i32;
        (xo, yo)
    }

    /// Run the orbit forward a little before drawing any of it, so what is
    /// drawn is on the attractor rather than on the way to it.
    fn init_draw(&self, rnd: &mut u64) -> (i32, i32) {
        let (mut x, mut y) = (0, 0);
        for _ in 0..SKIP_FIRST {
            let (xo, yo) = self.iterate(x, y);
            x = xo + (goodrnd(rnd) >> (31 - 3)) as i32 - 4;
            y = yo + (goodrnd(rnd) >> (31 - 3)) as i32 - 4;
        }
        (x, y)
    }

    /// The mapping from the unit square the attractor lives in to the window.
    fn recalc_scale(&self) -> (f32, f32, i32, i32) {
        let (xmin, ymin, xmax, ymax) = (-UNIT, -UNIT, UNIT, UNIT);
        let lx = self.zoom * self.width as f32 / (xmax - xmin) as f32;
        let ly = -self.zoom * self.height as f32 / (ymax - ymin) as f32;
        let mx = self.width / 2 - ((xmax + xmin) as f32 * lx / 2.0) as i32;
        let my = self.height / 2 - ((ymax + ymin) as f32 * ly / 2.0) as i32;
        (lx, ly, mx, my)
    }

    /// `ramp_color`: one step of the logarithmic ramp from black through the
    /// chosen hue to white.
    fn ramp_color(c: &XColor, i: usize, n: usize) -> XColor {
        const MINBLUE: f32 = 1.0;
        const FULLBLUE: f32 = 128.0;
        let li = MINBLUE
            + (255.0 - MINBLUE) * (1.0 + ACC_GAMMA * i as f32 / n as f32).ln()
                / (1.0 + ACC_GAMMA).ln();
        let low = |c: u16| ((c as f32 * li / FULLBLUE) as i32).clamp(0, 65535) as u16;
        let high = |c: u16| {
            (((65535.0 - c as f32) * (li - FULLBLUE) / (256.0 - FULLBLUE) + c as f32) as i32)
                .clamp(0, 65535) as u16
        };
        if li < FULLBLUE {
            XColor::from_rgb16(low(c.red), low(c.green), low(c.blue))
        } else {
            XColor::from_rgb16(high(c.red), high(c.green), high(c.blue))
        }
    }

    /// The accumulator renderer: count the hits, blur, blend, and map the
    /// counts through the ramp.
    fn draw_accumulator(&mut self, d: &mut Dpy) -> (i32, i32, i32, i32, u32) {
        let (lx, ly, cx, cy) = self.recalc_scale();
        let (width, height) = (self.width, self.height);
        let point_size = self.point_size as usize;
        let max_pt = self.max_pt;

        // Restricts the viewable area to eight thousand pixels square.
        const L_BITS: u32 = 19;
        let ilx = {
            let v = (lx * (1 << L_BITS) as f32) as i32;
            if v == 0 { 1 } else { v }
        };
        let ily = {
            let v = (ly * (1 << L_BITS) as f32) as i32;
            if v == 0 { 1 } else { v }
        };

        // Out of `self` for the duration, so the orbit can be iterated while
        // its buffers are being written.
        let mut acc = match self.acc.take() {
            Some(a) => a,
            None => return (0, 0, 0, 0, 0),
        };
        let mut rnd = acc.rnd;
        let (mut x, mut y) = self.init_draw(&mut rnd);
        let mut cheap = goodrnd(&mut rnd) as u32;
        acc.rnd = rnd;
        acc.acc_map.fill(0);

        let aw = acc.aligned_width;
        for _ in 0..max_pt {
            let (xo, yo) = self.iterate(x, y);
            // Unsigned, so one comparison covers both ends.
            let mx = (((ilx as i64 * x as i64) >> L_BITS) as i32).wrapping_add(cx) as u32;
            let my = (((ily as i64 * y as i64) >> L_BITS) as i32).wrapping_add(cy) as u32;
            if mx < width as u32 && my < height as u32 {
                let i = my as usize * aw + mx as usize;
                acc.acc_map[i] = acc.acc_map[i].wrapping_add(1);
            }
            // Skimp on the randomness.
            x = xo + (cheaprnd(&mut cheap) >> (32 - 3)) as i32 - 4;
            y = yo + (cheaprnd(&mut cheap) >> (32 - 3)) as i32 - 4;
        }

        // Rebuild the ramp from this frame's hue.
        let npixels = self.mi.npixels().max(1) as usize;
        let col = self.col as usize % npixels;
        let src = acc.palette[col.min(acc.palette.len() - 1)];
        for i in 0..acc.num_cols {
            acc.cols[i] = Self::ramp_color(&src, i, acc.num_cols).pixel;
        }
        let mut color_scale = (width as f64
            * height as f64
            * (1 << COLOR_BITS) as f64
            * self.brightness as f64
            * acc.color_fac
            * (self.zoom as f64 * self.zoom as f64)
            / (0.9 * 0.9)
            / 640.0
            / 480.0
            / (point_size * point_size) as f64
            * 800000.0
            / max_pt as f64
            * acc.num_cols as f64
            / 256.0) as u64;
        if acc.num_cols == 2 {
            // Brighter for monochrome.
            color_scale *= 4;
        }

        let (mut xmax, mut xmin, mut ymax, mut ymin) = (0, width, 0, height);
        let mut pixel_count = 0u32;
        acc.bloom_rows.fill(0);
        let mut color_row = vec![0 as Pixel; aw];

        for j in 0..height as usize {
            let mut has_col = false;
            let mut accum: i32 = 0;
            let bloom_at = aw * (j % point_size);
            let accum_at = aw * point_size;

            // Drop the row that has fallen out of the vertical window, then
            // take in the new one.
            for i in 0..aw {
                let b = acc.bloom_rows[bloom_at + i];
                acc.bloom_rows[accum_at + i] = acc.bloom_rows[accum_at + i].wrapping_sub(b);
                acc.bloom_rows[bloom_at + i] = acc.acc_map[j * aw + i];
            }

            // The horizontal half of the box blur, expanding to the right.
            for i in 0..point_size - 1 {
                accum += acc.bloom_rows[bloom_at + i] as i32;
            }
            for i in 0..aw {
                let old = acc.bloom_rows[bloom_at + i];
                let ahead = bloom_at + i + point_size - 1;
                accum += acc.bloom_rows.get(ahead).copied().unwrap_or(0) as i32;
                acc.bloom_rows[bloom_at + i] = accum as u16;
                acc.bloom_rows[accum_at + i] =
                    acc.bloom_rows[accum_at + i].wrapping_add(accum as u16);
                accum -= old as i32;
            }

            let blur = acc.blur_fac;
            for (i, out) in color_row.iter_mut().enumerate() {
                let a = acc.bloom_rows[accum_at + i];
                let v = if blur != 0 {
                    let m = &mut acc.motion_blur[j * aw + i];
                    *m = (((*m as u32 * blur as u32) >> 16) as u16).wrapping_add(a);
                    *m
                } else {
                    a
                };
                let mut c = ((v as u64 * color_scale) >> COLOR_BITS) as usize;
                if c > acc.num_cols - 1 {
                    c = acc.num_cols - 1;
                }
                if c > 0 {
                    // Maxed out pixels do not count towards being interesting.
                    if c < acc.num_cols - 1 {
                        pixel_count += 1;
                    }
                    xmax = xmax.max(i as i32);
                    xmin = xmin.min(i as i32);
                    has_col = true;
                }
                *out = acc.cols[c];
            }
            if has_col {
                ymax = ymax.max(j as i32);
                ymin = ymin.min(j as i32);
            }

            for (i, p) in color_row.iter().enumerate().take(width as usize) {
                d.win().put_pixel(i as i32, j as i32, *p);
            }
        }
        self.acc = Some(acc);
        (xmin, ymin, xmax, ymax, pixel_count)
    }

    /// The simple renderer: the orbit as bare points in a one-bit mask, copied
    /// onto the window in one colour.
    fn draw_points(&mut self, d: &mut Dpy) -> (i32, i32, i32, i32) {
        let (lx, ly, cx, cy) = self.recalc_scale();
        let mut rnd = random() as u64;
        let (mut x, mut y) = self.init_draw(&mut rnd);

        let (mut xmax, mut xmin, mut ymax, mut ymin) = (0, self.width, 0, self.height);
        self.points.clear();
        for _ in 0..self.max_pt {
            let (xo, yo) = self.iterate(x, y);
            let x1 = (lx * x as f32) as i32 + cx;
            let y1 = (ly * y as f32) as i32 + cy;
            self.points.push(XPoint { x: x1, y: y1 });
            xmax = xmax.max(x1);
            xmin = xmin.min(x1);
            ymax = ymax.max(y1);
            ymin = ymin.min(y1);
            x = xo + nrand(8) - 4;
            y = yo + nrand(8) - 4;
        }

        // Clear the mask, draw this frame's points into it, and paint the
        // whole window through it.
        self.dbuf_gc.set_foreground(0);
        let (w, h) = (self.width, self.height);
        self.dbuf.fill_rectangle(&self.dbuf_gc, 0, 0, w, h);
        self.dbuf_gc.set_foreground(1);
        if self.point_size == 1 {
            self.dbuf.draw_points(&self.dbuf_gc, &self.points);
        } else {
            let rects: Vec<XRectangle> = self
                .points
                .iter()
                .map(|p| XRectangle {
                    // The position matches the bloom in accumulator mode.
                    x: p.x - self.point_size + 1,
                    y: p.y,
                    width: self.point_size,
                    height: self.point_size,
                })
                .collect();
            self.dbuf.fill_rectangles(&self.dbuf_gc, &rects);
        }

        let npixels = self.mi.npixels();
        let fg = if npixels <= 2 {
            self.mi.white
        } else {
            self.mi.pixel(self.col as usize % npixels as usize)
        };
        self.gc.set_foreground(fg);
        self.gc.set_background(self.mi.black);
        d.win().copy_plane(&self.gc, &self.dbuf, 0, 0, w, h, 0, 0);
        (xmin, ymin, xmax, ymax)
    }

    /// Roll the coefficients forward, faster if the picture has gone dull.
    fn advance(&mut self, boring: bool) {
        if boring {
            self.count += 4 * self.speed;
        } else {
            self.count += self.speed;
        }
        if self.count >= 1000 {
            self.prm1 = self.prm2;
            self.random_prm_2();
            self.count = 0;
        }
        self.col += 1;
    }
}

impl Screenhack for Strange {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let u = self.count as f32 / 40000.0;
        for j in (0..MAX_PRM).rev() {
            self.prm[j] = dbl_to_prm((1.0 - u) * self.prm1[j] + u * self.prm2[j]);
        }

        if self.acc.is_some() {
            let (lx, ly, _, _) = self.recalc_scale();
            let (xmin, ymin, xmax, ymax, pixel_count) = self.draw_accumulator(d);
            // Speed up the drift if the attractor has become visually boring.
            let small = ((xmax - xmin) as f32) < lx * dbl_to_prm(0.2) as f32
                && ((ymax - ymin) as f32) < ly * dbl_to_prm(0.2) as f32;
            if small || (pixel_count > 0 && pixel_count < (self.width * self.height / 1000) as u32)
            {
                self.speed = (self.speed as f32 * 1.25) as i32;
            } else {
                self.speed = 4;
            }
            self.speed = self.speed.min(32);
            self.count += self.speed;
            if self.count >= 1000 {
                self.prm1 = self.prm2;
                self.random_prm_2();
                self.count = 0;
            }
            self.col += 1;
        } else {
            let (lx, ly, _, _) = self.recalc_scale();
            let (xmin, ymin, xmax, ymax) = self.draw_points(d);
            let boring = ((xmax - xmin) as f32) < lx * dbl_to_prm(0.2) as f32
                && ((ymax - ymin) as f32) < ly * dbl_to_prm(0.2) as f32;
            self.advance(boring);
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.restart(d);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(Strange::new(d))
}

const DEFAULTS: &[&str] = &[
    "*delay: 10000",
    "*ncolors: 100",
    "*fpsSolid: True",
    "*ignoreRotation: True",
    "*curve: 10",
    "*points: 5500",
    "*pointSize: 1",
    "*zoom: 0.9",
    "*brightness: 1.0",
    "*motionBlur: 3.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("curve", "Curviness", 1.0, 50.0, 1.0, 0, "10"),
    Opt::slider(
        "points",
        "Number of points",
        1000.0,
        500_000.0,
        500.0,
        0,
        "5500",
    ),
    Opt::slider("pointSize", "Point size", 1.0, 8.0, 1.0, 0, "1"),
    Opt::slider("zoom", "Zoom", 0.1, 4.0, 0.1, 2, "0.9"),
    Opt::slider("brightness", "Brightness", 0.1, 4.0, 0.1, 2, "1.0"),
    Opt::slider("motionBlur", "Motion blur", 1.0, 10.0, 0.5, 1, "3.0"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "100"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "strange",
    label: "Strange",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Massimino Pascal",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=F1qna7UAxC0"),
        blurb: "Strange attractors: a swarm of dots swoops and twists around.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
