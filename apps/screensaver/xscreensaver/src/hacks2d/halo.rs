//! Port of `hacks/halo.c`.
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
//! Expanding circles XOR-ed together into a bitmap, which is then blitted to
//! the screen in two colours. Where an even number of circles overlap the bit
//! is clear and where an odd number do it is set, which is what produces the
//! interference rings the hack is named for.
//!
//! From the header: "I wanted to lay down new circles with TV:ALU-ADD instead
//! of TV:ALU-XOR, but X doesn't support arithmetic combinations of pixmaps!!
//! What losers."

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{make_smooth_colormap, make_uniform_colormap};
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::{
    About, Dpy, GXFunc, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, SelectItem, StartArgs,
    XColor, random, random_below,
};

/// How the two blit colours are chosen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ColorMode {
    /// Every circle drawn, XOR-ed into one bitmap. The Dr Seuss look.
    Seuss,
    /// Circles drawn straight to the screen, colour ramping as they go.
    Ramp,
}

struct Circle {
    x: i32,
    y: i32,
    radius: i32,
    increment: i32,
    dx: i32,
    dy: i32,
}

struct Halo {
    circles: Vec<Circle>,
    global_count: i32,
    global_inc: i32,
    cmode: ColorMode,
    /// The bitmap the circles are XOR-ed into, and the one they accumulate in.
    pixmap: Pixmap,
    buffer: Option<Pixmap>,
    width: i32,
    height: i32,
    delay: i32,
    draw_gc: Gc,
    erase_gc: Gc,
    copy_gc: Gc,
    merge_gc: Gc,
    anim_p: bool,
    colors: Vec<XColor>,
    fg_index: usize,
    bg_index: usize,
    iterations: i32,
    done_once: bool,
    clear_tick: i32,
    scale: i32,
    mono: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let scale = if d.width() > 2560 || d.height() > 2560 {
        3 // Retina displays
    } else {
        1
    };

    let global_count = d.res.int("count").max(0);
    let global_inc = d.res.int("increment").max(0);
    let mut anim_p = d.res.bool("animate");

    let mut cmode = match d.res.string("colorMode") {
        "seuss" => ColorMode::Seuss,
        "ramp" => ColorMode::Ramp,
        // "random", or anything unrecognised.
        _ => {
            if random() & 3 == 1 {
                ColorMode::Ramp
            } else {
                ColorMode::Seuss
            }
        }
    };
    if d.mono_p {
        cmode = ColorMode::Seuss;
    }
    if cmode == ColorMode::Ramp {
        // This combination doesn't work right.
        anim_p = false;
    }

    let ncolors = d.res.int("colors").max(2) as usize;
    let mut mono = d.mono_p || ncolors <= 2;
    let colors = if mono {
        Vec::new()
    } else if !random().is_multiple_of(if cmode == ColorMode::Seuss { 2 } else { 10 }) {
        make_uniform_colormap(ncolors)
    } else {
        make_smooth_colormap(ncolors)
    };
    if colors.len() <= 2 {
        mono = true;
    }
    let cmode = if mono { ColorMode::Seuss } else { cmode };

    let (fg_index, bg_index, fg_pixel, bg_pixel) = if mono {
        (0, 0, d.res.pixel("foreground"), d.res.pixel("background"))
    } else {
        let fg_index = 0;
        let bg_index = if colors.len() / 4 == fg_index {
            fg_index + 1
        } else {
            colors.len() / 4
        };
        (
            fg_index,
            bg_index,
            colors[fg_index].pixel,
            colors[bg_index % colors.len()].pixel,
        )
    };

    let width = d.width().max(50);
    let height = d.height().max(50);

    let mut st = Halo {
        circles: Vec::new(),
        global_count,
        global_inc,
        cmode,
        pixmap: Pixmap::new_bitmap(width, height),
        buffer: (cmode == ColorMode::Seuss).then(|| Pixmap::new_bitmap(width, height)),
        width,
        height,
        delay: d.res.int("delay"),
        // On a bitmap the "colours" are bits.
        draw_gc: Gc::new(1, 0),
        erase_gc: Gc::new(0, 1),
        copy_gc: Gc::new(fg_pixel, bg_pixel),
        merge_gc: if cmode == ColorMode::Seuss {
            let mut gc = Gc::new(1, 0);
            gc.set_function(GXFunc::Xor);
            gc
        } else {
            Gc::new(fg_pixel, bg_pixel)
        },
        anim_p,
        colors,
        fg_index,
        bg_index,
        iterations: 0,
        done_once: false,
        clear_tick: 0,
        scale,
        mono,
    };

    st.init_circles();
    d.clear_window();
    st.clear_buffer();
    Box::new(st)
}

impl Halo {
    fn init_circles(&mut self) {
        let count = if self.global_count != 0 {
            self.global_count
        } else {
            let n = (self.width.min(self.height) / 50).max(1);
            3 + random_below(n) + random_below(n)
        };

        self.circles = (0..count)
            .map(|_| {
                let increment = if self.global_inc != 0 {
                    self.global_inc
                } else {
                    // Prefer smaller increments to larger ones.
                    let j = 8;
                    let mut inc =
                        (random_below(j) + random_below(j) + random_below(j)) - ((j * 3) / 2);
                    if inc < 0 {
                        inc = -inc + 3;
                    }
                    (inc + 3) * self.scale
                };
                Circle {
                    x: 10 + random_below(self.width - 20),
                    y: 10 + random_below(self.height - 20),
                    increment,
                    radius: random_below(increment),
                    dx: (random_below(3) - 1) * (1 + random_below(5)),
                    dy: (random_below(3) - 1) * (1 + random_below(5)),
                }
            })
            .collect();
    }

    /// `XFillRectangle(buffer, erase_gc, ..)`: clear the accumulated bitmap.
    fn clear_buffer(&mut self) {
        let (w, h) = (self.width, self.height);
        if let Some(b) = self.buffer.as_mut() {
            b.fill_rectangle(&self.erase_gc, 0, 0, w, h);
        }
    }

    /// Is this circle big enough to swallow the whole window?
    ///
    /// Upstream: "Probably there's a simpler way to ask the musical question,
    /// is this square completely enclosed by this circle, but I've forgotten
    /// too much trig to know it."
    fn encloses_window(&self, c: &Circle) -> bool {
        let radius = c.radius as f64;
        let x1 = (-c.x) as f64 / radius;
        let y1 = (-c.y) as f64 / radius;
        let x2 = (self.width - c.x) as f64 / radius;
        let y2 = (self.height - c.y) as f64 / radius;
        let (x1, x2, y1, y2) = (x1 * x1, x2 * x2, y1 * y1, y2 * y2);
        (x1 + y1) < 1.0 && (x2 + y2) < 1.0 && (x1 + y2) < 1.0 && (x2 + y1) < 1.0
    }
}

impl Screenhack for Halo {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut done = false;
        let mut inhibit_sleep = false;

        self.pixmap.clear(0);
        for i in 0..self.circles.len() {
            let (radius, inc) = (self.circles[i].radius, self.circles[i].increment);

            // Never stop on an odd number of iterations.
            if self.iterations & 1 == 0 {
            } else if radius == 0 {
                // Eschew infinity.
            } else if radius < 0 {
                // Stop when the circles are points.
                done = true;
            } else if self.encloses_window(&self.circles[i]) {
                done = true;
            }

            if radius > 0 && (self.cmode == ColorMode::Seuss || self.circles[0].increment < 0) {
                let c = &self.circles[i];
                let (x, y) = (c.x - radius, c.y - radius);
                if self.cmode == ColorMode::Seuss {
                    self.pixmap.fill_arc(
                        &self.draw_gc,
                        x,
                        y,
                        radius * 2,
                        radius * 2,
                        0,
                        FULL_CIRCLE,
                    );
                } else {
                    d.win()
                        .fill_arc(&self.merge_gc, x, y, radius * 2, radius * 2, 0, FULL_CIRCLE);
                }
            }
            self.circles[i].radius += inc;
        }

        if self.anim_p && !self.done_once {
            inhibit_sleep = !done;
        }

        if done {
            if self.anim_p {
                self.done_once = true;
                for c in self.circles.iter_mut() {
                    c.x += c.dx;
                    c.y += c.dy;
                    if c.increment != 0 {
                        c.radius %= c.increment;
                    }
                    if c.x < 0 || c.x >= self.width {
                        c.dx = -c.dx;
                        c.x += 2 * c.dx;
                    }
                    if c.y < 0 || c.y >= self.height {
                        c.dy = -c.dy;
                        c.y += 2 * c.dy;
                    }
                }
            } else if self.circles[0].increment < 0 {
                // Zoomed out and the screen is blank: re-pick the centre
                // points and shift the colours.
                self.init_circles();
                if !self.mono && !self.colors.is_empty() {
                    self.fg_index = (self.fg_index + 1) % self.colors.len();
                    self.bg_index = (self.fg_index + (self.colors.len() / 2)) % self.colors.len();
                    self.copy_gc
                        .set_foreground(self.colors[self.fg_index].pixel);
                    self.copy_gc
                        .set_background(self.colors[self.bg_index].pixel);
                }
            } else if self.clear_tick == 0 && random_below(3) == 0 {
                // Sometimes go out from the inside instead of the outside.
                self.iterations = 0;
                for c in self.circles.iter_mut() {
                    if c.increment != 0 {
                        c.radius %= c.increment;
                    }
                }
                self.clear_tick = (random_below(8) + 4) | 1; // must be odd
            } else {
                for c in self.circles.iter_mut() {
                    c.increment = -c.increment;
                    c.radius += 2 * c.increment;
                }
            }
        }

        let (w, h) = (self.width, self.height);
        if let Some(buffer) = self.buffer.as_mut() {
            buffer.copy_plane(&self.merge_gc, &self.pixmap, 0, 0, w, h, 0, 0);
        } else if self.cmode != ColorMode::Seuss {
            if !self.mono && !self.colors.is_empty() {
                self.fg_index += 1;
                self.bg_index += 1;
                if self.fg_index >= self.colors.len() {
                    self.fg_index = 0;
                }
                if self.bg_index >= self.colors.len() {
                    self.bg_index = 0;
                }
                self.merge_gc
                    .set_foreground(self.colors[self.fg_index].pixel);
            }
            if self.circles[0].increment >= 0 {
                inhibit_sleep = true;
            }
        } else {
            d.win()
                .copy_plane(&self.merge_gc, &self.pixmap, 0, 0, w, h, 0, 0);
        }

        // The buffer is only used in seuss mode or anim mode.
        let show_buffer = if self.anim_p {
            done || (!self.done_once && (self.iterations & 1) != 0)
        } else {
            (self.iterations & 1) != 0
        };
        if show_buffer && self.buffer.is_some() {
            let buffer = self.buffer.take().expect("just checked");
            d.win().copy_plane(&self.copy_gc, &buffer, 0, 0, w, h, 0, 0);
            self.buffer = Some(buffer);
            if self.anim_p
                && done
                && let Some(b) = self.buffer.as_mut()
            {
                b.clear(0);
            }
        }

        if done {
            self.iterations = 0;
        } else {
            self.iterations += 1;
        }

        let mut this_delay = self.delay;
        if self.delay != 0 && !inhibit_sleep && self.cmode == ColorMode::Seuss && self.anim_p {
            this_delay = self.delay / 100;
        }

        if done && self.clear_tick > 0 {
            self.clear_tick -= 1;
            if self.clear_tick == 0 {
                d.clear_window();
                self.clear_buffer();
            }
        }

        if inhibit_sleep {
            this_delay = 0;
        }
        this_delay.max(0) as u32
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.scale = if width > 2560 || height > 2560 { 3 } else { 1 };
        self.width = width.max(50);
        self.height = height.max(50);
        self.pixmap.resize(self.width, self.height);
        if let Some(b) = self.buffer.as_mut() {
            b.resize(self.width, self.height);
        }
    }
}

const DEFAULTS: &[&str] = &[
    ".background:		black",
    ".foreground:		white",
    "*colorMode:		random",
    "*colors:		100",
    "*count:		0",
    "*delay:		100000",
    "*delay2:		20",
    "*increment:		0",
    "*animate:		False",
];

const COLOR_MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random color scheme",
    },
    SelectItem {
        value: "seuss",
        label: "Seuss mode",
    },
    SelectItem {
        value: "ramp",
        label: "Ramp mode",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 300_000.0, 1000.0, 0, "100000").inverted(),
    Opt::slider("count", "Number of circles", 0.0, 20.0, 1.0, 0, "0"),
    Opt::slider("increment", "Increment", 0.0, 20.0, 1.0, 0, "0"),
    Opt::slider("colors", "Number of colors", 2.0, 255.0, 1.0, 0, "100"),
    Opt::select("colorMode", "Colors", COLOR_MODES, "random"),
    Opt::boolean("animate", "Animate", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "halo",
    label: "Halo",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1993",
        video: Some("https://www.youtube.com/watch?v=K7LbfXh3LTc"),
        blurb: "Circles that grow and overlap, XOR-ed into interference rings.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
