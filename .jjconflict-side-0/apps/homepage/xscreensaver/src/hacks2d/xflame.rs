//! Port of `hacks/xflame.c`.
//!
//! ```text
//! xflame, Copyright (c) 1996-2018 Carsten Haitzler <raster@redhat.com>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! portions by Daniel Zahn <stumpy@religions.com>
//! Rahul Jain <rahul@rice.edu> added support for TrueColor displays.
//! Jamie Zawinski <jwz@jwz.org> pieced several versions together in 1999.
//! ```
//!
//! The oldest trick in demo graphics, and still the best one. A row of random
//! noise along the bottom edge, and then every cell pushes some of its heat
//! into the three cells above it and keeps a fraction of what is left. Heat
//! rises, spreads sideways as it goes, and fades; nobody models a flame
//! anywhere in it.
//!
//! Everything is done at half resolution and doubled on the way out, with each
//! output pixel of a two by two block averaged against its right and lower
//! neighbours, which is the cheap blur that keeps the fire from looking like
//! a grid. The palette is the whole colour model: one ramp where red saturates
//! first, then green, then blue, so a cell's temperature reads as black, red,
//! orange, yellow, white in that order without anything ever choosing a
//! colour.
//!
//! An image can be set alight as well: it is turned to greyscale, and its
//! brightness is added into the fire as extra fuel, so the picture burns from
//! the bottom up rather than sitting on top of the flames. Upstream burns a
//! logo compiled into the binary; the image channel supplies the picture here,
//! scaled down to leave the fire room to work.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, XColor, rgb, unrgb};
use crate::runtime::{
    About, Dpy, ImageLoad, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, random_below,
};

const MAX_VAL: i32 = 255;

struct State {
    width: i32,
    height: i32,
    /// The fire, at half the resolution of the window, with a one cell gutter
    /// all the way round.
    flame: Vec<u8>,
    fwidth: i32,
    fheight: i32,
    /// The highest row that still has any heat in it. Everything above is
    /// black and is not redrawn.
    top: i32,
    ctab: [Pixel; 256],

    hspread: i32,
    vspread: i32,
    residual: i32,
    ihspread: i32,
    ivspread: i32,
    iresidual: i32,
    variance: i32,
    vartrend: i32,
    bloom: bool,

    delay: u32,
    baseline: i32,
    /// The greyscale picture to set alight, and its size.
    fuel: Option<Vec<u8>>,
    fuel_w: i32,
    fuel_h: i32,
    img_loader: Option<ImageLoad>,
    loading: bool,
}

impl State {
    /// A ramp in which red saturates first, then green, then blue, so heat
    /// reads as black, red, orange, yellow, white without anything choosing.
    fn init_colors(&mut self, fg: Pixel) {
        let (fr, fg_, fb) = unrgb(fg);
        let red = 255 - fr as i32;
        let green = 255 - fg_ as i32;
        let blue = 255 - fb as i32;

        let mut j = 0;
        let mut i = 0;
        while i < 256 * 2 {
            let r = ((i - red) * 3).clamp(0, 255) as u8;
            let g = ((i - green) * 3).clamp(0, 255) as u8;
            let b = ((i - blue) * 3).clamp(0, 255) as u8;
            self.ctab[j] = rgb(r, g, b);
            j += 1;
            i += 2;
        }
    }

    fn init_flame(&mut self) {
        self.fwidth = self.width / 2;
        self.fheight = self.height / 2;
        self.flame = vec![0u8; ((self.fwidth + 2) * (self.fheight + 2)) as usize];
        self.top = 1;
        self.hspread = self.ihspread;
        self.vspread = self.ivspread;
        self.residual = self.iresidual;
    }

    /// Stoke the bottom row with noise, and let the spread parameters wander
    /// a little before being pulled back towards their settings.
    fn flame_active(&mut self) {
        let base = ((self.fheight + 1) * (self.fwidth + 2)) as usize;
        for x in 0..(self.fwidth + 2) as usize {
            let mut v1 = self.flame[base + x] as i32;
            v1 += random_below(self.variance.max(1)) - self.vartrend;
            // Upstream's remainder is signed and the store is not: a cell
            // that goes below zero comes back near the top of the range,
            // which is where the flicker comes from.
            self.flame[base + x] = (v1 % 255) as u8;
        }

        if self.bloom {
            match random_below(100) {
                10 => self.residual += random_below(10),
                20 => self.hspread += random_below(15),
                30 => self.vspread += random_below(20),
                _ => {}
            }
        }

        self.residual = (self.iresidual * 10 + self.residual * 90) / 100;
        self.hspread = (self.ihspread * 10 + self.hspread * 90) / 100;
        self.vspread = (self.ivspread * 10 + self.vspread * 90) / 100;
    }

    /// One step of the fire: every cell pushes heat up and to both sides, and
    /// keeps a fraction of what is left.
    fn flame_advance(&mut self) {
        let stride = (self.fwidth + 2) as usize;
        let mut newtop = self.top;

        for y in (self.top..=self.fheight + 1).rev() {
            let mut used = false;
            let row = 1 + y as usize * stride;
            for x in 0..self.fwidth as usize {
                let p1 = row + x;
                let v1 = self.flame[p1] as i32;
                if v1 > 0 {
                    used = true;
                    let p2 = p1 - stride;

                    let mut v3 = (v1 * self.vspread) >> 8;
                    let mut v2 = self.flame[p2] as i32 + v3;
                    self.flame[p2] = v2.min(MAX_VAL) as u8;

                    v3 = (v1 * self.hspread) >> 8;
                    v2 = self.flame[p2 + 1] as i32 + v3;
                    self.flame[p2 + 1] = v2.min(MAX_VAL) as u8;

                    v2 = self.flame[p2 - 1] as i32 + v3;
                    self.flame[p2 - 1] = v2.min(MAX_VAL) as u8;

                    if y < self.fheight + 1 {
                        self.flame[p1] = ((v1 * self.residual) >> 8) as u8;
                    }
                }
                if used {
                    newtop = y - 1;
                }
            }

            // Clean up the right gutter.
            let g = row + self.fwidth as usize;
            self.flame[g] = ((self.flame[g] as i32 * self.residual) >> 8) as u8;
        }

        self.top = (newtop - 1).max(1);
    }

    /// Add the picture's brightness into the fire as extra fuel.
    fn flame_paste_data(&mut self, xx: i32, yy: i32, w: i32, h: i32) {
        let Some(data) = self.fuel.take() else {
            return;
        };
        let (xx, yy) = (xx.max(0), yy.max(0));
        if xx + w <= self.fwidth && yy + h <= self.fheight {
            let stride = (self.fwidth + 2) as usize;
            let mut src = 0usize;
            for y in 0..h {
                let row = 1 + xx as usize + (yy + y) as usize * stride;
                for x in 0..w as usize {
                    let v = data[src] / 24;
                    if v != 0 {
                        let p1 = row + x;
                        self.flame[p1] = self.flame[p1].wrapping_add(random_below(v as i32) as u8);
                    }
                    src += 1;
                }
            }
        }
        self.fuel = Some(data);
    }

    /// Double the fire into the window, averaging each output pixel of a two
    /// by two block against its right and lower neighbours.
    fn flame_to_image(&self, d: &mut Dpy) {
        let stride = (self.fwidth + 2) as usize;
        for y in self.top..self.fheight {
            let row = 1 + y as usize * stride;
            for x in 0..self.fwidth as usize {
                let p1 = row + x;
                let v1 = self.flame[p1] as usize;
                let v2 = self.flame[p1 + 1] as usize;
                let v3 = self.flame[p1 + stride] as usize;
                let v4 = self.flame[p1 + stride + 1] as usize;
                let (px, py) = ((x as i32) << 1, y << 1);
                let w = d.win();
                w.put_pixel(px, py, self.ctab[v1]);
                w.put_pixel(px + 1, py, self.ctab[(v1 + v2) >> 1]);
                w.put_pixel(px, py + 1, self.ctab[(v1 + v3) >> 1]);
                w.put_pixel(px + 1, py + 1, self.ctab[(v1 + v4) >> 1]);
            }
        }
    }

    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    /// Upstream's `loadBitmap`: turn the picture to greyscale and invert it,
    /// so the dark parts of the image are the ones that burn hardest. Its
    /// bundled logo is small and gets doubled up to size; the channel here
    /// hands over a window-sized picture, so this samples down instead.
    fn image_arrived(&mut self, d: &mut Dpy) {
        self.loading = false;

        // Upstream's logo is a small badge in a much larger grid. Keep that
        // proportion: a picture that fills the grid is all fuel and no fire.
        let max_w = (self.fwidth / 3).max(1);
        let max_h = (self.fheight / 3).max(1);
        let (sw, sh) = (d.width().max(1), d.height().max(1));
        let scale = (sw as f64 / max_w as f64)
            .max(sh as f64 / max_h as f64)
            .max(1.0);
        let w = ((sw as f64 / scale) as i32).clamp(1, max_w);
        let h = ((sh as f64 / scale) as i32).clamp(1, max_h);

        let mut out = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let sx = (x as f64 * scale) as i32;
                // The picture is read bottom-up, so it burns from its base.
                let sy = ((h - y - 1) as f64 * scale) as i32;
                let c = XColor::from_pixel(d.win_ref().get_pixel(sx.min(sw - 1), sy.min(sh - 1)));
                let mut gray =
                    ((c.red >> 8) as u32 + (c.green >> 8) as u32 + (c.blue >> 8) as u32) / 3;
                if gray < 96 {
                    gray /= 2; // A little more contrast.
                }
                out.push((255 - gray) as u8);
            }
        }

        self.fuel = Some(out);
        self.fuel_w = w;
        self.fuel_h = h;

        // The channel draws the picture into the window to hand it over.
        // Upstream starts on a black window, and the rows above the fire are
        // never redrawn, so anything left up there would stay forever.
        d.clear_window();
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // Upstream rounds the window up to an even size and lets the extra column
    // and row fall off the edge.
    let mut width = d.width();
    let mut height = d.height();
    if width % 2 != 0 {
        width += 1;
    }
    if height % 2 != 0 {
        height += 1;
    }

    // Upstream prints a message and exits if any of these is out of range.
    let ihspread = d.res.int("hspread").clamp(0, 255);
    let ivspread = d.res.int("vspread").clamp(0, 255);
    let iresidual = d.res.int("residual").clamp(0, 255);

    let mut st = State {
        width,
        height,
        flame: Vec::new(),
        fwidth: 1,
        fheight: 1,
        top: 1,
        ctab: [0; 256],
        hspread: ihspread,
        vspread: ivspread,
        residual: iresidual,
        ihspread,
        ivspread,
        iresidual,
        variance: d.res.int("variance").clamp(0, 255),
        vartrend: d.res.int("vartrend").clamp(0, 255),
        bloom: d.res.bool("bloom"),
        delay: d.res.int("delay").max(0) as u32,
        baseline: d.res.int("bitmapBaseline"),
        fuel: None,
        fuel_w: 0,
        fuel_h: 0,
        img_loader: None,
        loading: false,
    };
    let fg = d.res.pixel("foreground");
    st.init_colors(fg);
    st.init_flame();
    if d.res.string("bitmap") != "none" {
        st.start_load(d);
    }
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

        self.flame_active();

        if self.fuel.is_some() {
            let (x, y) = (
                (self.fwidth - self.fuel_w) / 2,
                self.fheight - self.fuel_h - self.baseline,
            );
            let (w, h) = (self.fuel_w, self.fuel_h);
            self.flame_paste_data(x, y, w, h);
        }

        self.flame_advance();
        self.flame_to_image(d);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.width = width + (width % 2);
        self.height = height + (height % 2);
        self.init_flame();
        d.clear_window();
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: #FFAF5F",
    "*fpsTop: true",
    "*fpsSolid: true",
    "*bitmap: (default)",
    "*bitmapBaseline: 20",
    "*delay: 10000",
    "*hspread: 30",
    "*vspread: 97",
    "*residual: 99",
    "*variance: 50",
    "*vartrend: 20",
    "*bloom: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::boolean("bloom", "Enable blooming", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "xflame",
    label: "XFlame",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Haitzler and many others",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=jUJiULU4i0k"),
        blurb: "Pulsing fire. It can also take an arbitrary image and set it on fire too.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
