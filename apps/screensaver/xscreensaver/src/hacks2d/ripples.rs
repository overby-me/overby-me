//! Port of `hacks/ripples.c`.
//!
//! ```text
//! ripples, Copyright (c) 1999 Ian McConnell <ian@emit.demon.co.uk>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! "Water" ripples that can cross and interfere with each other.
//!
//! I can't remember where I got this idea from, but it's been around for a
//! while in various demos. Some inspiration from
//!      water.txt by Tom Hammersley,tomh@globalnet.co.uk
//!
//! Code mainly hacked from xflame and decayscreen.
//! ```
//!
//! A height field on a grid, stepped by the wave equation, used as a lens over a
//! picture. Every cell is set to the average of its four neighbours minus what
//! it was two steps ago, which is the second derivative written out as a
//! difference, and then a fraction of itself is subtracted off so the ripples
//! die away. Upstream's own note explains why that average has no coefficient:
//! the term it belongs to vanishes exactly when the timestep is chosen so that
//! `a dt / h = 1/2`.
//!
//! Two arrays hold the field at successive steps and swap each frame, since the
//! step before last is only ever read to produce the next one.
//!
//! What is drawn is not the height but its gradient: each cell's slope displaces
//! where the picture is sampled from, which is what makes it look like looking
//! through water rather than at a heightmap. The lighting option adds the
//! vertical gradient to the brightness on top, which reads as the sun catching
//! the sides of the waves. The grid is half the resolution of the window and
//! each cell paints a two-by-two block; a dirty count per cell stops flat water
//! from being redrawn.
//!
//! Three knobs here are upstream's rather than the XML's. The water switch turns
//! off the picture and draws the height field in colour instead, which is a
//! whole second rendering path, and without it upstream's own psychedelic-colour
//! option cannot do anything at all, because the palette it builds is only read
//! by the path the switch selects. The box option throws in square splashes and
//! the colour count sizes that palette.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{ALPHA, Pixel, make_smooth_colormap, rgb, unrgb};
use crate::runtime::{
    About, Dpy, Gc, ImageLoad, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, XImage,
    random, screenhack_event_helper,
};

const TABLE: usize = 256;
/// How hard to hit the water.
const SPLASH: i32 = 512;
/// A cell stays redrawn for this many frames after it last moved.
const DIRTY: i8 = 3;

/// Distribution of drops: many little ones and a few big ones.
const DROP_DIST: [f64; 10] = [0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.2, 0.6];

/// The shapes of splash. Upstream has a fourth, a bare two-by-two spike, which
/// sits in the `default:` arm of its switch and which nothing ever asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RippleMode {
    Blob,
    Box,
    Stir,
}

/// C's `random() % n`, which for a negative `n` still yields a non-negative
/// value because the remainder takes the sign of the dividend. Upstream leans
/// on that: the splash height is passed in negative.
fn crandom_mod(n: i32) -> i32 {
    if n == 0 {
        return 0;
    }
    ((random() & 0x7fff_ffff) as i32) % n
}

struct Ripples {
    gc: Gc,
    orig_map: Option<XImage>,
    /// The ripple palette, only read when the picture is switched off.
    ctab: Vec<Pixel>,
    ncolors: usize,
    light: i32,
    light_mode: bool,

    /// The ripple grid, which is half the window's resolution.
    width: i32,
    height: i32,
    bigwidth: i32,
    bigheight: i32,

    transparent: bool,
    grayscale_p: bool,
    buffer_a: Vec<i16>,
    buffer_b: Vec<i16>,
    temp: Vec<i16>,
    dirty_buffer: Vec<i8>,
    cos_tab: [f64; TABLE],

    stir_ang: f64,
    draw_toggle: bool,
    draw_count: i32,

    iterations: i64,
    delay: u32,
    rate: i32,
    boxen: i32,
    stir: bool,
    fluidity: i32,
    duration: f64,
    start_time: f64,

    img_loader: Option<ImageLoad>,
    loading: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let delay = d.res.int("delay").max(0) as u32;
    let duration = (d.res.float("duration")).max(1.0);
    let rate = d.res.int("rate");
    let boxen = d.res.int("box");
    let oily = d.res.bool("oily");
    let stir = d.res.bool("stir");
    let fluidity = d.res.int("fluidity");
    let transparent = d.res.bool("water");
    let grayscale_p = d.res.bool("grayscale");
    let light = d.res.int("light");

    // Sixteen is the width of the short the field is kept in.
    let fluidity = fluidity.clamp(1, 16);
    let light = light.max(0);

    let mut cos_tab = [0.0f64; TABLE];
    for (i, c) in cos_tab.iter_mut().enumerate() {
        *c = (i as f64 * std::f64::consts::FRAC_PI_2 / TABLE as f64).cos();
    }

    let (bigwidth, bigheight) = (d.width(), d.height());
    let (width, height) = (bigwidth / 2, bigheight / 2);

    let mut ncolors = d.res.int("colors").max(2) as usize;
    if ncolors > 256 {
        ncolors = 256;
    }
    // The palette runs from the background colour to the foreground, or right
    // round the wheel if psychedelic colours were asked for. Upstream reads one
    // entry past the end of what it filled; the last entry repeats here.
    let ctab: Vec<Pixel> = if oily {
        let colors = make_smooth_colormap(ncolors);
        (0..=ncolors)
            .map(|i| colors[i.min(ncolors - 1)].pixel)
            .collect()
    } else {
        let (fr, fg, fb) = unrgb(d.res.pixel("foreground"));
        let (br, bg, bb) = unrgb(d.res.pixel("background"));
        let cinterp = |a: f64, bgc: u8, fgc: u8| {
            (((1.0 - a) * bgc as f64 + a * fgc as f64 + 0.5) as i32).clamp(0, 255) as u8
        };
        (0..=ncolors)
            .map(|i| {
                let a = i as f64 / ncolors as f64;
                rgb(cinterp(a, br, fr), cinterp(a, bg, fg), cinterp(a, bb, fb))
            })
            .collect()
    };

    let light_mode = transparent && light > 0;

    let mut st = Ripples {
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        orig_map: None,
        ctab,
        ncolors,
        light,
        light_mode,
        width,
        height,
        bigwidth,
        bigheight,
        transparent,
        grayscale_p,
        buffer_a: Vec::new(),
        buffer_b: Vec::new(),
        temp: Vec::new(),
        dirty_buffer: Vec::new(),
        cos_tab,
        stir_ang: 0.0,
        draw_toggle: false,
        draw_count: 0,
        iterations: 0,
        delay,
        rate,
        boxen,
        stir,
        fluidity,
        duration,
        start_time: 0.0,
        img_loader: None,
        loading: false,
    };

    if transparent {
        st.start_load(d);
    } else {
        st.init_ripples(d);
    }
    Box::new(st)
}

impl Ripples {
    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        self.start_time = d.time;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    fn image_arrived(&mut self, d: &mut Dpy) {
        self.loading = false;
        self.start_time = d.time;
        let (w, h) = (self.bigwidth, self.bigheight);
        self.orig_map = Some(d.win_ref().sub_image(0, 0, w, h));
        self.init_ripples(d);
    }

    /// Turn a wave height into a colour, which is only used when the picture is
    /// switched off.
    fn map_color(&self, grey: i32) -> Pixel {
        let mut g = self.ncolors as i32 * grey.abs() / (SPLASH / 4);
        if g > self.ncolors as i32 {
            g = self.ncolors as i32;
        }
        self.ctab[g as usize]
    }

    fn grayscale(&self, color: Pixel) -> Pixel {
        if !self.grayscale_p || !self.transparent {
            return color;
        }
        let (r, g, b) = unrgb(color);
        let gray = ((r as i32 + g as i32 + b as i32) / 3).clamp(0, 255) as u8;
        rgb(gray, gray, gray)
    }

    /// Add `dx` to every channel, clamped. Upstream builds this from the
    /// visual's masks; the alpha has to be put back, which an X pixel does not
    /// carry.
    fn bright(dx: i32, color: Pixel) -> Pixel {
        let (r, g, b) = unrgb(color);
        let f = |c: u8| (c as i32 + dx).clamp(0, 255) as u8;
        rgb(f(r), f(g), f(b)) | ALPHA
    }

    /// The shape of a drop: a cosine hump.
    fn sinc(&self, x: f64) -> f64 {
        let mut i = (x * TABLE as f64 + 0.5) as i32;
        if i >= TABLE as i32 {
            i = (TABLE as i32 - 1) - (i - (TABLE as i32 - 1));
        }
        if i < 0 {
            return 0.0;
        }
        self.cos_tab[i as usize]
    }

    fn add_circle_drop(&mut self, x: i32, y: i32, radius: i32, dheight: i32) {
        let use_a = random() & 1 != 0;
        let r2 = radius * radius;

        'outer: for cy in -radius..=radius {
            for cx in -radius..=radius {
                let xx = x + cx;
                let yy = y + cy;
                if xx < 0 || yy < 0 || xx >= self.width || yy >= self.height {
                    // Upstream breaks rather than skipping, which clips the
                    // drop against the edge instead of wrapping it.
                    continue 'outer;
                }
                let r = cx * cx + cy * cy;
                if r > r2 {
                    continue 'outer;
                }
                let v = (dheight as f64
                    * self.sinc(if radius > 0 {
                        (r as f64).sqrt() / radius as f64
                    } else {
                        0.0
                    })) as i16;
                let k = (xx + yy * self.width) as usize;
                if use_a {
                    self.buffer_a[k] = v;
                } else {
                    self.buffer_b[k] = v;
                }
            }
        }
    }

    fn add_drop(&mut self, mode: RippleMode, drop: i32) {
        let mut radius = self.width.min(self.height) / 50;
        // Don't put drops too near the edge of the screen or they get stuck.
        let mut border = 8;

        match mode {
            RippleMode::Blob => {
                let power = DROP_DIST[(random() as usize) % DROP_DIST.len()];
                let dheight = (drop as f64 * (power + 0.01)) as i32;
                let tmp_i =
                    (self.width as f64 - 2.0 * border as f64 - 2.0 * radius as f64 * power) as i32;
                let tmp_j =
                    (self.height as f64 - 2.0 * border as f64 - 2.0 * radius as f64 * power) as i32;
                let newx = radius + border + if tmp_i > 0 { crandom_mod(tmp_i) } else { 0 };
                let newy = radius + border + if tmp_j > 0 { crandom_mod(tmp_j) } else { 0 };
                self.add_circle_drop(newx, newy, radius, dheight);
            }
            RippleMode::Box => {
                // Adding too many boxes too quickly doesn't give the waves time
                // to disperse, and they build up and overflow.
                radius = (1 + crandom_mod(5)) * (1 + crandom_mod(5));
                let mut dheight = drop / 128;
                if random() & 1 != 0 {
                    dheight = -dheight;
                }
                let tmp_i = self.width - 2 * border - 2 * radius;
                let tmp_j = self.height - 2 * border - 2 * radius;
                let newx = radius + border + if tmp_i > 0 { crandom_mod(tmp_i) } else { 0 };
                let newy = radius + border + if tmp_j > 0 { crandom_mod(tmp_j) } else { 0 };
                let use_a = random() & 1 != 0;
                for cy in -radius..=radius {
                    for cx in -radius..=radius {
                        let (xx, yy) = (newx + cx, newy + cy);
                        if xx < 0 || yy < 0 || xx >= self.width || yy >= self.height {
                            continue;
                        }
                        let k = (xx + yy * self.width) as usize;
                        if use_a {
                            self.buffer_a[k] = dheight as i16;
                        } else {
                            self.buffer_b[k] = dheight as i16;
                        }
                    }
                }
            }
            RippleMode::Stir => {
                border += radius;
                let newx = border
                    + ((self.width - 2 * border) as f64 * (1.0 + (3.0 * self.stir_ang).cos()) / 2.0)
                        as i32;
                let newy = border
                    + ((self.height - 2 * border) as f64 * (1.0 + (2.0 * self.stir_ang).sin())
                        / 2.0) as i32;
                self.add_circle_drop(newx, newy, radius, drop / 10);
                self.stir_ang += 0.02;
                if self.stir_ang > 12.0 * std::f64::consts::PI {
                    self.stir_ang = 0.0;
                }
            }
        }
    }

    fn init_ripples(&mut self, d: &mut Dpy) {
        let n = (self.width * self.height).max(1) as usize;
        self.buffer_a = vec![0; n];
        self.buffer_b = vec![0; n];
        self.temp = vec![0; n];
        self.dirty_buffer = vec![0; n];

        if self.transparent {
            if let Some(orig) = self.orig_map.take() {
                for down in 0..self.bigheight {
                    for across in 0..self.bigwidth {
                        let p = self.grayscale(orig.get_pixel(across, down));
                        d.win().put_pixel(across, down, p);
                    }
                }
                self.orig_map = Some(orig);
            }
        } else {
            let color = self.map_color(0); // Background colour.
            self.gc.set_foreground(color);
            let (w, h) = (self.bigwidth, self.bigheight);
            d.win().fill_rectangle(&self.gc, 0, 0, w, h);
        }
    }

    /// One step of the wave equation, plus the smoothing pass upstream does on
    /// half the frames.
    fn ripple(&mut self, d: &mut Dpy) {
        let w = self.width as usize;
        let (src, dest) = if !self.draw_toggle {
            self.draw_toggle = true;
            (&mut self.buffer_a, &mut self.buffer_b)
        } else {
            self.draw_toggle = false;
            (&mut self.buffer_b, &mut self.buffer_a)
        };

        match self.draw_count {
            0 | 1 => {
                let mut pixel = w + 1;
                for _ in 1..self.height - 1 {
                    for _ in 1..self.width - 1 {
                        self.temp[pixel] = (((src[pixel - 1] as i32
                            + src[pixel + 1] as i32
                            + src[pixel - w] as i32
                            + src[pixel + w] as i32)
                            / 2)
                            - dest[pixel] as i32) as i16;
                        pixel += 1;
                    }
                    pixel += 2;
                }

                // Smooth the output.
                let mut pixel = w + 1;
                for _ in 1..self.height - 1 {
                    for _ in 1..self.width - 1 {
                        if self.temp[pixel] != 0 {
                            // Close enough for government work.
                            let damp = (self.temp[pixel - 1] as i32
                                + self.temp[pixel + 1] as i32
                                + self.temp[pixel - w] as i32
                                + self.temp[pixel + w] as i32
                                + self.temp[pixel - w - 1] as i32
                                + self.temp[pixel - w + 1] as i32
                                + self.temp[pixel + w - 1] as i32
                                + self.temp[pixel + w + 1] as i32
                                + self.temp[pixel] as i32)
                                / 9;
                            dest[pixel] = (damp - (damp >> self.fluidity)) as i16;
                        } else {
                            dest[pixel] = 0;
                        }
                        pixel += 1;
                    }
                    pixel += 2;
                }
            }
            _ => {
                let mut pixel = w + 1;
                for _ in 1..self.height - 1 {
                    for _ in 1..self.width - 1 {
                        let damp = ((src[pixel - 1] as i32
                            + src[pixel + 1] as i32
                            + src[pixel - w] as i32
                            + src[pixel + w] as i32)
                            / 2)
                            - dest[pixel] as i32;
                        dest[pixel] = (damp - (damp >> self.fluidity)) as i16;
                        pixel += 1;
                    }
                    pixel += 2;
                }
            }
        }
        self.draw_count += 1;
        if self.draw_count > 3 {
            self.draw_count = 0;
        }

        // `dest` is the field just computed; draw from it.
        let from_a = !self.draw_toggle;
        if self.transparent {
            self.draw_transparent(d, from_a);
        } else {
            self.draw_ripple(d, from_a);
        }
    }

    fn field(&self, from_a: bool) -> &[i16] {
        if from_a {
            &self.buffer_a
        } else {
            &self.buffer_b
        }
    }

    /// Colour the height field directly, when there is no picture under it.
    fn draw_ripple(&mut self, d: &mut Dpy, from_a: bool) {
        let w = self.width as usize;
        let mut idx = 0usize;
        for down in 0..self.height - 1 {
            for across in 0..self.width - 1 {
                let src = self.field(from_a);
                let v1 = src[idx] as i32;
                let v2 = src[idx + 1] as i32;
                let v3 = src[idx + w] as i32;
                let v4 = src[idx + w + 1] as i32;
                if v1 == 0 && v2 == 0 && v3 == 0 && v4 == 0 {
                    if self.dirty_buffer[idx] > 0 {
                        self.dirty_buffer[idx] -= 1;
                    }
                } else {
                    self.dirty_buffer[idx] = DIRTY;
                }

                if self.dirty_buffer[idx] > 0 {
                    let dx = if self.light > 0 {
                        ((v3 - v1) + (v4 - v2)) << self.light // Light from top.
                    } else {
                        0
                    };
                    let (x, y) = (across << 1, down << 1);
                    let c = [
                        self.map_color(dx + v1),
                        self.map_color(dx + ((v1 + v2) >> 1)),
                        self.map_color(dx + ((v1 + v3) >> 1)),
                        self.map_color(dx + ((v1 + v4) >> 1)),
                    ];
                    d.win().put_pixel(x, y, c[0]);
                    d.win().put_pixel(x + 1, y, c[1]);
                    d.win().put_pixel(x, y + 1, c[2]);
                    d.win().put_pixel(x + 1, y + 1, c[3]);
                }
                idx += 1;
            }
            idx += 1;
        }
    }

    /// Use the field's gradient to displace where the picture is sampled, which
    /// is what makes it look like water rather than a heightmap.
    fn draw_transparent(&mut self, d: &mut Dpy, from_a: bool) {
        let Some(orig) = self.orig_map.take() else {
            return;
        };
        let w = self.width as usize;
        let mut pixel = 0usize;

        for down in 0..self.height - 2 {
            for across in 0..self.width - 2 {
                let src = self.field(from_a);
                let x0 = src[pixel] as i32;
                let x1 = src[pixel + 1] as i32;
                let x2 = src[pixel + 2] as i32;
                let y1 = src[pixel + w] as i32;
                let y2 = src[pixel + 2 * w] as i32;
                let corner = src[pixel + w + 1] as i32;

                let mut gradx = x1 - x0;
                let mut grady = y1 - x0;
                let mut gradx1 = 1 + (gradx + (x2 - x1)) / 2;
                let mut grady1 = 1 + (grady + (y2 - y1)) / 2;

                if (2 * across + gradx.min(gradx1) < 0)
                    || (2 * across + gradx.max(gradx1) >= self.bigwidth)
                {
                    gradx = 0;
                    gradx1 = 1;
                }
                if (2 * down + grady.min(grady1) < 0)
                    || (2 * down + grady.max(grady1) >= self.bigheight)
                {
                    grady = 0;
                    grady1 = 1;
                }

                if gradx == 0 && gradx1 == 1 && grady == 0 && grady1 == 1 {
                    if self.dirty_buffer[pixel] > 0 {
                        self.dirty_buffer[pixel] -= 1;
                    }
                } else {
                    self.dirty_buffer[pixel] = DIRTY;
                }

                if self.dirty_buffer[pixel] > 0 {
                    let (bx, by) = (across << 1, down << 1);
                    let sample = [
                        (bx + gradx, by + grady),
                        (bx + gradx1, by + grady),
                        (bx + gradx, by + grady1),
                        (bx + gradx1, by + grady1),
                    ];
                    // Light from top.
                    let dx = if self.light_mode {
                        let g = grady + (corner - x1);
                        if 4 - self.light >= 0 {
                            g >> (4 - self.light)
                        } else {
                            g << (self.light - 4)
                        }
                    } else {
                        0
                    };

                    for (k, (sx, sy)) in sample.into_iter().enumerate() {
                        let mut p = self.grayscale(orig.get_pixel(sx, sy));
                        if dx != 0 {
                            p = Self::bright(dx, p);
                        }
                        d.win()
                            .put_pixel(bx + (k as i32 & 1), by + (k as i32 >> 1), p);
                    }
                }
                pixel += 1;
            }
            pixel += 2;
        }

        self.orig_map = Some(orig);
    }
}

impl Screenhack for Ripples {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.loading {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return self.delay;
        }

        if self.transparent && self.start_time + self.duration < d.time {
            self.start_load(d);
            return self.delay;
        }

        if self.rate > 0 && self.iterations % self.rate as i64 == 0 {
            self.add_drop(RippleMode::Blob, -SPLASH);
        }
        if self.stir {
            self.add_drop(RippleMode::Stir, -SPLASH);
        }
        if self.boxen > 0 && crandom_mod(self.boxen) == 0 {
            self.add_drop(RippleMode::Box, -SPLASH);
        }

        self.ripple(d);
        self.iterations += 1;

        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        if width == self.bigwidth && height == self.bigheight {
            return;
        }
        self.bigwidth = width;
        self.bigheight = height;
        self.width = width / 2;
        self.height = height / 2;
        self.orig_map = None;
        if self.transparent {
            self.start_load(d);
        } else {
            self.init_ripples(d);
        }
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.start_time = 0.0;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: #FFAF5F",
    "*colors: 200",
    "*dontClearRoot: True",
    "*delay: 50000",
    "*duration: 120",
    "*rate: 5",
    "*box: 0",
    "*water: True",
    "*oily: False",
    "*stir: False",
    "*fluidity: 6",
    "*light: 4",
    "*grayscale: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "50000").inverted(),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::slider("rate", "Drippiness", 1.0, 100.0, 1.0, 0, "5").inverted(),
    Opt::slider("fluidity", "Fluidity", 0.0, 16.0, 1.0, 0, "6"),
    Opt::boolean("stir", "Moving splashes", "False"),
    Opt::boolean("oily", "Psychedelic colors", "False"),
    Opt::boolean("grayscale", "Grayscale", "False"),
    Opt::spin("light", "Magic lighting effect", 0.0, 8.0, "4"),
    Opt::boolean("water", "Ripple a picture", "True"),
    Opt::slider("box", "Square splashes", 0.0, 100.0, 1.0, 0, "0"),
    Opt::slider("colors", "Number of colors", 2.0, 255.0, 1.0, 0, "200"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "ripples",
    label: "Ripples",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tom Hammersley",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=w8YXAalnRzc"),
        blurb: "Rippling interference patterns reminiscent of splashing water distort a loaded image.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
