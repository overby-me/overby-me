//! Port of `hacks/blitspin.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2018 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Rotate a bitmap using using bitblts.
//! The bitmap must be square, and must be a power of 2 in size.
//! This was translated from SmallTalk code which appeared in the
//! August 1981 issue of Byte magazine.
//!
//! The input bitmap may be non-square, it is padded and centered
//! with the background color.  Another way would be to subdivide
//! the bitmap into square components and rotate them independently
//! (and preferably in parallel), but I don't think that would be as
//! interesting looking.
//!
//! It's too bad almost nothing uses blitter hardware these days,
//! or this might actually win.
//! ```
//!
//! Fifteen overlapping blits with `or`, `and` and `xor` swap the quadrants of
//! a square clockwise, then the same fifteen do it again on quadrants half the
//! size, all of them at once. Six or eight rounds of that and the picture has
//! turned ninety degrees, having passed through what looks like static on the
//! way. No pixel is ever addressed individually, which was the point in 1981.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, GXFunc, Gc, ImageLoad, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, StartArgs,
    XEvent, screenhack_event_helper,
};

/// The three working buffers, which the blit sequence names over and over.
const SELF: usize = 0;
const TEMP: usize = 1;
const MASK: usize = 2;

struct State {
    width: i32,
    height: i32,
    size: i32,
    scale_up: bool,
    bufs: [Pixmap; 3],
    gc: Gc,
    delay: u32,
    delay2: u32,
    duration: f64,
    bitmap: Pixmap,
    fg: Pixel,
    bg: Pixel,

    /// How big the quadrants are this round. `-1` means start a new rotation.
    qwad: i32,
    first_time: bool,
    last_w: i32,
    last_h: i32,

    start_time: f64,
    loaded_p: bool,
    img_loader: Option<ImageLoad>,
    loading: bool,
}

/// Round up to a power of two.
fn to_pow2(n: i32) -> i32 {
    let mut p = 1;
    while p < n {
        p <<= 1;
    }
    p
}

fn blitspin_to_pow2(n: i32, up: bool) -> i32 {
    let pow2 = to_pow2(n);
    if n == pow2 {
        n
    } else if up {
        pow2
    } else {
        pow2 >> 1
    }
}

impl State {
    /// One blit. Upstream builds six graphics contexts, one per raster op, and
    /// lets `XCopyArea` do the work; here the op rides on the context and the
    /// source is snapshotted first, since several of these copy a buffer onto
    /// itself with the regions overlapping.
    #[allow(clippy::too_many_arguments)]
    fn bitblt(
        &mut self,
        from: usize,
        to: usize,
        op: GXFunc,
        sx: i32,
        sy: i32,
        w: i32,
        h: i32,
        dx: i32,
        dy: i32,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        let mut gc = Gc::new(self.fg, self.bg);
        gc.set_function(op);
        if op == GXFunc::Clear || op == GXFunc::Set {
            // No source is read: these just write zeroes or ones.
            self.bufs[to].fill_rectangle(&gc, dx, dy, w, h);
            return;
        }
        let src = self.bufs[from].sub_image(sx, sy, w, h);
        self.bufs[to].copy_area(&gc, &src, 0, 0, w, h, dx, dy);
    }

    /// `copy_to`: the whole square less an offset, landing at that offset.
    #[allow(clippy::too_many_arguments)]
    fn copy_to(&mut self, from: usize, xoff: i32, yoff: i32, to: usize, op: GXFunc) {
        let (w, h) = (self.size - xoff, self.size - yoff);
        self.bitblt(from, to, op, 0, 0, w, h, xoff, yoff);
    }

    /// `copy_from`: the square from an offset inwards, landing at the origin.
    /// Note the argument order upstream gives this one: the buffer named first
    /// is the destination.
    #[allow(clippy::too_many_arguments)]
    fn copy_from(&mut self, to: usize, xoff: i32, yoff: i32, from: usize, op: GXFunc) {
        let (w, h) = (self.size - xoff, self.size - yoff);
        self.bitblt(from, to, op, xoff, yoff, w, h, 0, 0);
    }

    fn display(&mut self, d: &mut Dpy) {
        let (w, h) = (d.width(), d.height());
        if w != self.last_w || h != self.last_h {
            d.clear_window();
            self.last_w = w;
            self.last_h = h;
        }
        let s = self.size;
        d.win().copy_area(
            &self.gc,
            &self.bufs[SELF],
            0,
            0,
            s,
            s,
            (w - s) >> 1,
            (h - s) >> 1,
        );
    }

    /// Size the square, and lay the picture into the middle of it.
    fn init_2(&mut self) {
        // Make it square, then round to a power of two, then do not exceed
        // the screen.
        self.size = self.width.max(self.height);
        self.size = blitspin_to_pow2(self.size, self.scale_up);
        let w = blitspin_to_pow2(self.width, false);
        let h = blitspin_to_pow2(self.height, false);
        self.size = self.size.min(w).min(h).max(1);

        let s = self.size;
        self.bufs = [Pixmap::new(s, s), Pixmap::new(s, s), Pixmap::new(s, s)];

        // Clear self to the background colour, not to zero as `clear` does.
        self.bufs[SELF].clear(self.bg);
        let gc = Gc::new(self.fg, self.bg);
        let (bw, bh) = (self.width, self.height);
        let src = self.bitmap.clone();
        self.bufs[SELF].copy_area(&gc, &src, 0, 0, bw, bh, (s - bw) >> 1, (s - bh) >> 1);

        self.qwad = -1;
        self.first_time = true;
    }

    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    fn image_arrived(&mut self, d: &mut Dpy) {
        self.loading = false;
        self.bitmap = d.win_ref().sub_image(0, 0, self.width, self.height);
        self.first_time = false;
        self.loaded_p = true;
        self.qwad = -1;
        self.start_time = d.time;
        self.init_2();
        d.clear_window();
        self.last_w = 0;
        self.last_h = 0;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");
    let (width, height) = (d.width(), d.height());

    let mut st = State {
        width,
        height,
        size: 1,
        // The picture comes off the screen, so scaling up is the guess
        // upstream makes.
        scale_up: true,
        bufs: [Pixmap::new(1, 1), Pixmap::new(1, 1), Pixmap::new(1, 1)],
        gc: Gc::new(fg, bg),
        delay: d.res.int("delay").max(0) as u32,
        delay2: d.res.int("delay2").max(0) as u32,
        duration: d.res.int("duration").max(1) as f64,
        bitmap: Pixmap::new(width, height),
        fg,
        bg,
        qwad: -1,
        first_time: true,
        last_w: 0,
        last_h: 0,
        start_time: 0.0,
        loaded_p: false,
        img_loader: None,
        loading: false,
    };
    st.start_load(d);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut this_delay = self.delay;

        if self.loading {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            // Rotate nothing if the very first image is not yet loaded.
            if !self.loaded_p {
                return this_delay;
            }
        }

        if !self.loading && self.start_time + self.duration < d.time {
            // Start a new image loading, but keep rotating the old one until
            // the new one arrives.
            self.start_load(d);
        }

        if self.qwad == -1 {
            let s = self.size;
            self.bitblt(MASK, MASK, GXFunc::Clear, 0, 0, s, s, 0, 0);
            self.bitblt(MASK, MASK, GXFunc::Set, 0, 0, s >> 1, s >> 1, 0, 0);
            self.qwad = s >> 1;
        }

        if self.first_time {
            self.first_time = false;
            self.display(d);
            return self.delay2;
        }

        let q = self.qwad;
        self.copy_to(MASK, 0, 0, TEMP, GXFunc::Copy); // 1
        self.copy_to(MASK, 0, q, TEMP, GXFunc::Or); // 2
        self.copy_to(SELF, 0, 0, TEMP, GXFunc::And); // 3
        self.copy_to(TEMP, 0, 0, SELF, GXFunc::Xor); // 4
        self.copy_from(TEMP, q, 0, SELF, GXFunc::Xor); // 5
        self.copy_from(SELF, q, 0, SELF, GXFunc::Or); // 6
        self.copy_to(TEMP, q, 0, SELF, GXFunc::Xor); // 7
        self.copy_to(SELF, 0, 0, TEMP, GXFunc::Copy); // 8
        self.copy_from(TEMP, q, q, SELF, GXFunc::Xor); // 9
        self.copy_to(MASK, 0, 0, TEMP, GXFunc::And); // A
        self.copy_to(TEMP, 0, 0, SELF, GXFunc::Xor); // B
        self.copy_to(TEMP, q, q, SELF, GXFunc::Xor); // C
        self.copy_from(MASK, q >> 1, q >> 1, MASK, GXFunc::And); // D
        self.copy_to(MASK, q, 0, MASK, GXFunc::Or); // E
        self.copy_to(MASK, 0, q, MASK, GXFunc::Or); // F
        self.display(d);

        self.qwad >>= 1;
        if self.qwad == 0 {
            // Done with this round.
            self.qwad = -1;
            this_delay = self.delay2;
        }

        this_delay
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.start_time = f64::NEG_INFINITY;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    ".fpsSolid: true",
    "*delay: 500000",
    "*delay2: 500000",
    "*duration: 120",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "delay",
        "Fuzzy rotation speed",
        1.0,
        800_000.0,
        10_000.0,
        0,
        "500000",
    )
    .inverted(),
    Opt::slider(
        "delay2",
        "90 degree rotation speed",
        1.0,
        800_000.0,
        10_000.0,
        0,
        "500000",
    )
    .inverted(),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "blitspin",
    label: "Blit Spin",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=UTtcwb-UWW8"),
        blurb: "Repeatedly rotates an image by 90 degrees by using bitwise-logical operations.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
