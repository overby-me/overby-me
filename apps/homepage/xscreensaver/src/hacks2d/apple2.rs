//! Port of `hacks/apple2.c` and `hacks/apple2-main.c`.
//!
//! ```text
//! xscreensaver, Copyright © 1998-2025 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Apple ][ CRT simulator, by Trevor Blackwell <tlb@tlb.org>
//! with additional work by Jamie Zawinski <jwz@jwz.org>
//! Pty and vt100 emulation by Fredrik Tolf <fredrik@dolda2000.com>
//! ```
//!
//! An Apple ][+ wired to a colour television, and neither half knows what the
//! other is. The machine fills two blocks of memory, one 40x24 of characters
//! and one 192x40 of bytes, and the video circuitry shifts those bits out at
//! four times the colour subcarrier. Nothing here draws a coloured pixel: the
//! only colours on the screen are the ones a television invents when it tries
//! to demodulate a bit pattern that was never meant to be chroma. That is why
//! hires has six colours, why they depend on whether a pixel is odd or even,
//! and why the high bit of each byte shifts the whole group by half a dot and
//! turns green into orange.
//!
//! Three programs run on it, one picked at random each time it reboots:
//!
//! * BASIC, where a simulated user types a listing in, sometimes mistypes it,
//!   backs up over the mistake, and runs it.
//! * Text, which is a VT100 ([`crate::runtime::tty`]) whose grid is copied into
//!   the character memory, upper-cased and inverted, since the machine has no
//!   lower case.
//! * Slideshow, which takes a picture, dithers it to the six colours with
//!   Floyd-Steinberg, and BLOADs it into the hires screen in the interleaved
//!   order the memory map imposes, so it appears in eight passes rather than
//!   from the top down.
//!
//! The machine itself is not private to this saver: `bsod` borrows it for
//! three of its crashes, and drives it with a controller of its own, the way
//! upstream's `apple2_start` takes one.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::analogtv::{self, AnalogTv, Input, Reception};
use crate::runtime::color::unrgb;
use crate::runtime::tty::{TTY_BLINK, TTY_BOLD, TTY_INVERSE, TTY_SYMBOLS, Tty};
use crate::runtime::{
    About, Dpy, Fb, ImageLoad, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent,
    color, frand, png, random,
};
use std::rc::Rc;

const SCREEN_COLS: usize = 40;
const SCREEN_ROWS: usize = 24;

/// Graphics is showing on the bottom four rows too, rather than text.
pub(crate) const A2_GR_FULL: u32 = 1;
/// 40x48 blocks of sixteen colours, sharing memory with the text screen.
pub(crate) const A2_GR_LORES: u32 = 2;
/// 280x192, one bit per pixel, whatever colour the television makes of it.
pub(crate) const A2_GR_HIRES: u32 = 4;

/// The controller has finished; wind the machine up.
pub(crate) const A2CONTROLLER_DONE: i32 = -1;
/// Last call: let the controller put its things away.
pub(crate) const A2CONTROLLER_FREE: i32 = -2;

/// The character generator: 64 glyphs of 7x8, side by side in one row.
const FONT_W: i32 = 64 * 7;
const FONT_H: i32 = 8;

/// The 6502's video memory, as far as anything here cares about it.
pub(crate) struct A2State {
    hireslines: Box<[[u8; 40]; 192]>,
    textlines: [[u8; 40]; SCREEN_ROWS],
    pub(crate) gr_mode: u32,
    cursx: i32,
    cursy: i32,
    blink: bool,
}

impl A2State {
    pub(crate) fn new() -> A2State {
        A2State {
            hireslines: Box::new([[0; 40]; 192]),
            textlines: [[0; 40]; SCREEN_ROWS],
            gr_mode: 0,
            cursx: 0,
            cursy: 0,
            blink: false,
        }
    }

    /// The byte under the cursor.
    ///
    /// A backspace at the left margin leaves `cursx` at -1, and upstream's
    /// screen is one contiguous array, so the byte before a row is the last
    /// byte of the row above: the cursor lands at the end of the previous line,
    /// which is where it should be. Indexing flat keeps that and clamps only at
    /// the very start of the screen, where upstream leaves the array behind.
    fn curs(&mut self) -> &mut u8 {
        let last = (SCREEN_ROWS * SCREEN_COLS) as i32 - 1;
        let i = (self.cursy * SCREEN_COLS as i32 + self.cursx).clamp(0, last) as usize;
        &mut self.textlines[i / SCREEN_COLS][i % SCREEN_COLS]
    }

    fn scroll(&mut self) {
        *self.curs() |= 0xc0; /* turn off cursor */
        for i in 0..SCREEN_ROWS - 1 {
            self.textlines[i] = self.textlines[i + 1];
        }
        self.textlines[SCREEN_ROWS - 1] = [0xe0; 40];
    }

    fn printc_1(&mut self, c: u8, scroll_p: bool) {
        *self.curs() |= 0xc0; /* turn off blink */

        if c == b'\n' {
            /* ^J == NL */
            if self.cursy == 23 {
                if scroll_p {
                    self.scroll();
                }
            } else {
                self.cursy += 1;
            }
            self.cursx = 0;
        } else if c == 0o14 {
            /* ^L == CLS, Home */
            self.cls();
            self.goto(0, 0);
        } else if c == b'\t' {
            /* ^I == tab */
            let (y, x) = (self.cursy, self.cursx);
            self.goto(y, (x + 8) & !7);
        } else if c == 0o10 {
            /* ^H == backspace */
            *self.curs() = 0xe0;
            let (y, x) = (self.cursy, self.cursx);
            self.goto(y, x - 1);
        } else if c == b'\r' {
            /* ^M == CR */
            self.cursx = 0;
        } else {
            *self.curs() = c ^ 0xc0;
            self.cursx += 1;
            if self.cursx == 40 {
                if self.cursy == 23 {
                    if scroll_p {
                        self.scroll();
                    }
                } else {
                    self.cursy += 1;
                }
                self.cursx = 0;
            }
        }

        *self.curs() &= 0x7f; /* turn on blink */
    }

    pub(crate) fn printc(&mut self, c: u8) {
        self.printc_1(c, true);
    }

    pub(crate) fn printc_noscroll(&mut self, c: u8) {
        self.printc_1(c, false);
    }

    pub(crate) fn prints(&mut self, s: &str) {
        for c in s.bytes() {
            self.printc(c);
        }
    }

    pub(crate) fn goto(&mut self, r: i32, c: i32) {
        let r = r.min(23);
        let c = c.min(39);
        *self.curs() |= 0xc0; /* turn off blink */
        self.cursy = r;
        self.cursx = c;
        *self.curs() &= 0x7f; /* turn on blink */
    }

    pub(crate) fn cls(&mut self) {
        self.textlines = [[0xe0; 40]; SCREEN_ROWS];
    }

    fn clear_gr(&mut self) {
        self.textlines = [[0x00; 40]; SCREEN_ROWS];
    }

    fn clear_hgr(&mut self) {
        *self.hireslines = [[0; 40]; 192];
    }

    /// Write a byte to an address, as a program would, and let the video
    /// circuitry make of it what it will. The row arithmetic is the machine's
    /// famously scrambled screen memory map.
    ///
    /// Nothing in this saver pokes anything; `bsod` does, when it borrows the
    /// machine to crash it.
    pub(crate) fn poke(&mut self, addr: usize, val: u8) {
        if (0x400..0x800).contains(&addr) {
            /* text memory */
            let row = ((addr & 0x380) / 0x80) + ((addr & 0x7f) / 0x28) * 8;
            let col = (addr & 0x7f) % 0x28;
            if row < 24 && col < 40 {
                self.textlines[row][col] = val;
            }
        } else if (0x2000..0x4000).contains(&addr) {
            let row = ((addr & 0x1c00) / 0x400)
                + ((addr & 0x0380) / 0x80) * 8
                + ((addr & 0x0078) / 0x28) * 64;
            let col = (addr & 0x07f) % 0x28;
            if row < 192 && col < 40 {
                self.hireslines[row][col] = val;
            }
        }
    }

    /// Simulate plausible initial memory contents for running a program.
    pub(crate) fn init_memory_active(&mut self, font: &A2Font) {
        let mut addr = 0;
        while addr < 0x4000 {
            match random() % 4 {
                0 | 1 => {
                    let n = random() % 500;
                    for _ in 0..n {
                        if addr >= 0x4000 {
                            break;
                        }
                        let lo = if random().is_multiple_of(6) {
                            0
                        } else {
                            random() % 16
                        };
                        let hi = if random().is_multiple_of(5) {
                            0
                        } else {
                            random() % 16
                        };
                        self.poke(addr, (lo | (hi << 4)) as u8);
                        addr += 1;
                    }
                }

                2 => {
                    /* Simulate shapes stored in memory. We use the font since we
                    have it. Unreadable, since rows of each character are stored
                    in consecutive bytes. It was typical to store each of the 7
                    possible shifts of bitmaps, for fastest blitting to the
                    screen. */
                    let mut x = (random() % FONT_W as u32) as i32;
                    for _ in 0..100 {
                        for y in 0..8 {
                            let mut c = 0u8;
                            for j in 0..8 {
                                c |= u8::from(font.pixel((x + j) % FONT_W, y)) << j;
                            }
                            self.poke(addr, c);
                            addr += 1;
                        }
                        x = (x + 1) % FONT_W;
                    }
                }

                _ => {
                    if addr > 0x2000 {
                        let n = random() % 200;
                        for _ in 0..n {
                            if addr >= 0x4000 {
                                break;
                            }
                            self.poke(addr, 0);
                            addr += 1;
                        }
                    }
                }
            }
        }
    }

    /// `HPLOT`. Sets two adjacent bits, because one bit is half a colour.
    fn hplot(&mut self, hcolor: i32, x: i32, y: i32) {
        /* capture bit 2 into bit 7 */
        let highbit = (((hcolor << 5) & 0x80) ^ 0x80) as u8;

        if !(0..192).contains(&y) || !(0..280).contains(&x) {
            return;
        }

        let mut x = x;
        let mut run = 0;
        while run < 2 && x < 280 {
            let vidbyte = &mut self.hireslines[y as usize][(x / 7) as usize];
            let whichbit = 1u8 << (x % 7);

            *vidbyte = (*vidbyte & 0x7f) | highbit;

            /* use either bit 0 or 1 of hcolor for odd or even pixels */
            let masked_bit = (hcolor >> (1 - (x & 1))) & 1;

            /* Set whichbit to 1 or 0 depending on color */
            *vidbyte = (*vidbyte & !whichbit) | if masked_bit != 0 { whichbit } else { 0 };

            x += 1;
            run += 1;
        }
    }

    /// `HPLOT TO`: Bresenham's line drawing algorithm.
    fn hline(&mut self, hcolor: i32, x1: i32, y1: i32, x2: i32, y2: i32) {
        let (mut dx, incx) = if x2 >= x1 {
            (x2 - x1, 1)
        } else {
            (x1 - x2, -1)
        };
        let (mut dy, incy) = if y2 >= y1 {
            (y2 - y1, 1)
        } else {
            (y1 - y2, -1)
        };

        let (mut x, mut y) = (x1, y1);

        if dx >= dy {
            dy *= 2;
            let mut balance = dy - dx;
            dx *= 2;
            while x != x2 {
                self.hplot(hcolor, x, y);
                if balance >= 0 {
                    y += incy;
                    balance -= dx;
                }
                balance += dy;
                x += incx;
            }
            self.hplot(hcolor, x, y);
        } else {
            dx *= 2;
            let mut balance = dx - dy;
            dy *= 2;
            while y != y2 {
                self.hplot(hcolor, x, y);
                if balance >= 0 {
                    x += incx;
                    balance -= dy;
                }
                balance += dx;
                y += incy;
            }
            self.hplot(hcolor, x, y);
        }
    }

    /// `PLOT`: a lores block, which is half a character cell.
    fn plot(&mut self, color: u8, x: i32, y: i32) {
        if !(0..40).contains(&x) || !(0..48).contains(&y) {
            return;
        }
        let textrow = (y / 2) as usize;
        let byte = self.textlines[textrow][x as usize];
        self.textlines[textrow][x as usize] = if y & 1 != 0 {
            (byte & 0xf0) | (color & 0x0f)
        } else {
            (byte & 0x0f) | ((color & 0x0f) << 4)
        };
    }

    /// When loading images, it would normally just load the big binary dump
    /// into screen memory while you watched. Because of the way screen memory
    /// was laid out, it wouldn't load from the top down, but in a funny
    /// interleaved way. Call this with `lineno` increasing from 0 through 191
    /// over a period of a few seconds.
    fn display_image_loading(&mut self, image: &[u8], lineno: usize) {
        let row = ((lineno / 24) % 8) + ((lineno / 3) % 8) * 8 + (lineno % 3) * 64;
        if image.len() >= (row + 1) * 40 {
            self.hireslines[row].copy_from_slice(&image[row * 40..row * 40 + 40]);
        }
    }
}

/// The character generator ROM, one bool per pixel.
pub(crate) struct A2Font {
    ink: Vec<bool>,
}

impl A2Font {
    /// jwz's dump of the machine's font, since MacOS has no "6x10" to tweak
    /// into one. Ink is where the sheet is both opaque and black.
    pub(crate) fn load() -> A2Font {
        let mut ink = vec![false; (FONT_W * FONT_H) as usize];
        if let Some((im, mask)) = png::decode(crate::images::APPLE2FONT)
            && im.width() == FONT_W
            && im.height() == FONT_H
        {
            for y in 0..FONT_H {
                for x in 0..FONT_W {
                    let opaque = mask.as_ref().is_none_or(|m| m.get_pixel(x, y) != 0);
                    let black = im.get_pixel(x, y) & color::RGB_MASK == 0;
                    ink[(y * FONT_W + x) as usize] = opaque && black;
                }
            }
        }
        A2Font { ink }
    }

    fn pixel(&self, x: i32, y: i32) -> bool {
        if (0..FONT_W).contains(&x) && (0..FONT_H).contains(&y) {
            self.ink[(y * FONT_W + x) as usize]
        } else {
            false
        }
    }
}

/// Which of the three programs is running.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Slideshow,
    Terminal,
    Basic,
}

/// What is driving the machine.
///
/// Upstream passes `apple2_start` a function pointer and a `void *` for its
/// state, and calls it whenever the machine is ready to be told what to do
/// next. That is this: one implementation per program, each owning whatever it
/// needs to remember. `bsod` supplies three of its own.
pub(crate) trait Controller {
    fn run(&mut self, sim: &mut Sim, d: &mut Dpy);
}

/// One run of the machine: switched on, doing something for a while, switched
/// off again.
pub(crate) struct Sim {
    pub(crate) st: A2State,
    pub(crate) dec: AnalogTv,
    inp: Input,
    reception: Reception,
    font: Rc<A2Font>,

    /// What the simulated user is typing, one character every few tenths of a
    /// second.
    typing: Vec<u8>,
    typing_pos: usize,
    typing_active: bool,
    typing_rate: f64,

    /// What the machine is printing, which comes out much faster: two lines a
    /// frame.
    printing: Vec<u8>,
    printing_pos: usize,
    printing_active: bool,

    basetime: f64,
    pub(crate) curtime: f64,
    /// How long this run lasts, in seconds.
    pub(crate) delay: f64,

    pub(crate) stepno: i32,
    pub(crate) next_actiontime: f64,

    /// Taken out while it runs, so it can be handed the machine.
    controller: Option<Box<dyn Controller>>,
}

impl Sim {
    /// `apple2_start`: switch the machine on with a program to run.
    pub(crate) fn start(
        d: &mut Dpy,
        delay: f64,
        controller: Box<dyn Controller>,
        font: Rc<A2Font>,
        knobs: [f32; 4],
    ) -> Sim {
        let mut dec = AnalogTv::new(d.width(), d.height());
        let mut inp = Input::new();

        // Upstream's other flutter, `flutter_tint`, sets a field that nothing
        // reads any more, so only the horizontal one has an effect. The die is
        // still rolled, since it decides whether the second one is rolled too.
        if !random().is_multiple_of(4) && random().is_multiple_of(3) {
            dec.flutter_horiz_desync = true;
        }

        dec.set_defaults(knobs[0], knobs[1], knobs[2], knobs[3]);
        dec.squish_control = 0.05;
        inp.setup_sync(true, false);

        let mut st = A2State::new();
        st.goto(23, 0);

        // Upstream picks a random blink phase here by pushing the base time
        // back a second, and then overwrites the base time with the clock two
        // lines later, so the phase never reaches the screen. The roll is kept
        // because the sequence of random numbers after it does.
        let _unused_blink_phase = random() % 2;

        let mut sim = Sim {
            st,
            dec,
            inp,
            reception: Reception {
                level: 1.0,
                ..Default::default()
            },
            font,
            typing: Vec::new(),
            typing_pos: 0,
            typing_active: false,
            typing_rate: 1.0,
            printing: Vec::new(),
            printing_pos: 0,
            printing_active: false,
            basetime: d.time,
            curtime: 0.0,
            delay,
            stepno: 0,
            next_actiontime: 0.0,
            controller: Some(controller),
        };
        sim.run_controller(d);
        sim
    }

    pub(crate) fn type_str(&mut self, s: &str) {
        self.typing.clear();
        self.typing.extend_from_slice(s.as_bytes());
        self.typing_pos = 0;
        self.typing_active = true;
    }

    pub(crate) fn print_str(&mut self, s: &str) {
        self.printing.clear();
        self.printing.extend_from_slice(s.as_bytes());
        self.printing_pos = 0;
        self.printing_active = true;
    }

    fn run_controller(&mut self, d: &mut Dpy) {
        // Out and back, so the controller can be handed the machine it is
        // driving.
        if let Some(mut c) = self.controller.take() {
            c.run(self, d);
            self.controller = Some(c);
        }
    }

    /// Last call. Returns true if the controller asked to stay on a while
    /// longer, which the slideshow does when a picture is still on its way.
    fn finish(&mut self, d: &mut Dpy) -> bool {
        self.stepno = A2CONTROLLER_FREE;
        self.run_controller(d);
        self.stepno != A2CONTROLLER_FREE
    }

    /// Returns false when the machine has been switched off for good.
    pub(crate) fn one_frame(&mut self, d: &mut Dpy) -> bool {
        if self.stepno == A2CONTROLLER_DONE {
            return self.finish(d);
        }

        self.curtime = d.time - self.basetime;
        if self.curtime > f64::from(self.dec.powerup) {
            self.dec.powerup = self.curtime as f32;
        }

        /* The blinking rate was controlled by 555 timer with a resistor/capacitor
        time constant. Because the capacitor was electrolytic, the flash rate
        varied somewhat between machines. I'm guessing 1.6 seconds/cycle was
        reasonable. (I soldered a resistor in mine to make it blink faster.) */
        //
        // Upstream then walks the screen looking for blinking text so it can
        // mark those rows dirty, which nothing reads since the whole frame is
        // rebuilt anyway.
        self.st.blink = (self.curtime / 0.8) as i64 & 1 != 0;

        if self.curtime >= self.delay {
            self.stepno = A2CONTROLLER_DONE;
        }

        if self.printing_active {
            let mut nlcnt = 0;
            while self.printing_pos < self.printing.len() {
                let c = self.printing[self.printing_pos];
                if c == 1 {
                    /* pause */
                    self.printing_pos += 1;
                    break;
                }
                self.st.printc(c);
                self.printing_pos += 1;
                if c == b'\n' {
                    nlcnt += 1;
                    if nlcnt >= 2 {
                        break;
                    }
                }
            }
            if self.printing_pos >= self.printing.len() {
                self.printing_active = false;
            }
        } else if self.curtime >= self.next_actiontime {
            if self.typing_active {
                /* If we're in the midst of typing a string, emit a character with
                random timing. */
                if self.typing_pos >= self.typing.len() {
                    self.typing_active = false;
                } else {
                    let c = self.typing[self.typing_pos];
                    self.typing_pos += 1;
                    self.st.printc(c);
                    if c == b'\r' || c == b'\n' {
                        self.next_actiontime = self.curtime;
                    } else if c == 0o10 {
                        self.next_actiontime = self.curtime + 0.1;
                    } else {
                        self.next_actiontime = self.curtime
                            + (f64::from(random() % 1000) * 0.001 + 0.3) * self.typing_rate;
                    }
                }
            } else {
                self.next_actiontime = self.curtime;
                self.run_controller(d);
                if self.stepno == A2CONTROLLER_DONE {
                    return self.finish(d);
                }
            }
        }

        self.rasterise();
        self.reception.update();
        self.dec
            .draw(d.win(), 0.02, &[(&self.reception, &self.inp)]);
        true
    }

    /// Generate the pattern that the video circuitry shifts out of memory.
    ///
    /// It has a 14.something MHz dot clock, equal to 4 times the color burst
    /// frequency. So each group of 4 bits defines a color. Each character
    /// position, or byte in hires, defines 14 dots, so odd and even bytes have
    /// different color spaces.
    fn rasterise(&mut self) {
        // No colour burst in text mode, which is why text is grey and sharp and
        // the graphics modes are not.
        self.inp.setup_sync(self.st.gr_mode != 0, false);

        let white = analogtv::WHITE_LEVEL as i8;
        let black = analogtv::BLACK_LEVEL as i8;
        let sig = &mut self.inp.signal;

        for textrow in 0..SCREEN_ROWS {
            for row in textrow * 8..textrow * 8 + 8 {
                let mut pp = (row + analogtv::TOP + 4) * analogtv::H + analogtv::PIC_START + 100;

                if self.st.gr_mode & A2_GR_HIRES != 0
                    && (row < 160 || self.st.gr_mode & A2_GR_FULL != 0)
                {
                    /* Emulate the mysterious pink line, due to a bit getting
                    stuck in a shift register between the end of the last row
                    and the beginning of this one. */
                    if self.st.hireslines[row][0] & 0x80 != 0
                        && self.st.hireslines[row][39] & 0x40 != 0
                    {
                        sig[pp - 1] = white;
                    }

                    for col in 0..40 {
                        let b = self.st.hireslines[row][col];
                        let shift = usize::from(b & 0x80 == 0);

                        /* Each of the low 7 bits in hires mode corresponded to 2
                        dot clocks, shifted by one if the high bit was set. */
                        for i in 0..7 {
                            let v = if (b >> i) & 1 != 0 { white } else { black };
                            sig[pp + shift] = v;
                            sig[pp + shift + 1] = v;
                            pp += 2;
                        }
                    }
                } else if self.st.gr_mode & A2_GR_LORES != 0
                    && (row < 160 || self.st.gr_mode & A2_GR_FULL != 0)
                {
                    for col in 0..40 {
                        let nib = (self.st.textlines[textrow][col] >> (((row / 4) & 1) * 4)) & 0xf;
                        /* The low or high nybble was shifted out one bit at a time. */
                        for i in 0..14 {
                            sig[pp] = if (nib >> ((col * 14 + i) & 3)) & 1 != 0 {
                                white
                            } else {
                                black
                            };
                            pp += 1;
                        }
                    }
                } else {
                    for col in 0..40 {
                        let c = self.st.textlines[textrow][col];
                        /* hi bits control inverse/blink as follows:
                        0x00: inverse
                        0x40: blink
                        0x80: normal
                        0xc0: normal */
                        let rev = c & 0x80 == 0 && (c & 0x40 == 0 || self.st.blink);

                        for i in 0..7 {
                            let pix = self
                                .font
                                .pixel(i32::from((c & 0x3f) ^ 0x20) * 7 + i, (row % 8) as i32);
                            let v = if pix != rev { white } else { black };
                            sig[pp + 1] = v;
                            sig[pp + 2] = v;
                            pp += 2;
                        }
                    }
                }
            }
        }
    }
}

/* ---------------------------------------------------------------- basic */

/*
  Adding more programs is easy. Just add a listing here and to ALL_PROGRAMS,
  then add the state machine to actually execute it to basic_controller.
*/
const MOIRE_PROGRAM: &[&str] = &[
    "10 HGR2\n",
    "20 FOR Y = 0 TO 190 STEP 2\n",
    "30 HCOLOR=4 : REM BLACK\n",
    "40 HPLOT 0,191-Y TO 279,Y\n",
    "60 HCOLOR=7 : REM WHITE\n",
    "80 HPLOT 0,190-Y TO 279,Y+1\n",
    "90 NEXT Y\n",
    "100 FOR X = 0 TO 278 STEP 3\n",
    "110 HCOLOR=4\n",
    "120 HPLOT 279-X,0 TO X,191\n",
    "140 HCOLOR=7\n",
    "150 HPLOT 278-X,0 TO X+1,191\n",
    "160 NEXT X\n",
];

const SINEWAVE_PROGRAM: &[&str] = &[
    "10 HGR\n",
    "25 K=0\n",
    "30 FOR X = 0 TO 279\n",
    "32 HCOLOR= 0\n",
    "35 HPLOT X,0 TO X,159\n",
    "38 HCOLOR= 3\n",
    "40 Y = 80 + SIN(15*(X-K)/279) * 40\n",
    "50 HPLOT X,Y\n",
    "60 NEXT X\n",
    "70 K=K+4\n",
    "80 GOTO 30\n",
];

const RANDOM_LORES_PROGRAM: &[&str] = &[
    "1 REM APPLE ][ SCREEN SAVER\n",
    "10 GR\n",
    "100 COLOR= RND(1)*16\n",
    "110 X=RND(1)*40\n",
    "120 Y1=RND(1)*40\n",
    "130 Y2=RND(1)*40\n",
    "140 FOR Y = Y1 TO Y2\n",
    "150 PLOT X,Y\n",
    "160 NEXT Y\n",
    "210 Y=RND(1)*40\n",
    "220 X1=RND(1)*40\n",
    "230 X2=RND(1)*40\n",
    "240 FOR X = X1 TO X2\n",
    "250 PLOT X,Y\n",
    "260 NEXT X\n",
    "300 GOTO 100\n",
];

/// Each program, and the state that runs it once it has been typed in.
const ALL_PROGRAMS: &[(&[&str], i32)] = &[
    (MOIRE_PROGRAM, 100),
    (SINEWAVE_PROGRAM, 400),
    (RANDOM_LORES_PROGRAM, 500),
];

/// Which key is next to which, for a plausible mistype.
fn typo_for(c: u8) -> u8 {
    match c {
        b'A' => b'Q',
        b'S' => b'A',
        b'D' => b'S',
        b'F' => b'G',
        b'G' => b'H',
        b'H' => b'J',
        b'J' => b'H',
        b'K' => b'L',
        b'L' => b';',

        b'Q' => b'1',
        b'W' => b'Q',
        b'E' => b'3',
        b'R' => b'T',
        b'T' => b'Y',
        b'Y' => b'U',
        b'U' => b'Y',
        b'I' => b'O',
        b'O' => b'P',
        b'P' => b'[',

        b'Z' => b'X',
        b'X' => b'C',
        b'C' => b'V',
        b'V' => b'C',
        b'B' => b'N',
        b'N' => b'B',
        b'M' => b'N',
        b',' => b'.',
        b'.' => b',',

        b'!' => b'1',
        b'@' => b'2',
        b'#' => b'3',
        b'$' => b'4',
        b'%' => b'5',
        b'^' => b'6',
        b'&' => b'7',
        b'*' => b'8',
        b'(' => b'9',
        b')' => b'0',

        b'1' => b'Q',
        b'2' => b'W',
        b'3' => b'E',
        b'4' => b'R',
        b'5' => b'T',
        b'6' => b'Y',
        b'7' => b'U',
        b'8' => b'I',
        b'9' => b'O',
        b'0' => b'-',
        _ => 0,
    }
}

/// Mistype a line, either by transposing two letters, which BASIC will reject,
/// or by hitting the next key over and noticing a character or two later, which
/// is the one that leaves a trail of backspaces.
///
/// Returns the line as typed and the error the machine will answer with, which
/// is empty when the typist caught it themselves.
fn make_typo(orig: &str) -> (Vec<u8>, &'static str) {
    let mut out: Vec<u8> = orig.bytes().collect();
    let mut err = "";

    let mut i = 0;
    while i < out.len() {
        if i > 2 && out[i - 2] == b'R' && out[i - 1] == b'E' && out[i] == b'M' {
            break;
        }

        if out[i].is_ascii_alphabetic()
            && i + 1 < out.len()
            && out[i + 1].is_ascii_alphabetic()
            && out[i] != out[i + 1]
            && random().is_multiple_of(15)
        {
            out.swap(i, i + 1);
            err = "?SYNTAX ERROR\n";
            break;
        }

        let remain = out.len() - i;
        let errc = typo_for(out[i]);
        if random().is_multiple_of(10) && remain >= 4 && errc != 0 {
            // Type the wrong key, then a few of the characters that should have
            // followed it, then back up over the lot. The characters left in
            // between are whatever the shift moved off the end, which is what
            // upstream's overlapping copy leaves behind.
            let past = (random() % (remain as u32 - 2) + 1) as usize;
            let old_len = out.len();
            out.resize(old_len + 2 * past, 0);
            out.copy_within(i..old_len, i + 2 * past);
            out[i] = errc;
            for j in 0..past {
                out[i + past + j] = 0o10;
            }
            break;
        }

        i += 1;
    }
    (out, err)
}

#[derive(Default)]
struct Basic {
    prog_line: usize,
    x: i32,
    y: i32,
    k: i32,
    progtext: &'static [&'static str],
    progstep: i32,
    prog_start_time: f64,
    error_buf: &'static str,
}

/// Upstream also carries two states for a program it no longer runs
/// (`dumb_program`, which is `#if 0`ed out) and one, 420, that nothing reaches.
impl Controller for Basic {
    fn run(&mut self, sim: &mut Sim, _d: &mut Dpy) {
        let mine = self;

        match sim.stepno {
            0 => {
                sim.st.gr_mode = 0;
                sim.st.cls();
                sim.st.goto(0, 16);
                sim.st.prints("APPLE ][");
                sim.st.goto(23, 0);
                sim.st.printc(b']');
                sim.typing_rate = 0.2;

                let (progtext, progstep) = ALL_PROGRAMS[(random() as usize) % ALL_PROGRAMS.len()];
                mine.progtext = progtext;
                mine.progstep = progstep;
                mine.prog_line = 0;

                sim.next_actiontime += 1.0;
                sim.stepno = 10;
            }

            10 => {
                if sim.st.cursx == 0 {
                    sim.st.printc(b']');
                }
                if mine.prog_line < mine.progtext.len() {
                    if random().is_multiple_of(4) {
                        let (typed, err) = make_typo(mine.progtext[mine.prog_line]);
                        sim.typing.clear();
                        sim.typing.extend_from_slice(&typed);
                        sim.typing_pos = 0;
                        sim.typing_active = true;
                        if err.is_empty() {
                            mine.prog_line += 1;
                        } else {
                            mine.error_buf = err;
                            sim.stepno = 11;
                        }
                    } else {
                        let line = mine.progtext[mine.prog_line];
                        mine.prog_line += 1;
                        sim.type_str(line);
                    }
                } else {
                    sim.stepno = 15;
                }
            }

            11 => {
                sim.print_str(mine.error_buf);
                sim.stepno = 12;
            }

            12 => {
                if sim.st.cursx == 0 {
                    sim.st.printc(b']');
                }
                sim.next_actiontime += 1.0;
                sim.stepno = 10;
            }

            15 => {
                sim.type_str("RUN\n");
                mine.y = 0;
                mine.x = 0;
                mine.k = 0;
                mine.prog_start_time = sim.next_actiontime;
                sim.stepno = mine.progstep;
            }

            /* moire_program */
            100 => {
                sim.st.gr_mode = A2_GR_HIRES | A2_GR_FULL;
                for _ in 0..24 {
                    if mine.y >= 192 {
                        break;
                    }
                    sim.st.hline(4, 0, 191 - mine.y, 279, mine.y);
                    sim.st.hline(7, 0, 191 - mine.y - 1, 279, mine.y + 1);
                    mine.y += 2;
                }
                if mine.y >= 192 {
                    mine.x = 0;
                    sim.stepno = 110;
                }
            }

            110 => {
                for _ in 0..24 {
                    if mine.x >= 280 {
                        break;
                    }
                    sim.st.hline(4, 279 - mine.x, 0, mine.x, 192);
                    sim.st.hline(7, 279 - mine.x - 1, 0, mine.x + 1, 192);
                    mine.x += 3;
                }
                if mine.x >= 280 {
                    sim.stepno = 120;
                }
            }

            120 if sim.next_actiontime > mine.prog_start_time + sim.delay => sim.stepno = 999,

            /* sinewave_program */
            400 => {
                sim.st.gr_mode = A2_GR_HIRES;
                sim.stepno = 410;
            }

            410 => {
                for _ in 0..48 {
                    let y = 80 + (75.0 * (15.0 * f64::from(mine.x - mine.k) / 279.0).sin()) as i32;
                    sim.st.hline(0, mine.x, 0, mine.x, 159);
                    sim.st.hplot(3, mine.x, y);
                    mine.x += 1;
                    if mine.x >= 279 {
                        mine.x = 0;
                        mine.k += 4;
                    }
                }
                if sim.next_actiontime > mine.prog_start_time + sim.delay {
                    sim.stepno = 999;
                }
            }

            /* random_lores_program */
            500 => {
                sim.st.gr_mode = A2_GR_LORES | A2_GR_FULL;
                sim.st.clear_gr();
                sim.stepno = 510;
                basic_510(sim, mine); /* upstream falls through */
            }

            510 => basic_510(sim, mine),

            999 => sim.stepno = 0,

            _ => {}
        }
    }
}

/// Ten random horizontal and vertical runs of lores blocks per frame, forever.
fn basic_510(sim: &mut Sim, mine: &Basic) {
    for _ in 0..10 {
        let color = (random() % 15) as u8;
        let x = (random() % 40) as i32;
        let y1 = (random() % 48) as i32;
        let y2 = (random() % 48) as i32;
        for y in y1..y2 {
            sim.st.plot(color, x, y);
        }

        let x1 = (random() % 40) as i32;
        let x2 = (random() % 40) as i32;
        let y = (random() % 48) as i32;
        for x in x1..x2 {
            sim.st.plot(color, x, y);
        }
    }
    if sim.next_actiontime > mine.prog_start_time + sim.delay {
        sim.stepno = 999;
    }
}

/* ------------------------------------------------------------- terminal */

/*
  It's fun to put things like "gdb" as the command. For one, it's
  amusing how the standard mumble (version, no warranty, it's
  GNU/Linux dammit) occupies an entire screen on the Apple ][.
*/

struct Terminal {
    tty: Tty,
    last_emit_time: f64,
    fast_p: bool,
}

impl Terminal {
    fn new() -> Terminal {
        Terminal {
            tty: Tty::new(SCREEN_COLS as i32, SCREEN_ROWS as i32),
            last_emit_time: 0.0,
            fast_p: false,
        }
    }
}

/// Print one character to the terminal, then copy its whole grid onto the
/// character screen. Upstream notes that this could be made to redraw only what
/// changed, and that it seems plenty fast enough as it is.
fn a2_vt100_printc(st: &mut A2State, tty: &mut Tty, c: u8) {
    tty.print(u32::from(c));
    // A VT100 answers some queries; there is nothing here to answer to.
    tty.replies.clear();

    for y in 0..SCREEN_ROWS as i32 {
        for x in 0..SCREEN_COLS as i32 {
            let tc = tty.grid[(tty.width * y + x) as usize];
            let mut flag = tc.flags;
            let mut inv_p = flag & (TTY_INVERSE | TTY_BOLD) != 0;
            if tty.inverse_p {
                inv_p = !inv_p;
            }

            // Upstream converts anything outside ASCII to the nearest Latin-1,
            // and then to the nearest ASCII; here anything the machine cannot
            // show at all becomes a question mark.
            let mut ascii = if tc.c < 256 { tc.c as u8 } else { b'?' };

            if ascii == 0 {
                ascii = b' ';
            }

            st.goto(y, x);

            if flag & TTY_SYMBOLS != 0 {
                /* Convert to the nearest ASCII */
                let sub = match ascii {
                    0x60 => Some(b'*'),        /* ◆ */
                    0x6A => Some(b'J'),        /* ┘ */
                    0x6B => Some(b'T'),        /* ┐ */
                    0x6C => Some(b'r'),        /* ┌ */
                    0x6D => Some(b'L'),        /* └ */
                    0x6E => Some(b'+'),        /* ┼ */
                    0x6F | 0x70 => Some(b'-'), /* ⎺ ⎻ */
                    0x71 => Some(b'='),        /* ─ */
                    0x72 | 0x73 => Some(b'_'), /* ⎼ ⎽ */
                    0x74 => Some(b'F'),        /* ├ */
                    0x75 | 0x76 => Some(b'+'), /* ┤ ┴ */
                    0x77 => Some(b'T'),        /* ┬ */
                    0x78 => Some(b'#'),        /* │ */
                    _ => None,
                };
                if let Some(s) = sub {
                    ascii = s;
                    flag &= !TTY_SYMBOLS;
                }
            }

            if flag & TTY_SYMBOLS != 0 {
                /* Draw unknown symbol font characters as a box. */
                a2_ascii_printc(st, b' ', false, false, !inv_p, false);
            } else {
                a2_ascii_printc(st, ascii, false, flag & TTY_BLINK != 0, inv_p, false);
            }
        }
    }

    st.goto(tty.y, tty.x);
}

/// Put an ASCII character on the screen the way the machine would have to: no
/// lower case, control characters shown as their letter blinking, and capitals
/// with the high bit set, which is what makes ordinary text come out inverse.
fn a2_ascii_printc(
    st: &mut A2State,
    c: u8,
    bold_p: bool,
    blink_p: bool,
    rev_p: bool,
    scroll_p: bool,
) {
    let mut c = c;
    if c.is_ascii_lowercase() {
        /* upcase lower-case chars */
        c &= 0xDF;
    } else if c >= b'A' + 128 || (c < b' ' && c != 0o14 && c != b'\r' && c != b'\n' && c != b'\t') {
        /* upcase and blink: high-bit & ctl chrs */
        c = (c & 0x1F) | 0x80;
    } else if c.is_ascii_uppercase() {
        /* invert upper-case chars */
        c |= 0x80;
    }

    if bold_p {
        c |= 0xc0;
    }
    if blink_p {
        c = (c & !0x40) | 0x80;
    }
    if rev_p {
        c |= 0xc0;
    }

    if scroll_p {
        st.printc(c);
    } else {
        st.printc_noscroll(c);
    }
}

impl Controller for Terminal {
    fn run(&mut self, sim: &mut Sim, d: &mut Dpy) {
        let mine = self;

        match sim.stepno {
            0 => {
                if !random().is_multiple_of(2) {
                    /* Turn on color mode even though it's showing text */
                    sim.st.gr_mode |= A2_GR_FULL;
                }
                sim.st.cls();
                sim.st.goto(0, 16);
                sim.st.prints("APPLE ][");
                sim.st.goto(2, 0);

                d.text_reshape(SCREEN_COLS as i32, SCREEN_ROWS as i32);

                if !mine.fast_p {
                    sim.next_actiontime += 4.0;
                }
                sim.stepno = 10;

                mine.last_emit_time = sim.curtime;
            }

            10 | 11 => {
                let first_line_p = sim.stepno == 10;
                let elapsed = sim.curtime - mine.last_emit_time;

                let mut nwant = (elapsed * 25.0) as i32; /* characters per second */

                if first_line_p {
                    sim.stepno = 11;
                    nwant = 1;
                }

                if nwant > 40 {
                    nwant = 40;
                }

                if mine.fast_p {
                    nwant = 1023;
                }

                if nwant > 0 {
                    mine.last_emit_time = sim.curtime;

                    for _ in 0..nwant {
                        let Some(c) = d.text_getc() else { break };
                        a2_vt100_printc(&mut sim.st, &mut mine.tty, c);
                    }
                }
            }

            _ => {}
        }
    }
}

/* ------------------------------------------------------------ slideshow */

/// The Apple ][ colour map. Each pixel can only be 1 or 0, but what that means
/// depends on whether it's an odd or even pixel, and whether the high bit in
/// the byte is set or not. If it's 0, it's always black.
const A2_CMAP: [[[i32; 3]; 2]; 2] = [
    [
        /* hibit=0 */
        [0x00, 0x80, 0xff], /* odd pixels = blue */
        [0xff, 0x80, 0x00], /* even pixels = red */
    ],
    [
        /* hibit=1 */
        [0xa0, 0x40, 0xa0], /* even pixels = purple */
        [0x40, 0xff, 0x40], /* odd pixels = green */
    ],
];

#[derive(Default)]
struct Slideshow {
    render_img_lineno: usize,
    render_img: Vec<u8>,
    img_filename: Option<String>,
    load: Option<ImageLoad>,
    canvas: Option<Fb>,
}

/// Scale a picture down to the Apple's screen, averaging whatever number of
/// source pixels land in each destination one.
///
/// Upstream also has to decode the server's visual here, since the image it is
/// handed could be anything; ours is always 32bpp already.
fn scale_image(src: &Fb, fromx: i32, fromy: i32, fromw: i32, fromh: i32, out: &mut [u32], w: i32) {
    let h = (out.len() / w as usize) as i32;
    let scale = if fromw > fromh {
        fromw as f32 / w as f32
    } else {
        fromh as f32 / h as f32
    };

    /* iterate over dest pixels */
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);

            let xx1 = (x as f32 * scale) as i32 + fromx;
            let yy1 = (y as f32 * scale) as i32 + fromy;
            let xx2 = ((x + 1) as f32 * scale) as i32 + fromx;
            let yy2 = ((y + 1) as f32 * scale) as i32 + fromy;

            /* Iterate over the source pixels contributing to this one, and sum. */
            for xx in xx1..xx2 {
                for yy in yy1..yy2 {
                    let (rr, gg, bb) = unrgb(src.get_pixel(xx, yy));
                    r += u32::from(rr);
                    g += u32::from(gg);
                    b += u32::from(bb);
                }
            }

            /* Scale summed pixel values down to 8/8/8 range */
            let n = ((xx2 - xx1) * (yy2 - yy1)).max(1) as u32;
            out[(y * w + x) as usize] = ((r / n) << 16) | ((g / n) << 8) | (b / n);
        }
    }
}

/// Pick a random sub-image out of the source, near the middle, and scale that.
fn pick_a2_subimage(src: &Fb, out: &mut [u32], w: i32, h: i32) {
    let (iw, ih) = (src.width(), src.height());
    let (fromx, fromy, fromw, fromh);
    if iw <= w || ih <= h {
        fromx = 0;
        fromy = 0;
        fromw = iw;
        fromh = ih;
    } else {
        let (mut ww, mut hh);
        loop {
            let scale = 0.5 + frand(0.7) + frand(0.7) + frand(0.7);
            ww = (f64::from(w) * scale) as i32;
            hh = (f64::from(h) * scale) as i32;
            if ww <= iw && hh <= ih {
                break;
            }
        }
        fromw = ww;
        fromh = hh;

        let dw = (iw - fromw) / 2; /* near the center! */
        let dh = (ih - fromh) / 2;

        fromx = if dw <= 0 {
            0
        } else {
            (random() % dw as u32) as i32 + dw / 2
        };
        fromy = if dh <= 0 {
            0
        } else {
            (random() % dh as u32) as i32 + dh / 2
        };
    }

    scale_image(src, fromx, fromy, fromw, fromh, out, w);
}

/// Floyd-Steinberg dither to the six hires colours.
///
/// Derived from `ppmquant.c`, Copyright (c) 1989, 1991 by Jef Poskanzer.
///
/// It is not an ordinary dither: the seven pixels of a byte share one high bit,
/// which decides which pair of colours they can be, so every group has to be
/// tried both ways and the cheaper one kept.
fn a2_dither(input: &[u32], w: usize, h: usize) -> Vec<u8> {
    let maxval = 255i32;
    let fs_scale = 1024i32;
    let brightness = 75i32;

    let mut out = vec![0u8; (w / 7) * h];

    /* Initialize Floyd-Steinberg error vectors. */
    let mut this_rerr = vec![0i32; w + 2];
    let mut next_rerr = vec![0i32; w + 2];
    let mut this_gerr = vec![0i32; w + 2];
    let mut next_gerr = vec![0i32; w + 2];
    let mut this_berr = vec![0i32; w + 2];
    let mut next_berr = vec![0i32; w + 2];

    let mut pixels = input.to_vec();

    for x in 0..w + 2 {
        /* (random errors in [-1 .. 1]) */
        this_rerr[x] = (random() % (fs_scale as u32 * 2)) as i32 - fs_scale;
        this_gerr[x] = (random() % (fs_scale as u32 * 2)) as i32 - fs_scale;
        this_berr[x] = (random() % (fs_scale as u32 * 2)) as i32 - fs_scale;
    }

    for y in 0..h {
        for x in 0..w + 2 {
            next_rerr[x] = 0;
            next_gerr[x] = 0;
            next_berr[x] = 0;
        }

        /* It's too complicated to go back and forth on alternate rows, so we
        always go left-right here. It doesn't change the result very much.

        For each group of 7 pixels, we have to try it both with the high bit=0
        and =1. For each high bit value, we add up the total error and pick the
        best one.

        Because we have to go through each group of bits twice, we don't
        propagate the error values through this_[rgb]err since it would add them
        twice. So we keep seperate local_[rgb]err variables for propagating
        error within the 7-pixel group. */

        let row = y * w;
        let mut prev_byte = 0u8;

        let mut xbyte = 0;
        while xbyte < 280 {
            let mut best_byte = 0u8;
            let mut best_error = 2_000_000_000i32;

            for (hibit, cmap) in A2_CMAP.iter().enumerate() {
                let mut byte = (hibit as u8) << 7;
                let mut tot_error = 0i32;
                let (mut local_rerr, mut local_gerr, mut local_berr) = (0i32, 0i32, 0i32);

                for x in xbyte..xbyte + 7 {
                    /* Use Floyd-Steinberg errors to adjust actual color. */
                    let p = pixels[row + x];
                    let mut sr = ((p >> 16) & 0xFF) as i32 * brightness / 256;
                    let mut sg = ((p >> 8) & 0xFF) as i32 * brightness / 256;
                    let mut sb = (p & 0xFF) as i32 * brightness / 256;
                    sr += (this_rerr[x + 1] + local_rerr) / fs_scale;
                    sg += (this_gerr[x + 1] + local_gerr) / fs_scale;
                    sb += (this_berr[x + 1] + local_berr) / fs_scale;

                    sr = sr.clamp(0, maxval);
                    sg = sg.clamp(0, maxval);
                    sb = sb.clamp(0, maxval);

                    /* This is the color we'd get if we set the bit 1. For 0, we
                    get black */
                    let [r2, g2, b2] = cmap[x & 1];

                    /* dist0 and dist1 are the error (Minkowski 2-metric
                    distances in the color space) for choosing 0 and 1
                    respectively. 0 is black, 1 is the color r2,g2,b2. */
                    let dist1 =
                        (sr - r2) * (sr - r2) + (sg - g2) * (sg - g2) + (sb - b2) * (sb - b2);
                    let dist0 = sr * sr + sg * sg + sb * sb;

                    if dist1 < dist0 {
                        byte |= 1 << (x - xbyte);
                        tot_error += dist1;

                        /* Wanted sr but got r2, so propagate sr-r2 */
                        local_rerr = (sr - r2) * fs_scale * 7 / 16;
                        local_gerr = (sg - g2) * fs_scale * 7 / 16;
                        local_berr = (sb - b2) * fs_scale * 7 / 16;
                    } else {
                        tot_error += dist0;

                        /* Wanted sr but got 0, so propagate sr */
                        local_rerr = sr * fs_scale * 7 / 16;
                        local_gerr = sg * fs_scale * 7 / 16;
                        local_berr = sb * fs_scale * 7 / 16;
                    }
                }

                if tot_error < best_error {
                    best_byte = byte;
                    best_error = tot_error;
                }
            }

            /* Avoid alternating 7f and ff in all-white areas, because it makes
            regular pink vertical lines */
            if (best_byte & 0x7f) == 0x7f && (prev_byte & 0x7f) == 0x7f {
                best_byte = prev_byte;
            }
            prev_byte = best_byte;

            /* Now that we've chosen values for all 8 bits of the byte, we have to
            fill in the real pixel values into pP and propagate all the error
            terms. We end up repeating a lot of the code above. */

            for x in xbyte..xbyte + 7 {
                let bit = i32::from((best_byte >> (x - xbyte)) & 1);
                let hibit = ((best_byte >> 7) & 1) as usize;

                let p = pixels[row + x];
                let mut sr = ((p >> 16) & 0xFF) as i32;
                let mut sg = ((p >> 8) & 0xFF) as i32;
                let mut sb = (p & 0xFF) as i32;
                sr += this_rerr[x + 1] / fs_scale;
                sg += this_gerr[x + 1] / fs_scale;
                sb += this_berr[x + 1] / fs_scale;

                sr = sr.clamp(0, maxval);
                sg = sg.clamp(0, maxval);
                sb = sb.clamp(0, maxval);

                let [r2, g2, b2] = A2_CMAP[hibit][x & 1];
                let (r2, g2, b2) = (r2 * bit, g2 * bit, b2 * bit);

                pixels[row + x] = ((r2 as u32) << 16) | ((g2 as u32) << 8) | b2 as u32;

                /* Propagate Floyd-Steinberg error terms. */
                let err = (sr - r2) * fs_scale;
                this_rerr[x + 2] += (err * 7) / 16;
                next_rerr[x] += (err * 3) / 16;
                next_rerr[x + 1] += (err * 5) / 16;
                next_rerr[x + 2] += err / 16;
                let err = (sg - g2) * fs_scale;
                this_gerr[x + 2] += (err * 7) / 16;
                next_gerr[x] += (err * 3) / 16;
                next_gerr[x + 1] += (err * 5) / 16;
                next_gerr[x + 2] += err / 16;
                let err = (sb - b2) * fs_scale;
                this_berr[x + 2] += (err * 7) / 16;
                next_berr[x] += (err * 3) / 16;
                next_berr[x + 1] += (err * 5) / 16;
                next_berr[x + 2] += err / 16;
            }

            /* And put the actual byte into out. */
            out[y * (w / 7) + xbyte / 7] = best_byte;

            xbyte += 7;
        }

        std::mem::swap(&mut this_rerr, &mut next_rerr);
        std::mem::swap(&mut this_gerr, &mut next_gerr);
        std::mem::swap(&mut this_berr, &mut next_berr);
    }

    out
}

/// `BLOAD IMAGE`: what the file would have been called, in the machine's own
/// terms. No lower case, no punctuation, and thirty characters is a long name.
fn a2_basename(title: &str) -> String {
    let mut basename = title;
    while let Some(slash) = basename.find('/') {
        if slash + 1 >= basename.len() {
            break;
        }
        basename = &basename[slash + 1..];
    }
    if let Some(dot) = basename.rfind('.') {
        basename = &basename[..dot];
    }
    basename
        .bytes()
        .take(20)
        .map(|c| {
            let c = c.to_ascii_uppercase();
            if c <= b' ' { '_' } else { char::from(c) }
        })
        .collect()
}

/*
  TODO: this should load 10 images at startup time, then cycle through them
  to avoid the pause while it loads.
*/
impl Controller for Slideshow {
    fn run(&mut self, sim: &mut Sim, d: &mut Dpy) {
        let mine = self;

        match sim.stepno {
            0 => {
                sim.st.clear_hgr();
                sim.st.cls();
                sim.typing_rate = 0.3;
                sim.dec.powerup = 0.0;

                sim.st.goto(0, 16);
                sim.st.prints("APPLE ][");
                sim.st.goto(23, 0);
                sim.st.printc(b']');

                sim.stepno = 10;
            }

            10 => {
                let mut canvas = Fb::new(d.width().max(1), d.height().max(1));
                mine.load = d.load_image_into(&mut canvas, None);
                mine.canvas = Some(canvas);

                /* pause with a blank screen for a bit, while the image loads in the
                background. */
                sim.next_actiontime += 2.0;
                sim.stepno = 11;
            }

            11 => {
                if mine.load.is_some()
                    && let Some(mut canvas) = mine.canvas.take()
                {
                    mine.load = d.load_image_into(&mut canvas, mine.load.take());
                    mine.canvas = Some(canvas);
                }

                if mine.load.is_none() {
                    /* image is finally loaded */
                    if let Some(canvas) = mine.canvas.take() {
                        let (w, h) = (280usize, 192usize);
                        let mut buf32 = vec![0u32; w * h];
                        pick_a2_subimage(&canvas, &mut buf32, w as i32, h as i32);
                        mine.render_img = a2_dither(&buf32, w, h);
                        mine.img_filename = d.image_title().map(str::to_string);
                    }

                    sim.stepno = if sim.st.gr_mode != 0 { 30 } else { 20 };
                    sim.next_actiontime += 3.0;
                }
            }

            20 => {
                sim.type_str("HGR\n");
                sim.stepno = 29;
            }

            29 => {
                sim.print_str("]");
                sim.stepno = 30;
            }

            30 => {
                sim.st.gr_mode = A2_GR_HIRES;
                match &mine.img_filename {
                    Some(name) => {
                        let line = format!("BLOAD {}\n", a2_basename(name));
                        sim.type_str(&line);
                    }
                    None => sim.type_str("BLOAD IMAGE\n"),
                }
                mine.render_img_lineno = 0;

                sim.stepno = 35;
            }

            35 => {
                sim.next_actiontime += 0.7;
                sim.stepno = 40;
            }

            40 => {
                if mine.render_img_lineno >= 192 {
                    sim.print_str("]");
                    sim.type_str("POKE 49234,0\n");
                    sim.stepno = 50;
                } else {
                    for _ in 0..6 {
                        if mine.render_img_lineno >= 192 {
                            break;
                        }
                        sim.st
                            .display_image_loading(&mine.render_img, mine.render_img_lineno);
                        mine.render_img_lineno += 1;
                    }

                    /* The disk would have to seek every 13 sectors == 78 lines.
                    (This ain't no newfangled 16-sector operating system) */
                    if mine.render_img_lineno.is_multiple_of(78) {
                        sim.next_actiontime += 0.5;
                    } else {
                        sim.next_actiontime += 0.08;
                    }
                }
            }

            50 => {
                sim.st.gr_mode |= A2_GR_FULL;
                sim.stepno = 60;
                /* Note that sim->delay is sometimes "infinite" in this controller.
                These images are kinda dull anyway, so don't leave it on too long. */
                sim.next_actiontime += 2.0;
            }

            60 => {
                sim.print_str("]");
                sim.type_str("POKE 49235,0\n");
                sim.stepno = 70;
            }

            70 => {
                sim.print_str("]");
                sim.st.gr_mode &= !A2_GR_FULL;
                mine.render_img = Vec::new();
                mine.img_filename = None;
                sim.stepno = 10;
            }

            80 => {
                /* Do nothing, just wait */
                sim.next_actiontime += 2.0;
                sim.stepno = A2CONTROLLER_FREE;
            }

            /* It is possible that a still image is being loaded, in which case wait
            rather than throwing away what it is loading into. */
            A2CONTROLLER_FREE if mine.load.is_some() => sim.stepno = 80,

            _ => {}
        }
    }
}

/* ------------------------------------------------------------------ the hack */

struct Apple2 {
    duration: f64,
    random_p: bool,
    mode: Mode,
    knobs: [f32; 4],
    font: Rc<A2Font>,
    sim: Option<Sim>,
}

impl Screenhack for Apple2 {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.sim.is_none() {
            if self.random_p {
                self.mode = match random() % 3 {
                    0 => Mode::Slideshow,
                    1 => Mode::Terminal,
                    _ => Mode::Basic,
                };
            }
            let controller: Box<dyn Controller> = match self.mode {
                Mode::Slideshow => Box::new(Slideshow::default()),
                Mode::Terminal => Box::new(Terminal::new()),
                Mode::Basic => Box::new(Basic::default()),
            };
            self.sim = Some(Sim::start(
                d,
                self.duration,
                controller,
                self.font.clone(),
                self.knobs,
            ));
        }

        let mut done = false;
        if let Some(sim) = self.sim.as_mut() {
            done = !sim.one_frame(d);
        }
        if done {
            self.sim = None;
            d.win().clear(color::BLACK);
        }

        5000
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        if let Some(sim) = self.sim.as_mut() {
            sim.dec.configure(width, height);
        }
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        // Upstream's terminal mode is a working VT100 when it is run as an
        // application: what you type goes to the program it is showing. There
        // is no program at this end of the text, so there is nothing to type at.
        false
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut duration = f64::from(d.res.int("duration")).max(1.0);

    let (random_p, mode) = match d.res.string("mode") {
        "text" => (false, Mode::Terminal),
        "slideshow" => (false, Mode::Slideshow),
        "basic" => (false, Mode::Basic),
        _ => (true, Mode::Basic),
    };

    if !random_p && (mode == Mode::Terminal || mode == Mode::Slideshow) {
        duration = 999_999.0; /* these run "forever" */
    }

    Box::new(Apple2 {
        duration,
        random_p,
        mode,
        knobs: [
            d.res.float("TVColor") as f32,
            d.res.float("TVTint") as f32,
            d.res.float("TVBrightness") as f32,
            d.res.float("TVContrast") as f32,
        ],
        font: Rc::new(A2Font::load()),
        sim: None,
    })
}

const DEFAULTS: &[&str] = &[
    ".background:	   black",
    ".foreground:	   white",
    "*mode:		   random",
    "*duration:		   60",
    "*program:		   xscreensaver-text --cols 40",
    "*fast:		   False",
    "*TVColor:         70",
    "*TVTint:          5",
    "*TVBrightness:    2",
    "*TVContrast:    150",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Choose display mode randomly",
    },
    SelectItem {
        value: "text",
        label: "Display scrolling text",
    },
    SelectItem {
        value: "slideshow",
        label: "Display images",
    },
    SelectItem {
        value: "basic",
        label: "Run basic programs",
    },
];

/// The brightness default is the C's rather than
/// `hacks/config/apple2.xml`'s, which seeds its dialog with the value upstream
/// compiles in for phones. The ranges and labels are the XML's.
const OPTS: &[Opt] = &[
    Opt::select("mode", "Display mode", MODES, "random"),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "60"),
    Opt::slider("TVColor", "Color Knob", 0.0, 400.0, 5.0, 0, "70"),
    Opt::slider("TVTint", "Tint Knob", 0.0, 360.0, 5.0, 0, "5"),
    Opt::slider("TVBrightness", "Brightness Knob", -75.0, 100.0, 1.0, 0, "2"),
    Opt::slider("TVContrast", "Contrast Knob", 0.0, 500.0, 10.0, 0, "150"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "apple2",
    label: "Apple ][",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Trevor Blackwell and Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=p3QZqhp67l8"),
        blurb: "An Apple ][+ on a colour television, in all its 1979 glory.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_reads_the_right_way_round() {
        let font = A2Font::load();
        // Glyph 0 is space, glyph 33 is 'A': the sheet holds 32 onwards.
        let ink = |glyph: i32| (0..7).any(|x| (0..FONT_H).any(|y| font.pixel(glyph * 7 + x, y)));
        assert!(!ink(0), "the space is not blank");
        assert!(ink(33), "the A has no ink");
    }

    #[test]
    fn the_screen_scrolls_when_it_is_written_off_the_bottom() {
        let mut st = A2State::new();
        st.cls();
        st.goto(23, 0);
        st.prints("HELLO");
        assert_eq!(st.textlines[23][0], b'H' ^ 0xc0);

        st.printc(b'\n');
        // The bottom row is now blank and the cursor has not left it.
        assert_eq!(st.cursy, 23);
        assert_eq!(st.textlines[23][1], 0xe0);
        // And what was on it has gone: nothing scrolled up into row 22 that was
        // not there, because row 23 was the only row with anything on it.
        assert_eq!(st.textlines[22][0], b'H' ^ 0xc0);
    }

    #[test]
    fn a_backspace_at_the_left_margin_lands_at_the_end_of_the_line_above() {
        let mut st = A2State::new();
        st.cls();
        st.goto(5, 0);
        st.printc(0o10);
        assert_eq!(st.cursx, -1);
        // The blink bit is on at the end of the row above, and nothing was
        // written outside the screen.
        assert_eq!(st.textlines[4][39] & 0x80, 0);

        // And from the very first cell there is nowhere to back up to.
        st.goto(0, 0);
        st.printc(0o10);
        assert_eq!(st.textlines[0][0] & 0x80, 0);
    }

    #[test]
    fn hires_plots_two_dots_and_the_high_bit_picks_the_palette() {
        let mut st = A2State::new();
        // Colours 0 to 3 are the high-bit palette and green is its odd pixels,
        // so plotting green at x=0 lights x=1 instead.
        st.hplot(1, 0, 0);
        assert_eq!(st.hireslines[0][0], 0x80 | 0x02);
        // Blue is the other palette, and its even pixels.
        st.hplot(6, 0, 100);
        assert_eq!(st.hireslines[100][0], 0x01);
        // Black leaves the row empty of dots either way.
        st.hplot(0, 7, 3);
        assert_eq!(st.hireslines[3][1] & 0x7f, 0);
    }

    #[test]
    fn the_typist_leaves_a_trail_of_backspaces_or_a_syntax_error() {
        crate::runtime::ya_rand_init(7);
        let mut saw_backspace = false;
        let mut saw_syntax_error = false;
        for _ in 0..2000 {
            let (typed, err) = make_typo("100 FOR X = 0 TO 278 STEP 3\n");
            if typed.contains(&0o10) {
                saw_backspace = true;
                assert!(err.is_empty(), "a corrected typo should not be an error");
            }
            if !err.is_empty() {
                saw_syntax_error = true;
                assert!(!typed.contains(&0o10));
            }
        }
        assert!(saw_backspace);
        assert!(saw_syntax_error);
    }

    #[test]
    fn a_dithered_picture_is_not_one_flat_colour() {
        // A red-to-blue ramp, which the six colours have to work for.
        let (w, h) = (280usize, 192usize);
        let mut src = vec![0u32; w * h];
        for y in 0..h {
            for x in 0..w {
                let r = (x * 255 / w) as u32;
                let b = 255 - r;
                src[y * w + x] = (r << 16) | b;
            }
        }
        let out = a2_dither(&src, w, h);
        assert_eq!(out.len(), (w / 7) * h);
        let set: std::collections::HashSet<u8> = out.iter().copied().collect();
        assert!(set.len() > 8, "only {} distinct bytes", set.len());
    }
}
