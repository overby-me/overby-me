//! Port of `hacks/slip.c`.
//!
//! ```text
//! slip --- lots of slipping blits
//!
//! Copyright (c) 1992 by Scott Draves <spot@cs.cmu.edu>
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
//! 01-Nov-2000: Allocation checks
//! 10-May-1997: Jamie Zawinski <jwz@jwz.org> compatible with xscreensaver
//! 01-Dec-1995: Patched for VMS <joukj@hrem.stm.tudelft.nl>
//! ```
//!
//! Nothing is drawn: small blocks of whatever is already on screen are copied
//! a short distance, thousands of times a frame, and the direction comes from
//! one of three vector fields. The rotor field swirls, shuffle jitters each
//! block a few pixels, and explode pushes everything away from the middle.
//! Now and then the screen is seeded with random squares and a fresh picture,
//! and the whole thing starts eating that instead.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand};
use crate::runtime::{
    About, Dpy, ImageLoad, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, frand,
    screenhack_event_helper,
};

struct State {
    mi: ModeInfo,
    width: i32,
    height: i32,
    nblits_remaining: i64,
    blit_width: i32,
    blit_height: i32,
    mode: usize,
    first_time: bool,
    backwards: bool,
    /// The unused half of the last random word, kept for the next call.
    lasthalf: u16,
    /// How many nibbles are left in `r`.
    stage: u32,
    r: u32,
    img_loader: Option<ImageLoad>,
    image_loading_p: bool,
}

impl State {
    /// One random word yields two values, which is what the hack wants when it
    /// is asking for thousands of small numbers a frame.
    fn halfrandom(&mut self, mv: i32) -> i32 {
        if mv <= 0 {
            return 0;
        }
        let r: u32 = if self.lasthalf != 0 {
            let r = self.lasthalf as u32;
            self.lasthalf = 0;
            r
        } else {
            let r = lrand();
            self.lasthalf = (r >> 16) as u16;
            r
        };
        (r % mv as u32) as i32
    }

    /// Four bits at a time out of one word, signed by the top bit of each
    /// nibble, which is how the shuffle mode gets its small jitters.
    fn erandom(&mut self, mv: i32) -> f64 {
        if self.stage == 0 {
            self.r = lrand();
            self.stage = 7;
        }
        let res = (self.r & 0xf) as i32;
        self.r >>= 4;
        self.stage -= 1;
        if res & 8 != 0 {
            (res & mv) as f64
        } else {
            -((res & mv) as f64)
        }
    }

    /// Seed the screen with random squares, then ask for a fresh picture.
    /// Returns false when it has decided to start over instead.
    fn prepare_screen(&mut self, d: &mut Dpy) -> bool {
        let w = self.width / 20;
        let not_solid = self.halfrandom(10);
        let n;

        // Go the other way sometimes.
        self.backwards = lrand() & 1 == 1;

        if self.first_time {
            d.clear_window();
            n = 300;
        } else if !self.image_loading_p && self.halfrandom(10) == 0 {
            self.first_time = true;
            self.nblits_remaining = 0;
            d.clear_window();
            return false;
        } else {
            if self.halfrandom(5) != 0 {
                return true;
            }
            n = if self.halfrandom(5) != 0 { 100 } else { 2000 };
        }

        let pick = |st: &mut Self| {
            if st.mi.npixels() > 2 {
                let i = st.halfrandom(st.mi.npixels()) as usize;
                st.mi.pixel(i)
            } else if st.halfrandom(2) != 0 {
                st.mi.white
            } else {
                st.mi.black
            }
        };
        let p = pick(self);
        self.mi.gc.set_foreground(p);

        for _ in 0..n {
            let ww = (w / 2) + self.halfrandom(w.max(1));
            if not_solid != 0 {
                let p = pick(self);
                self.mi.gc.set_foreground(p);
            }
            let x = self.halfrandom((self.width - ww).max(1));
            let y = self.halfrandom((self.height - ww).max(1));
            d.win().fill_rectangle(&self.mi.gc, x, y, ww, ww);
        }
        self.first_time = false;

        // Sometimes hack the desktop image, says jwz; the condition upstream
        // leaves in front of that is `1 ||`, so it is every time.
        if !self.image_loading_p {
            self.image_loading_p = true;
            self.img_loader = d.load_image_async_simple(None);
            if self.img_loader.is_none() {
                self.image_loading_p = false;
            }
        }

        true
    }
}

/// `quantize`: floor, then round up with probability equal to the fraction, so
/// a fractional velocity moves a block that fraction of the time.
fn quantize(d: f64) -> i32 {
    let i = d.floor();
    let f = d - i;
    let mut i = i as i32;
    if frand(1.0) < f {
        i += 1;
    }
    i
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mi = ModeInfo::new(d, ColorScheme::Random);
    let (width, height) = (mi.width, mi.height);
    Box::new(State {
        mi,
        width,
        height,
        nblits_remaining: 0,
        blit_width: width / 25,
        blit_height: height / 25,
        mode: 0,
        first_time: true,
        backwards: false,
        lasthalf: 0,
        stage: 0,
        r: 0,
        img_loader: None,
        image_loading_p: false,
    })
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.image_loading_p {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            self.image_loading_p = self.img_loader.is_some();
        }

        let mut timer = self.mi.count as i64 * self.mi.cycles as i64;
        while timer > 0 {
            timer -= 1;
            let xi = self.halfrandom((self.width - self.blit_width).max(1));
            let yi = self.halfrandom((self.height - self.blit_height).max(1));
            let (mut dx, mut dy) = (0.0f64, 0.0f64);

            let n = self.nblits_remaining;
            self.nblits_remaining -= 1;
            if n == 0 {
                const LUT: [usize; 7] = [0, 0, 0, 1, 1, 1, 2];
                if !self.prepare_screen(d) {
                    break;
                }
                self.nblits_remaining = self.mi.count as i64
                    * (2000 + self.halfrandom(1000) as i64 + self.halfrandom(1000) as i64);
                self.mode = if self.mode == 2 {
                    self.halfrandom(2) as usize
                } else {
                    LUT[self.halfrandom(7) as usize]
                };
            }

            // (x, y) is in the biunit square.
            let x = (2 * xi + self.blit_width) as f64 / self.width as f64 - 1.0;
            let y = (2 * yi + self.blit_height) as f64 / self.height as f64 - 1.0;

            match self.mode {
                0 => {
                    // Rotor.
                    dx = x;
                    dy = y;
                    if dy < 0.0 {
                        dy += 0.04;
                        if dy > 0.0 {
                            dy = 0.0;
                        }
                    }
                    if dy > 0.0 {
                        dy -= 0.04;
                        if dy < 0.0 {
                            dy = 0.0;
                        }
                    }
                    let t = dx * dx + dy * dy + 1e-10;
                    let s1 = 2.0 * dx * dx / t - 1.0;
                    let s2 = 2.0 * dx * dy / t;
                    dx = s1 * 5.0;
                    dy = s2 * 5.0;
                    if self.backwards {
                        dx = -dx;
                        dy = -dy;
                    }
                }
                1 => {
                    // Shuffle.
                    dx = self.erandom(3);
                    dy = self.erandom(3);
                }
                2 => {
                    // Explode.
                    dx = x * 3.0;
                    dy = y * 3.0;
                }
                _ => {}
            }

            let qx = xi + quantize(dx);
            let qy = yi + quantize(dy);
            if qx < 0
                || qy < 0
                || qx >= self.width - self.blit_width
                || qy >= self.height - self.blit_height
            {
                continue;
            }

            let (bw, bh) = (self.blit_width, self.blit_height);
            d.win().copy_area_self(&self.mi.gc, xi, yi, bw, bh, qx, qy);

            if self.mode == 0 {
                // Wrap.
                let wrap = self.width - 2 * bw;
                if qx > wrap {
                    d.win()
                        .copy_area_self(&self.mi.gc, qx, qy, bw, bh, qx - wrap, qy);
                }
                if qx < 2 * bw {
                    d.win()
                        .copy_area_self(&self.mi.gc, qx, qy, bw, bh, qx + wrap, qy);
                }
                let wrap = self.height - 2 * bh;
                if qy > wrap {
                    d.win()
                        .copy_area_self(&self.mi.gc, qx, qy, bw, bh, qx, qy - wrap);
                }
                if qy < 2 * bh {
                    d.win()
                        .copy_area_self(&self.mi.gc, qx, qy, bw, bh, qx, qy + wrap);
                }
            }
        }

        self.mi.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.width = width;
        self.height = height;
        self.blit_width = width / 25;
        self.blit_height = height / 25;
        self.mode = 0;
        self.nblits_remaining = 0;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.first_time = true;
            self.nblits_remaining = 0;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 50000",
    "*count: 35",
    "*cycles: 50",
    "*ncolors: 200",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "50000").inverted(),
    Opt::slider("count", "Count", 0.0, 100.0, 1.0, 0, "35"),
    Opt::slider("cycles", "Timeout", 0.0, 100.0, 1.0, 0, "50"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "200"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "slip",
    label: "Slip",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Scott Draves and Jamie Zawinski",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=BgzNvBm4MuE"),
        blurb: "A jet engine consumes the image, then puts it through a spin cycle.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
