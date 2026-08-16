//! Port of `hacks/flag.c`.
//!
//! ```text
//! flag --- a waving flag
//!
//! Copyright (c) 1996 Charles Vidal <vidalc@univ-mlv.fr>.
//! PEtite demo X11 de charles vidal 15 05 96
//! tourne sous Linux et SOLARIS
//! thank's to Bas van Gaalen, Holland, PD, for his sources
//! in pascal vous devez rajouter une ligne dans mode.c
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
//! ```
//!
//! A bitmap on a flag, and the flag is nothing but a table of sines: every
//! pixel of the picture is drawn at its own place plus a lookup indexed by
//! where it is and how far the animation has got. Two lookups, in fact, one for
//! each axis and at different rates, which is why the cloth ripples rather than
//! simply sliding.
//!
//! The picture is a bitmap, so it has no colour of its own. It is drawn in
//! black and the ground behind it takes the colour, cycling through the map as
//! the wave passes, which is what makes the flag look lit.
//!
//! Upstream flies either a face called Bob or the output of `uname`. There is
//! no `uname` in a browser, so the words are the ones upstream falls back to
//! without one.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::font::Font;
use crate::runtime::png;
use crate::runtime::xlockmore::{ColorScheme, MAXRAND, ModeInfo, lrand, nrand};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent,
};

const MIN_SIZE: i32 = 1;
const MAX_SCALE: i32 = 8;
const MIN_SCALE: i32 = 2;
const MAX_INIT_SIZE: f32 = 6.0;
const MIN_INIT_SIZE: f32 = 2.0;
const MIN_AMP: i32 = 5;
const MAX_AMP: i32 = 20;
const ANGLES: usize = 360;

/// The words to fly when there is no picture. Upstream builds this from
/// `uname()`; where there is none it uses exactly this.
const DEFAULT_TEXT: &str = "X\nScreen\nSaver";

struct Flag {
    mi: ModeInfo,
    /// Amplitude of the wave, and the offset it swings about.
    samp: i32,
    sofs: i32,
    /// How far round the wave the animation has got.
    sidx: i32,
    x_flag: i32,
    y_flag: i32,
    timer: i32,
    stab: [i32; ANGLES],
    cache: Pixmap,
    /// The size of one pixel of the picture on screen.
    pointsize: i32,
    /// Distance between those pixels, which breathes in and out.
    size: f32,
    inctaille: f32,
    startcolor: i32,
    /// The picture: depth 1, so it is a stencil rather than a picture.
    image: Pixmap,
    gc: Gc,
}

/// `random_num`: upstream's own, which reaches `n` inclusive and can go
/// negative when handed a negative count, as it is on a small screen.
fn random_num(n: i32) -> i32 {
    ((f64::from(lrand()) / MAXRAND) * (f64::from(n) + 1.0)) as i32
}

impl Flag {
    fn maxw(&self) -> i32 {
        MAX_SCALE * self.image.width() + 2 * MAX_AMP + self.pointsize
    }

    fn maxh(&self) -> i32 {
        MAX_SCALE * self.image.height() + 2 * MAX_AMP + self.pointsize
    }

    fn minw(&self) -> i32 {
        MIN_SCALE * self.image.width() + 2 * MIN_AMP + self.pointsize
    }

    fn minh(&self) -> i32 {
        MIN_SCALE * self.image.height() + 2 * MIN_AMP + self.pointsize
    }

    /// The wave itself. The period is picked at random from the first five
    /// powers of two: beyond about sixteen the cloth stops reading as cloth.
    fn init_sintab(&mut self) {
        let periodicity = random_num(4);
        let puissance = 1 << periodicity;
        for (i, s) in self.stab.iter_mut().enumerate() {
            *s = (((i * puissance) as f64 * std::f64::consts::PI / ANGLES as f64).sin()
                * f64::from(self.samp)) as i32
                + self.sofs;
        }
    }

    /// Draw the picture on to the cloth, one pixel of it at a time.
    fn affiche(&mut self) {
        let npixels = self.mi.npixels().max(1);
        for x in 0..self.image.width() {
            for y in (0..self.image.height()).rev() {
                let i = (self.sidx + x + y) as usize % ANGLES;
                let j = (self.sidx + 4 * x + y + y) as usize % ANGLES;
                let xp = (self.size * x as f32) as i32 + self.stab[i];
                let yp = (self.size * y as f32) as i32 + self.stab[j];

                if self.image.get_pixel(x, y) != 0 {
                    self.gc.set_foreground(self.mi.black);
                } else if self.mi.npixels() <= 2 {
                    self.gc.set_foreground(self.mi.white);
                } else {
                    let k = (y + x + self.sidx + self.startcolor) % npixels;
                    self.gc.set_foreground(self.mi.pixel(k as usize));
                }

                if self.pointsize <= 1 {
                    self.cache.draw_point(&self.gc, xp, yp);
                } else if self.pointsize < 6 {
                    self.cache
                        .fill_rectangle(&self.gc, xp, yp, self.pointsize, self.pointsize);
                } else {
                    self.cache.fill_arc(
                        &self.gc,
                        xp,
                        yp,
                        self.pointsize,
                        self.pointsize,
                        0,
                        360 * 64,
                    );
                }
            }
        }
    }

    /// `init_flag`: pick a wave, a point size and somewhere to fly.
    fn restart(&mut self, d: &mut Dpy) {
        let size = d.res.int("size");
        self.pointsize = size;
        if size < -MIN_SIZE {
            self.pointsize = nrand(-size - MIN_SIZE + 1) + MIN_SIZE;
        }
        if self.pointsize < MIN_SIZE || d.width() <= self.maxw() || d.height() <= self.maxh() {
            self.pointsize = MIN_SIZE;
        }
        self.size = MAX_INIT_SIZE;
        self.inctaille = 0.05;
        self.timer = 0;
        self.sidx = 0;
        self.x_flag = 0;
        self.y_flag = 0;

        self.gc.set_foreground(self.mi.black);
        let (w, h) = (self.maxw(), self.maxh());
        self.cache.fill_rectangle(&self.gc, 0, 0, w, h);

        if self.mi.npixels() > 2 {
            self.startcolor = nrand(self.mi.npixels());
        }
        if d.width() <= self.maxw() || d.height() <= self.maxh() {
            self.samp = MIN_AMP;
            self.sofs = 0;
            self.x_flag = random_num(d.width() - self.minw());
            self.y_flag = random_num(d.height() - self.minh());
        } else {
            self.samp = MAX_AMP;
            self.sofs = 20;
            self.x_flag = random_num(d.width() - self.maxw());
            self.y_flag = random_num(d.height() - self.maxh());
        }

        self.init_sintab();
        d.clear_window();
    }
}

impl Screenhack for Flag {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let gc = self.gc.clone();
        if d.width() <= self.maxw() || d.height() <= self.maxh() {
            self.size = MIN_INIT_SIZE;
            let (w, h) = (self.minw(), self.minh());
            d.win()
                .copy_area(&gc, &self.cache, 0, 0, w, h, self.x_flag, self.y_flag);
        } else {
            if self.size + self.inctaille > MAX_SCALE as f32 {
                self.inctaille = -self.inctaille;
            }
            if self.size + self.inctaille < MIN_SCALE as f32 {
                self.inctaille = -self.inctaille;
            }
            self.size += self.inctaille;
            let (w, h) = (self.maxw(), self.maxh());
            d.win()
                .copy_area(&gc, &self.cache, 0, 0, w, h, self.x_flag, self.y_flag);
        }

        self.gc.set_foreground(self.mi.black);
        let (w, h) = (self.maxw(), self.maxh());
        self.cache.fill_rectangle(&self.gc, 0, 0, w, h);
        self.affiche();

        self.sidx += 2;
        self.sidx %= ANGLES as i32 * self.mi.npixels().max(1);
        self.timer += 1;
        if self.mi.cycles > 0 && self.timer >= self.mi.cycles {
            self.restart(d);
        }
        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.restart(d);
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

/// `make_flag_bits`: turn either the picture or the words into the depth-1
/// stencil the flag is made of.
///
/// Upstream tosses a coin between them when neither was asked for, and so does
/// this, since there is no way to type a string into a screen saver here.
fn make_flag_bits(d: &mut Dpy) -> Pixmap {
    let text = d.res.string("text").to_string();
    if !text.is_empty() {
        return text_bits(d, &text);
    }
    if lrand() & 1 == 0 {
        return text_bits(d, DEFAULT_TEXT);
    }
    // The face is compiled in, so it does not fail to decode; if it somehow
    // did, fly the words rather than nothing.
    bob_bits().unwrap_or_else(|| text_bits(d, DEFAULT_TEXT))
}

/// Set the words in the flag's font and keep the bits.
fn text_bits(d: &mut Dpy, text: &str) -> Pixmap {
    let font = Font::load(d.res.string("font"));
    let margin = 2;
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    let widest = lines.iter().map(|l| font.text_width(l)).max().unwrap_or(0);

    let width = widest + margin + margin + 1;
    let height = (font.ascent() + font.descent()) * lines.len() as i32 + margin + margin;
    let mut bitmap = Pixmap::new_bitmap(width.max(1), height.max(1));

    // Depth 1, so the "colours" are the two bits: the words are 1 and the cloth
    // around them is 0.
    let mut gc = Gc::new(1, 0);
    gc.set_foreground(0);
    bitmap.fill_rectangle(&gc, 0, 0, width, height);
    gc.set_foreground(1);
    for (i, line) in lines.iter().enumerate() {
        let i = i as i32;
        let xoff = (widest - font.text_width(line)) / 2;
        bitmap.draw_string(
            &gc,
            &font,
            margin + xoff,
            font.ascent() * (i + 1) + font.descent() * i + margin,
            line,
        );
    }
    bitmap
}

/// Bob, reduced to one bit and turned the right way up.
fn bob_bits() -> Option<Pixmap> {
    let (img, mask) = png::decode(crate::images::BOB)?;
    let mut bitmap = Pixmap::new_bitmap(img.width(), img.height());
    for y in 0..img.height() {
        for x in 0..img.width() {
            // Upstream reads the source bottom-up, and treats a transparent
            // pixel as white so the ground drops out rather than going black.
            let sy = img.height() - y - 1;
            let clear = mask.as_ref().is_some_and(|m| m.get_pixel(x, sy) == 0);
            let red = if clear {
                0xFF
            } else {
                img.get_pixel(x, sy) & 0xFF
            };
            bitmap.put_pixel(x, y, u32::from(red <= 0x7F));
        }
    }
    Some(bitmap)
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mi = ModeInfo::new(d, ColorScheme::Uniform);
    let image = make_flag_bits(d);
    let white = mi.white;
    let black = mi.black;
    let mut st = Flag {
        mi,
        samp: MAX_AMP,
        sofs: 20,
        sidx: 0,
        x_flag: 0,
        y_flag: 0,
        timer: 0,
        stab: [0; ANGLES],
        // Sized once, for the largest the flag can ever be.
        cache: Pixmap::new(
            MAX_SCALE * image.width() + 2 * MAX_AMP + MIN_SIZE,
            MAX_SCALE * image.height() + 2 * MAX_AMP + MIN_SIZE,
        ),
        pointsize: MIN_SIZE,
        size: MAX_INIT_SIZE,
        inctaille: 0.05,
        startcolor: 0,
        image,
        gc: Gc::new(white, black),
    };
    st.restart(d);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:		50000",
    "*cycles:		1000",
    "*size:			-7",
    "*ncolors:		200",
    "*bitmap:",
    "*font:		-*-fixed-medium-r-*-*-*-100-*-*-c-*-*-*",
    "*text:",
    "*fpsSolid:		true",
    "*lowrez:       true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 200_000.0, 1000.0, 0, "50000").inverted(),
    Opt::slider("cycles", "Timeout", 0.0, 800_000.0, 1000.0, 0, "1000"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "flag",
    label: "Flag",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Charles Vidal and Jamie Zawinski",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=LuEC3EONzjc"),
        blurb: "A waving flag, with either a face or some words on it.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
