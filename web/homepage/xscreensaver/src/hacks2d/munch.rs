//! Port of `hacks/munch.c` (Munching Squares and Mismunch).
//!
//! ```text
//! Portions copyright 1992-2014 Jamie Zawinski <jwz@jwz.org>
//! Portions Copyright 1997, Tim Showalter
//! Portions Copyright 2004 Steven Hazel <sah@thalassocracy.org>
//!
//!   Permission to use, copy, modify, distribute, and sell this
//!   software and its documentation for any purpose is hereby
//!   granted without fee, provided that the above copyright notice
//!   appear in all copies and that both that copyright notice and
//!   this permission notice appear in supporting documentation.  No
//!   representations are made about the suitability of this software
//!   for any purpose.  It is provided "as is" without express or
//!   implied warranty.
//!
//! "munch.c" and "mismunch.c" merged by jwz, 29-Aug-2008.
//! ```
//!
//! HAKMEM item 146, Jackson Wright's 1962 PDP-1 display hack: plot
//! `y = x XOR t` for successive `t`. Mismunch mode feeds `y` back in place of
//! `x` and negates a couple of terms, which breaks the pattern in a way that is
//! more interesting than the pattern.
//!
//! Almost all of the look comes from drawing in XOR, so this is the port that
//! proves the framebuffer's raster operations.

use crate::runtime::{
    About, Dpy, GXFunc, Gc, Opt, SaverDef, Screenhack, SelectItem, XColor, XEvent, random,
    random_below, screenhack_event_helper,
};

struct Muncher {
    mismunch: bool,
    width: i32,
    at_x: i32,
    at_y: i32,
    k_x: i32,
    k_t: i32,
    k_y: i32,
    grav: bool,
    fgc: XColor,
    yshadow: i32,
    xshadow: i32,
    x: i32,
    y: i32,
    t: i32,
    doom: i32,
    done: bool,
}

struct Munch {
    gc: Gc,
    delay: u32,
    simul: usize,
    clear: i32,
    logminwidth: u32,
    logmaxwidth: u32,
    restart: bool,
    window_width: i32,
    window_height: i32,
    /// Squares finished since the last clear.
    draw_n: i32,
    draw_i: usize,
    mismunch: bool,
    /// The `mismunch` resource as configured, so a restart can re-roll
    /// "random" the way upstream re-reads the resource.
    mismunch_mode: String,
    munchers: Vec<Muncher>,
}

/// `i_log2` from `utils/pow2.h`: floor(log2(x)).
fn i_log2(x: f64) -> u32 {
    let x = x as usize;
    if x == 0 { 0 } else { x.ilog2() }
}

impl Munch {
    /// Choose a range of square sizes based on the window size. The width has
    /// to be a power of two or the munch doesn't fill up, and a square bigger
    /// than 80% of the window makes mismunch look like noise.
    fn calc_logwidths(&mut self) {
        if self.window_height < self.window_width && self.window_width < self.window_height * 5 {
            self.logmaxwidth = i_log2(self.window_height as f64 * 0.8);
        } else {
            self.logmaxwidth = i_log2(self.window_width as f64 * 0.8);
        }
        if self.logmaxwidth < 2 {
            self.logmaxwidth = 2;
        }
        // We always want three sizes of squares.
        self.logminwidth = self.logmaxwidth.saturating_sub(2).max(2);
    }

    fn make_muncher(&self, width: i32, height: i32) -> Muncher {
        let logwidth = self.logminwidth + (random() % (1 + self.logmaxwidth - self.logminwidth));
        let w = 1 << logwidth;

        let at_x = random_below(if width <= w { 1 } else { width - w });
        let at_y = random_below(if height <= w { 1 } else { height - w });

        // Wrap-around by these values; no need to reduce, that happens later.
        let k_x = if random() % 2 == 1 {
            random_below(w)
        } else {
            0
        };
        let k_t = if random() % 2 == 1 {
            random_below(w)
        } else {
            0
        };
        let k_y = if random() % 2 == 1 {
            random_below(w)
        } else {
            0
        };

        // I like this color scheme better than random colors.
        let fgc = match random() % 4 {
            0 => XColor::from_rgb16(
                (random() % 65536) as u16,
                (random() % 16384) as u16,
                (random() % 32768) as u16,
            ),
            1 => XColor::from_rgb16(0, (random() % 16384) as u16, (random() % 65536) as u16),
            2 => XColor::from_rgb16(
                (random() % 8192) as u16,
                (random() % 49152) as u16,
                (random() % 8192) as u16,
            ),
            _ => {
                let g = (random() % 65536) as u16;
                XColor::from_rgb16(g, g, g)
            }
        };

        // Sometimes draw a mostly-overlapping copy of the square. In XOR mode
        // that generates all kinds of neat blocky graphics.
        let (xshadow, yshadow) = if !self.mismunch || !random().is_multiple_of(4) {
            (0, 0)
        } else {
            (random_below(w / 3) - (w / 6), random_below(w / 3) - (w / 6))
        };

        Muncher {
            mismunch: self.mismunch,
            width: w,
            at_x,
            at_y,
            k_x,
            k_t,
            k_y,
            grav: random() % 2 == 1,
            fgc,
            xshadow,
            yshadow,
            x: 0,
            // Start with a random y value; this sort of controls the type of
            // deformities seen in the squares.
            y: random_below(256),
            t: 0,
            // Doom each square to be aborted at some random point. When
            // doom == width - 1 the entire square gets drawn.
            doom: if self.mismunch {
                random_below(w)
            } else {
                w - 1
            },
            done: false,
        }
    }

    fn munch(&mut self, i: usize, d: &mut Dpy) {
        if self.munchers[i].done {
            return;
        }
        if !d.mono_p {
            let mut c = self.munchers[i].fgc;
            c.alloc();
            self.gc.set_foreground(c.pixel);
        }

        let m = &mut self.munchers[i];
        for x in 0..m.width {
            m.x = x;
            // The ordinary Munching Squares calculation is
            //   y = ((x ^ ((t + kT) % width)) + kY) % width;
            // Mismunch creates feedback by plugging in y in place of x, and
            // makes a couple of values negative so that some parts of some
            // squares get drawn in the wrong place.
            m.y = if m.mismunch {
                ((-m.y ^ ((-m.t + m.k_t) % m.width)) + m.k_y) % m.width
            } else {
                ((m.x ^ ((m.t + m.k_t) % m.width)) + m.k_y) % m.width
            };

            let draw_x = ((m.x + m.k_x) % m.width) + m.at_x;
            let draw_y = if m.grav {
                m.y + m.at_y
            } else {
                m.at_y + m.width - 1 - m.y
            };

            d.win().draw_point(&self.gc, draw_x, draw_y);
            if m.xshadow != 0 || m.yshadow != 0 {
                d.win()
                    .draw_point(&self.gc, draw_x + m.xshadow, draw_y + m.yshadow);
            }
        }

        m.t += 1;
        if m.t > m.doom {
            m.done = true;
        }
    }

    fn reroll_mismunch(&mut self) {
        if self.mismunch_mode.is_empty() || self.mismunch_mode == "random" {
            self.mismunch = random() & 1 == 1;
        }
    }

    fn restart_all(&mut self, d: &mut Dpy) {
        self.reroll_mismunch();
        let (w, h) = (self.window_width, self.window_height);
        self.munchers = (0..self.simul).map(|_| self.make_muncher(w, h)).collect();
        d.clear_window();
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");
    let mut gc = Gc::new(fg, bg);

    let mismunch_mode = d.res.string("mismunch").to_string();
    let mismunch = if mismunch_mode.is_empty() || mismunch_mode == "random" {
        random() & 1 == 1
    } else {
        d.res.bool("mismunch")
    };

    // Always draw xor on mono.
    if d.mono_p || d.res.bool("xor") {
        gc.set_function(GXFunc::Xor);
    }

    let mut st = Munch {
        gc,
        delay: d.res.int("delay").max(0) as u32,
        simul: d.res.int("simul").max(1) as usize,
        clear: d.res.int("clear").max(0),
        logminwidth: 2,
        logmaxwidth: 2,
        restart: false,
        window_width: d.width(),
        window_height: d.height(),
        draw_n: 0,
        draw_i: 0,
        mismunch,
        mismunch_mode,
        munchers: Vec::new(),
    };
    st.calc_logwidths();
    let (w, h) = (st.window_width, st.window_height);
    st.munchers = (0..st.simul).map(|_| st.make_muncher(w, h)).collect();
    Box::new(st)
}

impl Screenhack for Munch {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for _ in 0..5 {
            self.munch(self.draw_i, d);

            if self.munchers[self.draw_i].done {
                self.draw_n += 1;
                let (w, h) = (self.window_width, self.window_height);
                self.munchers[self.draw_i] = self.make_muncher(w, h);
            }

            self.draw_i += 1;
            if self.draw_i >= self.simul {
                self.draw_i = 0;
                if self.restart || (self.clear > 0 && self.draw_n >= self.clear) {
                    self.restart_all(d);
                    self.draw_n = 0;
                    self.restart = false;
                }
            }
        }
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        if width != self.window_width || height != self.window_height {
            self.window_width = width;
            self.window_height = height;
            self.calc_logwidths();
            self.restart = true;
            self.draw_i = 0;
        }
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.mismunch = random() & 1 == 1;
            self.restart_all(d);
            self.draw_i = 0;
            self.draw_n = 0;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background:       black",
    ".foreground:       white",
    "*fpsSolid:	      true",
    "*delay:            10000",
    "*mismunch:         random",
    "*simul:            5",
    "*clear:            65",
    "*xor:              True",
];

const MISMUNCH_MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Munch or mismunch",
    },
    SelectItem {
        value: "false",
        label: "Munch only",
    },
    SelectItem {
        value: "true",
        label: "Mismunch only",
    },
];

const DRAW_MODES: &[SelectItem] = &[
    SelectItem {
        value: "true",
        label: "XOR",
    },
    SelectItem {
        value: "false",
        label: "Solid",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("clear", "Duration", 1.0, 200.0, 1.0, 0, "65"),
    Opt::slider("simul", "Simultaneous squares", 1.0, 20.0, 1.0, 0, "5"),
    Opt::select("mismunch", "Algorithm", MISMUNCH_MODES, "random"),
    Opt::select("xor", "Drawing mode", DRAW_MODES, "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "munch",
    label: "Munch",
    new: init,
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jackson Wright, Tim Showalter, Jamie Zawinski and Steven Hazel",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=aXNIYpdh8Ug"),
        blurb: "HAKMEM item 146: plot y = x XOR t. Mismunch mode is a creatively \
                broken misimplementation of the same idea.",
    },
};
