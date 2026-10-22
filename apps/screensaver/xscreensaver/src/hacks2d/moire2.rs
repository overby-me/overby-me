//! Port of `hacks/moire2.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1997-2013 Jamie Zawinski <jwz@jwz.org>
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
//! Two or three planes of concentric rings, each four times the size of the
//! screen, slid over one another and combined bit by bit with XOR or OR. Where
//! the rings nearly line up the interference sprays out into the coarse bands
//! that give the effect its name. The planes are one-bit bitmaps, so the
//! combination is exact, and the result is stamped onto the screen in two
//! colours that cycle underneath it.
//!
//! Upstream's double buffering is left out: it exists to stop X flickering, and
//! this port composes a whole frame in memory before it reaches the canvas.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::{
    About, Dpy, GXFunc, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XColor,
    frand, random,
};

struct Moire2 {
    ncolors: usize,
    colors: Vec<XColor>,
    mono: bool,
    fg_pixel: Pixel,
    bg_pixel: Pixel,
    /// The composed plane, and the two or three ring planes it is built from.
    p0: Pixmap,
    p1: Pixmap,
    p2: Pixmap,
    p3: Option<Pixmap>,
    copy_gc: Gc,
    erase_gc: Gc,
    window_gc: Gc,
    width: i32,
    height: i32,
    size: i32,
    /// Where each plane currently sits, and how fast it is sliding.
    x1: i32,
    x2: i32,
    x3: i32,
    y1: i32,
    y2: i32,
    y3: i32,
    dx1: i32,
    dx2: i32,
    dx3: i32,
    dy1: i32,
    dy2: i32,
    dy3: i32,
    othickness: i32,
    thickness: i32,
    do_three: bool,
    flip_a: bool,
    flip_b: bool,
    pix: usize,
    delay: u32,
    color_shift: i32,
    reset: bool,
    iterations: i32,
    iteration: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut mono = d.mono_p;
    let mut ncolors = if mono { 2 } else { d.res.int("colors") };
    if ncolors < 2 {
        ncolors = 2;
    }
    if ncolors <= 2 {
        mono = true;
    }
    let colors = if mono {
        Vec::new()
    } else {
        make_smooth_colormap(ncolors as usize)
    };

    let fg_pixel = d.res.pixel("foreground");
    let bg_pixel = d.res.pixel("background");

    let mut st = Moire2 {
        ncolors: ncolors as usize,
        colors,
        mono,
        fg_pixel,
        bg_pixel,
        p0: Pixmap::new_bitmap(1, 1),
        p1: Pixmap::new_bitmap(1, 1),
        p2: Pixmap::new_bitmap(1, 1),
        p3: None,
        copy_gc: Gc::new(1, 0),
        erase_gc: Gc::new(0, 0),
        window_gc: Gc::new(fg_pixel, bg_pixel),
        width: 0,
        height: 0,
        size: 0,
        x1: 0,
        x2: 0,
        x3: 0,
        y1: 0,
        y2: 0,
        y3: 0,
        dx1: 0,
        dx2: 0,
        dx3: 0,
        dy1: 0,
        dy2: 0,
        dy3: 0,
        othickness: d.res.int("thickness"),
        thickness: 1,
        do_three: false,
        flip_a: false,
        flip_b: false,
        pix: 0,
        delay: d.res.int("delay").max(0) as u32,
        color_shift: d.res.int("colorShift").max(1),
        reset: true,
        iterations: 0,
        iteration: 0,
    };
    // Upstream builds the planes on the first draw, but the framebuffer has to
    // be a real size before then.
    st.width = d.width();
    st.height = d.height();
    Box::new(st)
}

/// The movement half of upstream's two `FROB` macros: step a plane, bounce it
/// off the ends, and occasionally change its mind about the speed.
fn frob_move(n: &mut i32, dn: &mut i32, max: i32) {
    *n += *dn;
    if *n <= 0 {
        *n = 0;
        *dn = -*dn;
    } else if *n >= max {
        *n = max;
        *dn = -*dn;
    } else if random().is_multiple_of(100) {
        *dn = -*dn;
    } else if random().is_multiple_of(50) {
        *dn += if *dn <= -20 {
            1
        } else if *dn >= 20 {
            -1
        } else if random() & 1 == 1 {
            1
        } else {
            -1
        };
    }
}

/// The setup half: a random starting offset and speed.
fn frob_init(n: &mut i32, dn: &mut i32, max: i32, thickness: i32) {
    *n = (max / 2) + (random() % max.max(1) as u32) as i32;
    *dn = (1 + (random() % (7 * thickness) as u32) as i32) * if random() & 1 == 1 { 1 } else { -1 };
}

impl Moire2 {
    /// Fill one plane with concentric rings, then invert it one time in five.
    fn draw_rings(
        plane: &mut Pixmap,
        gc: &mut Gc,
        size: i32,
        width: i32,
        height: i32,
        xor: bool,
        thickness: i32,
    ) {
        let mut maxx = size * 4;
        let mut maxy = size * 4;
        if random().is_multiple_of(5) {
            let f = 1.0 + frand(0.05);
            if random() & 1 == 1 {
                maxx = (maxx as f64 * f) as i32;
            } else {
                maxy = (maxy as f64 * f) as i32;
            }
        }

        let step = thickness + 1 + i32::from(!xor) + (random() % (4 * thickness) as u32) as i32;
        let mut i = 0;
        while i < size * 2 {
            plane.draw_arc(
                gc,
                i - size,
                i - size,
                maxx - i - i,
                maxy - i - i,
                0,
                FULL_CIRCLE,
            );
            i += step;
        }

        if random().is_multiple_of(5) {
            gc.set_function(GXFunc::Xor);
            plane.fill_rectangle(gc, 0, 0, width * 2, height * 2);
            gc.set_function(GXFunc::Copy);
        }
    }

    fn reset_planes(&mut self, d: &mut Dpy) {
        self.do_three = random().is_multiple_of(3);

        self.width = d.width();
        self.height = d.height();
        self.size = self.width.max(self.height);

        self.p0 = Pixmap::new_bitmap(self.width, self.height);
        self.p1 = Pixmap::new_bitmap(self.width * 2, self.height * 2);
        self.p2 = Pixmap::new_bitmap(self.width * 2, self.height * 2);
        self.p3 = if self.do_three {
            Some(Pixmap::new_bitmap(self.width * 2, self.height * 2))
        } else {
            None
        };

        self.thickness = if self.othickness > 0 {
            self.othickness
        } else {
            1 + (random() % 4) as i32
        };

        let mut gc = Gc::new(0, 0);
        gc.set_line_width(if self.thickness == 1 {
            0
        } else {
            self.thickness
        });
        gc.set_foreground(1);

        // A plane that is XOR-ed in wants thinner rings than one that is OR-ed,
        // since OR fills in every overlap.
        let xor = self.do_three || self.thickness == 1 || random() & 1 == 1;

        let (size, width, height, thickness) = (self.size, self.width, self.height, self.thickness);
        Self::draw_rings(&mut self.p1, &mut gc, size, width, height, xor, thickness);
        Self::draw_rings(&mut self.p2, &mut gc, size, width, height, xor, thickness);
        if let Some(p3) = self.p3.as_mut() {
            Self::draw_rings(p3, &mut gc, size, width, height, xor, thickness);
        }

        self.copy_gc = Gc::new(1, 0);
        self.copy_gc
            .set_function(if xor { GXFunc::Xor } else { GXFunc::Or });
        self.erase_gc = Gc::new(0, 0);
        self.window_gc = Gc::new(self.fg_pixel, self.bg_pixel);

        frob_init(&mut self.x1, &mut self.dx1, self.width, thickness);
        frob_init(&mut self.x2, &mut self.dx2, self.width, thickness);
        frob_init(&mut self.x3, &mut self.dx3, self.width, thickness);
        frob_init(&mut self.y1, &mut self.dy1, self.height, thickness);
        frob_init(&mut self.y2, &mut self.dy2, self.height, thickness);
        frob_init(&mut self.y3, &mut self.dy3, self.height, thickness);
    }

    fn compose(&mut self, d: &mut Dpy) {
        frob_move(&mut self.x1, &mut self.dx1, self.width);
        frob_move(&mut self.x2, &mut self.dx2, self.width);
        frob_move(&mut self.x3, &mut self.dx3, self.width);
        frob_move(&mut self.y1, &mut self.dy1, self.height);
        frob_move(&mut self.y2, &mut self.dy2, self.height);
        frob_move(&mut self.y3, &mut self.dy3, self.height);

        let (w, h) = (self.width, self.height);
        self.p0.fill_rectangle(&self.erase_gc, 0, 0, w, h);
        self.p0
            .copy_area(&self.copy_gc, &self.p1, self.x1, self.y1, w, h, 0, 0);
        self.p0
            .copy_area(&self.copy_gc, &self.p2, self.x2, self.y2, w, h, 0, 0);
        if let Some(p3) = self.p3.as_ref() {
            self.p0
                .copy_area(&self.copy_gc, p3, self.x3, self.y3, w, h, 0, 0);
        }

        d.win()
            .copy_plane(&self.window_gc, &self.p0, 0, 0, w, h, 0, 0);
    }
}

impl Screenhack for Moire2 {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.reset {
            self.reset = false;
            self.iteration = 0;
            self.iterations = 30 + (random() % 70) as i32 + (random() % 70) as i32;
            self.reset_planes(d);

            self.flip_a = !self.mono && random() & 1 == 1;
            self.flip_b = !self.mono && random() & 1 == 1;

            if self.flip_b {
                self.window_gc.set_foreground(self.bg_pixel);
                self.window_gc.set_background(self.fg_pixel);
            } else {
                self.window_gc.set_foreground(self.fg_pixel);
                self.window_gc.set_background(self.bg_pixel);
            }
        }

        if !self.mono {
            self.pix = (self.pix + 1) % self.ncolors;
            let c = self.colors[self.pix].pixel;
            if self.flip_a {
                self.window_gc.set_background(c);
            } else {
                self.window_gc.set_foreground(c);
            }
        }

        self.compose(d);

        self.iteration += 1;
        if self.iteration >= self.color_shift {
            self.iteration = 0;
            self.iterations -= 1;
            if self.iterations <= 0 {
                self.reset = true;
            }
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, _width: i32, _height: i32) {
        self.reset = true;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 50000",
    "*thickness: 0",
    "*colors: 150",
    "*colorShift: 5",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "50000").inverted(),
    Opt::slider("colors", "Number of colors", 1.0, 255.0, 1.0, 0, "150"),
    Opt::spin("thickness", "Thickness", 0.0, 100.0, "0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "moire2",
    label: "Moiré 2",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=7iBNbYCo8so"),
        blurb: "Fields of concentric circles, combined plane by plane.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
