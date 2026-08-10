//! Port of `hacks/memscroller.c`.
//!
//! ```text
//! xscreensaver, Copyright © 2002-2023 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Memscroller -- scrolls a dump of its own RAM across the screen.
//! ```
//!
//! Three bands of bytes scrolling right to left, each at its own resolution and
//! its own speed, with whatever four bytes went past last written in hex above
//! them. Every byte read becomes a colour directly: three bytes make a pixel in
//! colour mode, one makes a shade of green in mono.
//!
//! What it dumps is the part that cannot come across. Upstream walks its own
//! address space from an early allocation up to `sbrk`, which is why the
//! picture has texture: text sections stripe, code speckles, and long runs of
//! zeros get skipped past because a screen of black is not worth watching. A
//! page in a browser cannot read its own heap, so the memory it dumps here is
//! the largest block of the program's own data a safe program can point at,
//! which is the compiled-in glyph table. It wraps at the end the way upstream
//! wraps at the top of the heap, and upstream clamps itself to a window of the
//! same order whenever the addresses it gets back look implausible.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::rgb;
use crate::runtime::font::{self, Font};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XRectangle,
    random,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Seed {
    Ram,
    Random,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Draw {
    Color,
    Mono,
}

struct Scroller {
    rect: XRectangle,
    /// How many screen pixels one byte becomes, vertically and horizontally.
    rez: i32,
    /// How many pixels this band moves per frame.
    speed: i32,
    scroll_tick: i32,
    /// The last four bytes read, which is what the hex readout shows.
    value: u32,
    /// Where in the dump this band has got to.
    data: usize,
    count_zero: i32,
    /// The column of pixels about to be pushed in at the right.
    column: Vec<Pixel>,
}

struct MemScroller {
    draw_gc: Gc,
    erase_gc: Gc,
    /// Six sizes, largest first; the readout uses the first that fits.
    fonts: Vec<Font>,
    border: i32,
    seed_mode: Seed,
    draw_mode: Draw,
    scrollers: Vec<Scroller>,
    /// The block of the program's own memory being dumped.
    mem: Vec<u8>,
    width: i32,
    height: i32,
    delay: u32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.width(), d.height());

    let mut border = d.res.int("borderSize");
    if width > 2560 || height > 2560 {
        border *= 2; /* Retina displays */
    }

    let fonts = (1..=6)
        .map(|i| Font::load(d.res.string(&format!("font{i}"))))
        .collect();

    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");
    let mut draw_gc = Gc::new(fg, bg);
    draw_gc.set_line_width(border);

    let draw_mode = if d.res.string("drawMode").eq_ignore_ascii_case("mono") {
        Draw::Mono
    } else {
        Draw::Color
    };
    let filename = d.res.string("filename").to_string();
    let seed_mode =
        if filename.eq_ignore_ascii_case("(rand)") || filename.eq_ignore_ascii_case("(random)") {
            Seed::Random
        } else {
            // Upstream also reads a named file here. There is no filesystem, and
            // the panel offers only the two modes its own XML does.
            Seed::Ram
        };

    let mut st = MemScroller {
        draw_gc,
        erase_gc: Gc::new(bg, bg),
        fonts,
        border,
        seed_mode,
        draw_mode,
        scrollers: (0..3)
            .map(|i| {
                let mut speed = i + 1;
                if width > 2560 || height > 2560 {
                    speed = (f64::from(speed) * 2.5) as i32;
                }
                Scroller {
                    rect: XRectangle::default(),
                    rez: 1,
                    speed,
                    scroll_tick: 0,
                    value: 0,
                    data: 0,
                    count_zero: 0,
                    column: Vec::new(),
                }
            })
            .collect(),
        mem: font::program_bytes(),
        width,
        height,
        delay: d.res.int("delay").max(0) as u32,
    };
    st.lay_out(d);
    Box::new(st)
}

impl MemScroller {
    /// `reshape_memscroller`: three bands down the middle of the screen, each
    /// coarser and shorter than the one above it, and each with its frame drawn
    /// around it once.
    fn lay_out(&mut self, d: &mut Dpy) {
        let (w, h) = (self.width, self.height);
        for i in 0..self.scrollers.len() {
            if i == 0 {
                let mut rez = 6; /* #### */
                if w > 2560 || h > 2560 {
                    rez = (f64::from(rez) * 2.5) as i32;
                }
                let width = ((f64::from(w) * 0.8) as i32 / rez) * rez;
                let height = ((f64::from(h) * 0.3) as i32 / rez) * rez;
                let sc = &mut self.scrollers[0];
                sc.rez = rez;
                sc.rect = XRectangle {
                    x: (w - width) / 2,
                    y: (h - height) / 2,
                    width,
                    height,
                };
            } else {
                let prev = self.scrollers[i - 1].rect;
                let prev_rez = self.scrollers[i - 1].rez;
                let rez = (f64::from(prev_rez) * 1.8) as i32;
                let height = ((f64::from(h) * 0.1) as i32 / rez.max(1)) * rez;
                let sc = &mut self.scrollers[i];
                sc.rez = rez;
                sc.rect = XRectangle {
                    x: prev.x,
                    y: prev.y + prev.height + self.border + (self.border + 2) * 7,
                    width: prev.width,
                    height,
                };
            }

            let (sc, border) = (&self.scrollers[i], self.border);
            d.win().draw_rectangle(
                &self.draw_gc,
                sc.rect.x - border * 2,
                sc.rect.y - border * 2,
                sc.rect.width + border * 4,
                sc.rect.height + border * 4,
            );
        }
    }

    /// The next byte of the dump, wrapping at the end.
    fn next_byte(&mut self, i: usize) -> u8 {
        if self.mem.is_empty() {
            return 0;
        }
        let sc = &mut self.scrollers[i];
        if sc.data >= self.mem.len() {
            sc.data = 0;
        }
        let b = self.mem[sc.data];
        sc.data += 1;
        b
    }

    /// One pixel's worth of source: three bytes in colour, one in mono, rolled
    /// into the running value that the readout displays.
    fn more_bits(&mut self, i: usize) -> Pixel {
        let mut vv = self.scrollers[i].value;

        let (r, g, b) = match self.seed_mode {
            Seed::Ram => loop {
                let (r, g, b) = match self.draw_mode {
                    Draw::Color => {
                        let (r, g, b) = (self.next_byte(i), self.next_byte(i), self.next_byte(i));
                        vv = (vv << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
                        (r, g, b)
                    }
                    Draw::Mono => {
                        let g = self.next_byte(i);
                        vv = (vv << 8) | u32::from(g);
                        (0, g, 0)
                    }
                };

                // Avoid having many seconds of blackness: give up on a run of
                // zeros once it has gone on long enough to be boring.
                let sc = &mut self.scrollers[i];
                if vv == 0 {
                    sc.count_zero += 1;
                } else {
                    sc.count_zero = 0;
                }
                let limit = 1024 * if self.draw_mode == Draw::Color { 24 } else { 8 };
                if self.scrollers[i].count_zero <= limit {
                    break (r, g, b);
                }
            },
            Seed::Random => {
                vv = random();
                match self.draw_mode {
                    Draw::Color => (
                        ((vv >> 16) & 0xFF) as u8,
                        ((vv >> 8) & 0xFF) as u8,
                        (vv & 0xFF) as u8,
                    ),
                    Draw::Mono => (0, (vv & 0xFF) as u8, 0),
                }
            }
        };

        self.scrollers[i].value = vv;
        rgb(r, g, b)
    }

    /// The running value in hex, in the largest of the six sizes that fits
    /// between the top of the screen and the first band.
    fn draw_string(&mut self, d: &mut Dpy) {
        let bot = self.scrollers[0].rect.y;
        for font in self.fonts.clone() {
            let buf = format!("{:08X}", self.scrollers[0].value);
            let w = font.text_width(&buf);
            let h = font.ascent() + font.descent() + 1;
            let x = (self.width - w) / 2;
            let y = (bot - h) / 2;

            if y + h + 10 <= bot && x > -10 {
                d.win()
                    .fill_rectangle(&self.erase_gc, x - w - 1, y - 1, w * 3 + 2, h + 2);
                d.win()
                    .draw_string(&self.draw_gc, &font, x, y + font.ascent(), &buf);
                break;
            }
        }
    }
}

impl Screenhack for MemScroller {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.draw_string(d);

        for i in 0..self.scrollers.len() {
            let (rect, rez, speed) = {
                let sc = &self.scrollers[i];
                (sc.rect, sc.rez, sc.speed)
            };

            // Everything already drawn shuffles left, and the new bytes come
            // in as a column at the right edge.
            d.win().copy_area_self(
                &self.draw_gc,
                rect.x + speed,
                rect.y,
                rect.width - speed,
                rect.height,
                rect.x,
                rect.y,
            );

            if self.scrollers[i].scroll_tick == 0 {
                let top = rect.height / rez.max(1);
                let mut column = Vec::with_capacity((top * rez) as usize);
                for _ in 0..top {
                    let v = self.more_bits(i);
                    for _ in 0..rez {
                        column.push(v);
                    }
                }
                self.scrollers[i].column = column;
            }

            let sc = &mut self.scrollers[i];
            sc.scroll_tick += 1;
            if sc.scroll_tick * speed >= rez {
                sc.scroll_tick = 0;
            }

            for j in 0..speed {
                let x = rect.x + rect.width - 1 - j;
                for (y, px) in self.scrollers[i].column.iter().enumerate() {
                    d.win().put_pixel(x, rect.y + y as i32, *px);
                }
            }
        }

        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        d.clear_window();
        self.lay_out(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background:		   black",
    "*drawMode:		   color",
    "*fpsSolid:		   true",
    "*fpsTop:		   true",
    "*filename:		   (RAM)",
    ".textColor:		   #00FF00",
    ".foreground:		   #00FF00",
    "*borderSize:		   2",
    ".font1: OCR A 192, OCR A Std 192,Lucida Console 192,Monaco 192,Courier 192",
    ".font2: OCR A 144, OCR A Std 144,Lucida Console 144,Monaco 144,Courier 144",
    ".font3: OCR A 128, OCR A Std 128,Lucida Console 128,Monaco 128,Courier 128",
    ".font4: OCR A 96,  OCR A Std 96, Lucida Console 96, Monaco 96, Courier 96",
    ".font5: OCR A 48,  OCR A Std 48, Lucida Console 48, Monaco 48, Courier 48",
    ".font6: OCR A 24,  OCR A Std 24, Lucida Console 24, Monaco 24, Courier 24",
    "*delay:		   10000",
    "*offset:		   0",
];

const SEEDS: &[SelectItem] = &[
    SelectItem {
        value: "(RAM)",
        label: "Dump memory",
    },
    SelectItem {
        value: "(RANDOM)",
        label: "Draw random numbers",
    },
];

const DRAWS: &[SelectItem] = &[
    SelectItem {
        value: "color",
        label: "Draw in RGB",
    },
    SelectItem {
        value: "mono",
        label: "Draw green",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    // Upstream spells the choice of source as a filename, so that is the key
    // the two options set.
    Opt::select("filename", "Source", SEEDS, "(RAM)"),
    Opt::select("drawMode", "Colour", DRAWS, "color"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "memscroller",
    label: "Mem Scroller",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=DQJRNlTKCdA"),
        blurb: "Scrolls a dump of its own memory in three windows at three different rates.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
