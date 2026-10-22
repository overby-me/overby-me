//! Port of `hacks/lcdscrub.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 2008-2015 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Draws repetitive patterns that should undo burned in LCD screens.
//! Concept shamelessly cloned from
//! http://toastycode.com/blog/2008/02/05/lcd-scrub/
//! ```
//!
//! Functional rather than pretty: it walks a set of high-contrast patterns,
//! sliding each one across the screen a pixel at a time so that every pixel
//! spends the same time lit and unlit. The modes it visits are whichever ones
//! are switched on, in a fixed order, a fixed number of sweeps each.
//!
//! Upstream's tenth mode is a PRNG test harness that allocates a 32768-square
//! bitmap, a hundred and twenty-eight megabytes of it. It is off by default and
//! is left out here.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor};

/// The modes, in the order they are visited.
const MODES: usize = 9;
const HORIZ_W: usize = 0;
const HORIZ_B: usize = 1;
const VERT_W: usize = 2;
const VERT_B: usize = 3;
const DIAG_W: usize = 4;
const DIAG_B: usize = 5;
const WHITE: usize = 6;
const BLACK: usize = 7;
const RGB: usize = 8;

/// The resource name that switches each mode on.
const MODE_KEYS: [&str; MODES] = [
    "modeHW", "modeHB", "modeVW", "modeVB", "modeDW", "modeDB", "modeW", "modeB", "modeRGB",
];

/// The primaries the RGB mode walks, and how many frames each is held for.
const RGB_COLORS: [(u16, u16, u16); 8] = [
    (0xFFFF, 0x0000, 0x0000),
    (0x0000, 0xFFFF, 0x0000),
    (0x0000, 0x0000, 0xFFFF),
    (0xFFFF, 0xFFFF, 0x0000),
    (0xFFFF, 0x0000, 0xFFFF),
    (0x0000, 0xFFFF, 0xFFFF),
    (0xFFFF, 0xFFFF, 0xFFFF),
    (0x0000, 0x0000, 0x0000),
];
const RGB_SCALE: usize = 10 * 8; // 8 sec

struct LcdScrub {
    mode: usize,
    enabled: [bool; MODES],
    count: i32,
    fg: Gc,
    bg: Gc,
    bg2: Gc,
    color_tick: usize,
    delay: u32,
    spread: i32,
    cycles: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fgp = d.res.pixel("foreground");
    let bgp = d.res.pixel("background");

    let mut enabled = [false; MODES];
    for (i, key) in MODE_KEYS.iter().enumerate() {
        enabled[i] = d.res.bool(key);
    }
    // Upstream exits when nothing is enabled. There is nowhere to exit to here,
    // so fall back to the full set.
    if !enabled.iter().any(|e| *e) {
        enabled = [true; MODES];
    }

    let mut st = LcdScrub {
        mode: 0,
        enabled,
        count: 0,
        fg: Gc::new(fgp, bgp),
        bg: Gc::new(bgp, fgp),
        bg2: Gc::new(bgp, fgp),
        color_tick: 0,
        delay: d.res.int("delay").max(0) as u32,
        spread: d.res.int("spread").max(1),
        cycles: d.res.int("cycles").max(1),
    };
    st.pick_mode();
    Box::new(st)
}

impl LcdScrub {
    fn pick_mode(&mut self) {
        self.count = 0;
        loop {
            self.mode = (self.mode + 1) % MODES;
            if self.enabled[self.mode] {
                break;
            }
        }
    }
}

impl Screenhack for LcdScrub {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let count = self.count % self.spread;
        let (width, height) = (d.width(), d.height());
        // The even modes draw dark on light and the odd ones the other way,
        // which is what the W and B in their names refer to.
        let light = self.mode & 1 == 1;

        match self.mode {
            HORIZ_W | HORIZ_B => {
                let bg = if light { &self.bg } else { &self.fg };
                d.win().fill_rectangle(bg, 0, 0, width, height);
                let fg = if light { &self.fg } else { &self.bg };
                let mut i = count;
                while i < height {
                    d.win().draw_line(fg, 0, i, width, i);
                    i += self.spread;
                }
            }
            VERT_W | VERT_B => {
                let bg = if light { &self.bg } else { &self.fg };
                d.win().fill_rectangle(bg, 0, 0, width, height);
                let fg = if light { &self.fg } else { &self.bg };
                let mut i = count;
                while i < width {
                    d.win().draw_line(fg, i, 0, i, height);
                    i += self.spread;
                }
            }
            DIAG_W | DIAG_B => {
                let bg = if light { &self.bg } else { &self.fg };
                d.win().fill_rectangle(bg, 0, 0, width, height);
                let fg = if light { &self.fg } else { &self.bg };
                let mut i = count;
                while i < width {
                    d.win().draw_line(fg, i, 0, i + width, width);
                    i += self.spread;
                }
                let mut i = -count;
                while i < height {
                    d.win().draw_line(fg, 0, i, height, i + height);
                    i += self.spread;
                }
            }
            // These three just fill the screen. RGB walks the primaries, so it
            // paints its own colour first.
            RGB => {
                let (r, g, b) = RGB_COLORS[self.color_tick / RGB_SCALE];
                self.bg2.set_foreground(XColor::from_rgb16(r, g, b).pixel);
                self.color_tick = (self.color_tick + 1) % (RGB_COLORS.len() * RGB_SCALE);
                d.win().fill_rectangle(&self.bg2, 0, 0, width, height);
            }
            WHITE => {
                d.win().fill_rectangle(&self.fg, 0, 0, width, height);
            }
            BLACK => {
                d.win().fill_rectangle(&self.bg, 0, 0, width, height);
            }
            _ => {}
        }

        self.count += 1;
        if self.count > self.spread * self.cycles {
            self.pick_mode();
        }

        self.delay
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: True",
    "*delay: 100000",
    "*spread: 8",
    "*cycles: 60",
    "*modeHW: True",
    "*modeHB: True",
    "*modeVW: True",
    "*modeVB: True",
    "*modeDW: True",
    "*modeDB: True",
    "*modeW: True",
    "*modeB: True",
    "*modeRGB: True",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "delay",
        "Frame rate",
        0.0,
        5_000_000.0,
        50000.0,
        0,
        "100000",
    )
    .inverted(),
    Opt::spin("spread", "Line spread", 2.0, 8192.0, "8"),
    Opt::spin("cycles", "Cycles", 1.0, 600.0, "60"),
    Opt::boolean("modeHW", "Horizontal white", "True"),
    Opt::boolean("modeVW", "Vertical white", "True"),
    Opt::boolean("modeDW", "Diagonal white", "True"),
    Opt::boolean("modeW", "Solid white", "True"),
    Opt::boolean("modeRGB", "Primary colors", "True"),
    Opt::boolean("modeHB", "Horizontal black", "True"),
    Opt::boolean("modeVB", "Vertical black", "True"),
    Opt::boolean("modeDB", "Diagonal black", "True"),
    Opt::boolean("modeB", "Solid black", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "lcdscrub",
    label: "LCD Scrub",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2008",
        video: Some("https://www.youtube.com/watch?v=aWtHHBOkO4w"),
        blurb: "Repairs burn-in on LCD monitors. Functional, not pretty.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
