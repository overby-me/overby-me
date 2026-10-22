//! Port of `hacks/m6502.c`.
//!
//! ```text
//! Copyright (c) 2007 Jeremy English <jhe@jeremyenglish.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Created: 07-May-2007
//! ```
//!
//! A 6502 with a 32x32 screen, running the demos people wrote for the
//! 6502asm.com assembler in 2006 and 2007. The processor and its assembler are
//! in [`super::asm6502`]; this is the part that turns the 1024 bytes of display
//! memory into a picture, thirty seconds at a time.
//!
//! There is no video hardware between the program and the screen. Each of the
//! 1024 bytes from `$200` is one pixel, its low nibble an index into a
//! sixteen-colour table, and a program draws by storing to it. So the picture
//! is quite literally memory, and a program that walks off the end of its
//! array draws the mistake.
//!
//! The colours are converted to a composite signal the same way
//! [`crate::runtime::analogtv`] converts an image, and the receiver is what
//! actually paints the window. That is why the pixels bleed sideways into each
//! other and why the reds ring: at 32 pixels across the whole screen, a single
//! pixel is wider than the colour subcarrier can resolve, and the set does
//! what a set does.
//!
//! Upstream also takes `-file` and assembles a program of your own. It is
//! inside `#ifdef READ_FILES`, which upstream itself compiles out where there
//! is no filesystem to read, and that is the case here.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::analogtv::{self, AnalogTv, Input, Reception, lcp_to_ntsc};
use crate::runtime::{
    About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, random,
    screenhack_event_helper,
};

use super::asm6502::Machine;

/// The programs, in the order the build glob finds them, which is the order
/// upstream's `demo_files[]` ends up in.
///
/// They are the `.asm` files from `hacks/images/m6502/`, unchanged, and they
/// are assembled from this text at run time exactly as upstream assembles them
/// from the C string literals its build script makes out of the same files.
/// Several are ports of other XScreenSaver hacks, done the hard way.
pub(crate) const DEMOS: [(&str, &str); 33] = [
    ("amiga", include_str!("../../images/m6502/amiga.asm")),
    ("breakout", include_str!("../../images/m6502/breakout.asm")),
    ("byterun", include_str!("../../images/m6502/byterun.asm")),
    (
        "cellular-30",
        include_str!("../../images/m6502/cellular-30.asm"),
    ),
    (
        "cellular-600",
        include_str!("../../images/m6502/cellular-600.asm"),
    ),
    ("colors", include_str!("../../images/m6502/colors.asm")),
    (
        "crunch6502",
        include_str!("../../images/m6502/crunch6502.asm"),
    ),
    (
        "demoscene",
        include_str!("../../images/m6502/demoscene.asm"),
    ),
    ("disco", include_str!("../../images/m6502/disco.asm")),
    ("dmsc", include_str!("../../images/m6502/dmsc.asm")),
    (
        "dragon-fractal",
        include_str!("../../images/m6502/dragon-fractal.asm"),
    ),
    (
        "fullscreenlogo",
        include_str!("../../images/m6502/fullscreenlogo.asm"),
    ),
    (
        "greynetic",
        include_str!("../../images/m6502/greynetic.asm"),
    ),
    ("keftal", include_str!("../../images/m6502/keftal.asm")),
    ("life", include_str!("../../images/m6502/life.asm")),
    ("lines", include_str!("../../images/m6502/lines.asm")),
    ("matrix", include_str!("../../images/m6502/matrix.asm")),
    ("noise", include_str!("../../images/m6502/noise.asm")),
    (
        "random-walk",
        include_str!("../../images/m6502/random-walk.asm"),
    ),
    ("random", include_str!("../../images/m6502/random.asm")),
    ("random2", include_str!("../../images/m6502/random2.asm")),
    (
        "rorschach",
        include_str!("../../images/m6502/rorschach.asm"),
    ),
    ("santa", include_str!("../../images/m6502/santa.asm")),
    (
        "selfmodify",
        include_str!("../../images/m6502/selfmodify.asm"),
    ),
    ("sflake", include_str!("../../images/m6502/sflake.asm")),
    (
        "sierpinski",
        include_str!("../../images/m6502/sierpinski.asm"),
    ),
    (
        "sierpinsky",
        include_str!("../../images/m6502/sierpinsky.asm"),
    ),
    (
        "softsprite",
        include_str!("../../images/m6502/softsprite.asm"),
    ),
    ("spacer", include_str!("../../images/m6502/spacer.asm")),
    (
        "starfield2d",
        include_str!("../../images/m6502/starfield2d.asm"),
    ),
    ("texture", include_str!("../../images/m6502/texture.asm")),
    ("wave6502", include_str!("../../images/m6502/wave6502.asm")),
    (
        "zookeeper",
        include_str!("../../images/m6502/zookeeper.asm"),
    ),
];

/// We want to paint on a 32 by 32 grid of pixels. We will needed to divided
/// the screen up into chuncks.
const SCREEN_W: i32 = analogtv::VIS_LEN as i32;
const SCREEN_H: i32 = analogtv::VISLINES as i32;

/// The palette the 6502asm.com page used, and the reason these demos are all
/// the same handful of colours.
const CLR_TBL: [[f64; 3]; 16] = [
    [0.0, 0.0, 0.0],
    [255.0, 255.0, 255.0],
    [136.0, 0.0, 0.0],
    [170.0, 255.0, 238.0],
    [204.0, 68.0, 204.0],
    [0.0, 204.0, 85.0],
    [0.0, 0.0, 170.0],
    [238.0, 238.0, 119.0],
    [221.0, 136.0, 85.0],
    [102.0, 68.0, 0.0],
    [255.0, 119.0, 119.0],
    [51.0, 51.0, 51.0],
    [119.0, 119.0, 119.0],
    [170.0, 255.0, 102.0],
    [0.0, 136.0, 255.0],
    [187.0, 187.0, 187.0],
];

struct M6502 {
    machine: Machine,

    tv: AnalogTv,
    inp: Input,
    reception: Reception,
    /// Pixel width.
    pixw: i32,
    /// Pixel height.
    pixh: i32,
    /// Top boarder.
    topb: i32,
    /// How long to wait before changing the demo.
    dt: f64,
    /// The program to run.
    which: usize,
    start_time: f64,
    reset_p: bool,
    last_frame: f64,
    last_delay: f64,
    ips: u32,
}

impl M6502 {
    /// Pick a program that is not the one just shown, and start it.
    fn start_rand_bin_prog(&mut self) {
        let mut n = self.which;
        while n == self.which {
            n = (random() as usize) % DEMOS.len();
        }
        self.which = n;
        self.machine.start_eval_string(DEMOS[self.which].1);
    }

    /// One byte of display memory, drawn as a rectangle of composite video.
    fn paint_pixel(&mut self, x: i32, y: i32, idx: usize) {
        /* RGB conversion taken from analogtv draw xpm */
        let [r, g, b] = CLR_TBL[idx];
        let rawy = ((5.0 * r + 11.0 * g + 2.0 * b) / 64.0) as i32;
        let rawi = ((10.0 * r - 4.0 * g - 5.0 * b) / 64.0) as i32;
        let rawq = ((3.0 * r - 8.0 * g + 5.0 * b) / 64.0) as i32;

        let mut ntsc = [rawy + rawq, rawy - rawi, rawy - rawq, rawy + rawi];
        for n in ntsc.iter_mut() {
            *n = (*n).clamp(analogtv::BLACK_LEVEL, analogtv::WHITE_LEVEL);
        }

        let x = x * self.pixw;
        let y = y * self.pixh + self.topb;
        let (left, top) = (analogtv::VIS_START as i32 + x, analogtv::TOP as i32 + y);
        self.inp
            .draw_solid(left, left + self.pixw, top, top + self.pixh, ntsc);
    }
}

impl Screenhack for M6502 {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let last_delay = if self.last_delay >= 0.0 {
            self.last_delay
        } else {
            0.0
        };
        let mut insno = f64::from(self.ips) * ((1.0 / 29.97) + last_delay - self.last_delay);

        if self.reset_p {
            /* do something more interesting here XXX */
            self.reset_p = false;
            self.start_time = self.last_frame + last_delay;
            // Upstream clears its own copy of the pixels here. Ours are the
            // machine's, and starting a program resets the machine, which
            // clears them a moment later.
            self.start_rand_bin_prog();
        }

        insno = insno.clamp(10.0, 100_000.0); /* Real 6502 went no faster than 3 MHz. */
        self.machine.next_eval(insno as i32);

        for x in 0..32 {
            for y in 0..32 {
                let idx = usize::from(self.machine.pixels[x][y]);
                self.paint_pixel(x as i32, y as i32, idx);
            }
        }

        self.reception.update();
        {
            let (tv, rec, inp) = (&mut self.tv, &self.reception, &self.inp);
            tv.draw(d.win(), 0.04, &[(rec, inp)]);
        }

        let now = d.time;
        self.last_delay = (1.0 / 29.97) + self.last_frame + last_delay - now;
        self.last_frame = now;

        if now - self.start_time > self.dt {
            self.reset_p = true;
        }

        if self.last_delay >= 0.0 {
            (self.last_delay * 1e6) as u32
        } else {
            0
        }
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.tv.configure(width, height);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.reset_p = true;
            return true;
        }
        false
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut tv = AnalogTv::new(d.width(), d.height());
    tv.set_defaults(
        d.res.float("TVColor") as f32,
        d.res.float("TVTint") as f32,
        d.res.float("TVBrightness") as f32,
        d.res.float("TVContrast") as f32,
    );

    let mut inp = Input::new();
    inp.setup_sync(true, false);

    let mut st = M6502 {
        machine: Machine::new(),
        tv,
        inp,
        reception: Reception {
            level: 2.0,
            ..Reception::default()
        },
        pixw: SCREEN_W / 32,
        pixh: SCREEN_H / 32,
        topb: (SCREEN_H % 32) / 2,
        dt: d.res.float("displaytime"),
        which: (random() as usize) % DEMOS.len(),
        start_time: 0.0,
        reset_p: true,
        last_frame: d.time,
        last_delay: 0.0,
        ips: d.res.int("ips").max(0) as u32,
    };

    let field_ntsc = lcp_to_ntsc(f64::from(analogtv::BLACK_LEVEL), 0.0, 0.0);
    st.inp.draw_solid(
        analogtv::VIS_START as i32,
        analogtv::VIS_END as i32,
        analogtv::TOP as i32,
        analogtv::BOT as i32,
        field_ntsc,
    );

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    ".background:      black",
    ".foreground:      white",
    "*displaytime:     30.0",  /* demoscene: 24s, dmsc: 48s, sierpinsky: 26s
                               sflake, two runs: 35s
                               */
    "*ips:             15000", /* Actual MOS 6502 ran at least 1 MHz. */
    "*TVColor:         70",
    "*TVTint:          5",
    "*TVBrightness:    2",
    "*TVContrast:    150",
    "*fpsSolid:	     True",
    "*lowrez:	     True",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "displaytime",
        "Display time for each program",
        5.0,
        120.0,
        1.0,
        0,
        "30.0",
    ),
    Opt::slider(
        "ips",
        "Instructions per second",
        500.0,
        120_000.0,
        500.0,
        0,
        "15000",
    ),
    Opt::slider("TVColor", "Color Knob", 0.0, 400.0, 5.0, 0, "70"),
    Opt::slider("TVTint", "Tint Knob", 0.0, 360.0, 5.0, 0, "5"),
    Opt::slider("TVBrightness", "Brightness Knob", -75.0, 100.0, 1.0, 0, "2"),
    Opt::slider("TVContrast", "Contrast Knob", 0.0, 500.0, 10.0, 0, "150"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "m6502",
    label: "m6502",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Stian Soreng and Jeremy English",
        year: "2007",
        video: Some("https://www.youtube.com/watch?v=KlDw0nYwUe4"),
        blurb: "A 6502 microprocessor running example programs on a 32x32 screen.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
