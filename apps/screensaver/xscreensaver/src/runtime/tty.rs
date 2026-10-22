//! Port of `hacks/ansi-tty.c`: an ANSI (VT100) terminal, as a character grid.
//!
//! ```text
//! xscreensaver, Copyright © 2025-2026 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! An ANSI (VT100) terminal emulator.  Reads control codes and renders the
//! screen into a character grid.  Other programs (apple2 and phosphor) then
//! copy that layout to the screen while applying their own text styling.
//!
//!   https://en.wikipedia.org/wiki/ANSI_escape_code
//!   https://vt100.net/docs/vt100-ug/chapter3.html
//!   https://invisible-island.net/xterm/ctlseqs/ctlseqs.html
//!   https://en.wikipedia.org/wiki/ISO/IEC_2022
//! ```
//!
//! Bytes in, a grid of characters out. It knows nothing about drawing: a hack
//! feeds it whatever the text source produced and then renders the grid in
//! whatever way it renders things, which is how one emulator serves both a
//! phosphor screen and an Apple II.
//!
//! Upstream carries a large amount of logging, gated behind a debug level, that
//! names every sequence it recognises and every one it does not. There is
//! nowhere to log to here, so it is left out; the commands themselves are all
//! present, including the ones whose implementation upstream is to do nothing.
//!
//! The one genuinely subtle thing in here is the Last Column Flag, and
//! upstream's comment on it is worth keeping:
//!
//! > When a character is printed in column 80, the insertion point does not
//! > wrap to the next line until the 81st character is printed, which will then
//! > appear in the leftmost column of the following line. In this case, the
//! > cursor blinks atop the 80th character instead of after it. This means that
//! > a cursor at position 80 is visually ambiguous with one at position 79.
//! > Likewise, a character printed in the bottom right cell does not cause the
//! > screen to scroll up by one line until the *next* character is printed.
//! > However, this only applies to cursor coordinates caused by text insertion.

use super::color::{Pixel, rgb};

pub const TTY_BOLD: u8 = 1;
pub const TTY_ITALIC: u8 = 2;
pub const TTY_INVERSE: u8 = 4;
pub const TTY_DIM: u8 = 8;
pub const TTY_UNDERLINE: u8 = 16;
pub const TTY_BLINK: u8 = 32;
pub const TTY_SYMBOLS: u8 = 64;

const ESC: u32 = 0x1B;
/// The "no argument given" marker, distinct from a zero argument.
const UNDEF: i32 = -0xFFFF;

/// One cell. `c` is zero for a cell nothing has ever been printed to, which is
/// not the same as a space.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TtyChar {
    pub c: u32,
    pub flags: u8,
    pub fg: Pixel,
    pub bg: Pixel,
}

impl Default for TtyChar {
    fn default() -> Self {
        TtyChar {
            c: 0,
            flags: 0,
            fg: GREEN,
            bg: BLACK,
        }
    }
}

/// The sixteen VGA colours, which is what a terminal means by "colour 3".
const CMAP: [Pixel; 16] = [
    rgb(0x00, 0x00, 0x00), // 30 40  Black
    rgb(0xAA, 0x00, 0x00), // 31 41  Red
    rgb(0x00, 0xAA, 0x00), // 32 42  Green
    rgb(0xAA, 0x55, 0x00), // 33 43  Yellow
    rgb(0x00, 0x00, 0xAA), // 34 44  Blue
    rgb(0xAA, 0x00, 0xAA), // 35 45  Magenta
    rgb(0x00, 0xAA, 0xAA), // 36 46  Cyan
    rgb(0xAA, 0xAA, 0xAA), // 37 47  White
    rgb(0x55, 0x55, 0x55), // 90 100 Bright Black
    rgb(0xFF, 0x55, 0x55), // 91 101 Bright Red
    rgb(0x55, 0xFF, 0x55), // 92 102 Bright Green
    rgb(0xFF, 0xFF, 0x55), // 93 103 Bright Yellow
    rgb(0x55, 0x55, 0xFF), // 94 104 Bright Blue
    rgb(0xFF, 0x55, 0xFF), // 95 105 Bright Magenta
    rgb(0x55, 0xFF, 0xFF), // 96 106 Bright Cyan
    rgb(0xFF, 0xFF, 0xFF), // 97 107 Bright White
];
const BLACK: Pixel = CMAP[0];
const GREEN: Pixel = CMAP[2];

/// The DEC Special Graphics character set the VT100 drew boxes with. Only the
/// printable range differs from ASCII; everything else is zero.
pub const GRAPHICS_UNICODE: [u32; 256] = {
    let mut m = [0u32; 256];
    m[0x5F] = ' ' as u32;
    m[0x60] = 0x25C6; // ◆ BLACK DIAMOND
    m[0x61] = 0x2592; // ▒ MEDIUM SHADE
    m[0x62] = 0x2409; // ␉ SYMBOL FOR HORIZONTAL TABULATION
    m[0x63] = 0x240C; // ␌ SYMBOL FOR FORM FEED
    m[0x64] = 0x240D; // ␍ SYMBOL FOR CARRIAGE RETURN
    m[0x65] = 0x240A; // ␊ SYMBOL FOR LINE FEED
    m[0x66] = 0x00B0; // ° DEGREE SIGN
    m[0x67] = 0x00B1; // ± PLUS-MINUS SIGN
    m[0x68] = 0x2424; // ␤ SYMBOL FOR NEWLINE
    m[0x69] = 0x240B; // ␋ SYMBOL FOR VERTICAL TABULATION
    m[0x6A] = 0x2518; // ┘ BOX DRAWINGS LIGHT UP AND LEFT
    m[0x6B] = 0x2510; // ┐ BOX DRAWINGS LIGHT DOWN AND LEFT
    m[0x6C] = 0x250C; // ┌ BOX DRAWINGS LIGHT DOWN AND RIGHT
    m[0x6D] = 0x2514; // └ BOX DRAWINGS LIGHT UP AND RIGHT
    m[0x6E] = 0x253C; // ┼ BOX DRAWINGS LIGHT VERTICAL AND HORIZONTAL
    m[0x6F] = 0x23BA; // ⎺ HORIZONTAL SCAN LINE-1
    m[0x70] = 0x23BB; // ⎻ HORIZONTAL SCAN LINE-3
    m[0x71] = 0x2500; // ─ BOX DRAWINGS LIGHT HORIZONTAL
    m[0x72] = 0x23BC; // ⎼ HORIZONTAL SCAN LINE-7
    m[0x73] = 0x23BD; // ⎽ HORIZONTAL SCAN LINE-9
    m[0x74] = 0x251C; // ├ BOX DRAWINGS LIGHT VERTICAL AND RIGHT
    m[0x75] = 0x2524; // ┤ BOX DRAWINGS LIGHT VERTICAL AND LEFT
    m[0x76] = 0x2534; // ┴ BOX DRAWINGS LIGHT UP AND HORIZONTAL
    m[0x77] = 0x252C; // ┬ BOX DRAWINGS LIGHT DOWN AND HORIZONTAL
    m[0x78] = 0x2502; // │ BOX DRAWINGS LIGHT VERTICAL
    m[0x79] = 0x2264; // ≤ LESS-THAN OR EQUAL TO
    m[0x7A] = 0x2265; // ≥ GREATER-THAN OR EQUAL TO
    m[0x7B] = 0x03C0; // π GREEK SMALL LETTER PI
    m[0x7C] = 0x2260; // ≠ NOT EQUAL TO
    m[0x7D] = 0x00A3; // £ POUND SIGN
    m[0x7E] = 0x00B7; // · MIDDLE DOT
    m
};

/// Where the cursor and the text properties were when they were saved.
#[derive(Clone, Copy, Default)]
struct Saved {
    x: i32,
    y: i32,
    lcf: bool,
    flags: u8,
}

pub struct Tty {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Set by "reverse video"; the hack flips the sense of `TTY_INVERSE`.
    pub inverse_p: bool,
    pub grid: Vec<TtyChar>,

    /// What the terminal has to say back: cursor reports, identification. The
    /// hack drains this and feeds it to whatever is on the other end.
    pub replies: String,

    /// The command being assembled, if any.
    buf: Vec<u8>,
    awaiting_st: bool,
    unicrud: i32,
    auto_wrap_p: bool,
    origin_relative_p: bool,
    linefeed_p: bool,
    /// Last Column Flag: see the note at the top of the file.
    lcf: bool,
    scroll_y1: i32,
    scroll_y2: i32,
    saved: Saved,
    flags: u8,
    g0: u8,
    g1: u8,
    fg: Pixel,
    bg: Pixel,
    /// One entry per column, set where there is a tab stop.
    tabs: Vec<bool>,
}

impl Tty {
    pub fn new(w: i32, h: i32) -> Tty {
        let (w, h) = (w.max(3), h.max(3));
        let mut tty = Tty {
            x: 0,
            y: 0,
            width: w,
            height: h,
            inverse_p: false,
            grid: vec![TtyChar::default(); (w * h) as usize],
            replies: String::new(),
            buf: Vec::new(),
            awaiting_st: false,
            unicrud: 0,
            auto_wrap_p: true,
            origin_relative_p: false,
            linefeed_p: false,
            lcf: false,
            scroll_y1: 0,
            scroll_y2: h,
            saved: Saved::default(),
            flags: 0,
            g0: 0,
            g1: 0,
            fg: GREEN,
            bg: BLACK,
            tabs: vec![false; w as usize],
        };
        tty.reset();
        tty
    }

    /// A tab stop every eight columns.
    fn default_tabs(&mut self) {
        for (i, t) in self.tabs.iter_mut().enumerate() {
            *t = i % 8 == 0;
        }
    }

    fn reset(&mut self) {
        self.grid.fill(TtyChar::default());
        self.x = 0;
        self.y = 0;
        self.buf.clear();
        self.awaiting_st = false;
        self.unicrud = 0;
        self.origin_relative_p = false;
        self.linefeed_p = false;
        self.lcf = false;
        self.saved = Saved::default();
        self.flags = 0;
        self.g0 = 0;
        self.g1 = 0;
        self.scroll_y1 = 0;
        self.scroll_y2 = self.height;
        self.auto_wrap_p = true;
        self.fg = GREEN;
        self.bg = BLACK;
        self.default_tabs();
    }

    pub fn resize(&mut self, w: i32, h: i32) {
        let (w, h) = (w.max(3), h.max(3));
        let mut grid2 = vec![TtyChar::default(); (w * h) as usize];
        for y in 0..self.height.min(h) {
            for x in 0..self.width.min(w) {
                grid2[(y * w + x) as usize] = self.grid[(y * self.width + x) as usize];
            }
        }
        self.grid = grid2;
        self.width = w;
        self.height = h;
        self.tabs.resize(w as usize, false);

        if self.x >= w {
            self.x = w - 1;
        }
        self.scroll_y1 = self.scroll_y1.min(self.height);
        self.scroll_y2 = self.scroll_y2.min(self.height);
        if self.y >= self.scroll_y2 {
            self.y = self.scroll_y2 - 1;
        }
    }

    /// Move the lines inside the scrolling region up (positive) or down.
    fn scroll(&mut self, lines: i32) {
        let w = self.width as usize;
        let top = self.scroll_y1.max(0) as usize * w;
        let bot = (self.scroll_y2.max(0) as usize * w).min(self.grid.len());
        if top >= bot {
            return;
        }
        let span = self.scroll_y2 - self.scroll_y1;
        if lines >= span || -lines >= span {
            // Scrolling more lines than exist is clearing.
            self.grid[top..bot].fill(TtyChar::default());
        } else if lines > 0 {
            let move_ = lines as usize * w;
            self.grid.copy_within(top + move_..bot, top);
            self.grid[bot - move_..bot].fill(TtyChar::default());
        } else if lines < 0 {
            let move_ = (-lines) as usize * w;
            self.grid.copy_within(top..bot - move_, top + move_);
            self.grid[top..top + move_].fill(TtyChar::default());
        }
    }

    /// Clear a run of cells, both positions inclusive.
    fn erase(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) {
        let x1 = x1.clamp(0, self.width - 1);
        let x2 = x2.clamp(0, self.width - 1);
        let y1 = y1.clamp(0, self.height - 1);
        let y2 = y2.clamp(0, self.height - 1);
        let from = (y1 * self.width + x1) as usize;
        let to = (y2 * self.width + x2) as usize;
        if from > to {
            return;
        }
        // Note that this resets the colours to the defaults rather than to
        // whatever is currently selected: turning on inverse and clearing to
        // end of line does not give you an inverted line.
        self.grid[from..=to].fill(TtyChar::default());
    }

    fn set_color(&mut self, fg_p: bool, color: i32) {
        let c = if (0..CMAP.len() as i32).contains(&color) {
            CMAP[color as usize]
        } else {
            rgb(
                ((color >> 16) & 0xFF) as u8,
                ((color >> 8) & 0xFF) as u8,
                (color & 0xFF) as u8,
            )
        };
        if fg_p {
            self.fg = c;
        } else {
            self.bg = c;
        }
    }
}

impl Tty {
    /// One byte in. Multi-byte UTF-8 and escape sequences are assembled across
    /// calls, so a hack can just hand over whatever the text source gave it.
    pub fn print(&mut self, c: u32) {
        let mut done = false;
        let mut scrolled_p = false;

        // Backspace and friends are allowed *inside* command sequences, and
        // are executed without disturbing the sequence being assembled.
        if !self.buf.is_empty() && c < b' ' as u32 {
            self.self_insert(c, &mut scrolled_p);
            self.clip_cursor(scrolled_p);
            return;
        }

        self.buf.push(c as u8);
        let idx = self.buf.len();

        if idx >= 253 {
            /* Buffer overflow */
            done = true;
        } else if self.awaiting_st {
            // Some commands swallow every byte until a string terminator, so
            // keep filling the buffer.
        } else if c == ESC {
            /* Starting any command, abandoning whatever was in progress */
            self.buf.clear();
            self.buf.push(c as u8);
        } else if idx == 2 && self.buf[0] as u32 == ESC && (0x20..=0x2F).contains(&c) {
            /* nF escape sequence start */
        } else if idx > 2
            && self.buf[0] as u32 == ESC
            && (0x20..=0x2F).contains(&(self.buf[1] as u32))
            && (0x20..=0x2F).contains(&c)
        {
            /* nF escape sequence continues */
        } else if idx > 2
            && self.buf[0] as u32 == ESC
            && (0x20..=0x2F).contains(&(self.buf[1] as u32))
            && (0x30..=0x7E).contains(&c)
        {
            done = true;
            self.do_nf();
        } else if idx == 2 && self.buf[0] as u32 == ESC && (0x30..=0x3F).contains(&c) {
            done = true;
            self.do_fp(c);
        } else if idx >= 3
            && self.buf[0] as u32 == ESC
            && self.buf[1] == b'['
            && ((0x30..=0x3F).contains(&c) || (0x20..=0x2F).contains(&c))
        {
            /* CSI parameter or intermediate byte */
        } else if idx >= 3
            && self.buf[0] as u32 == ESC
            && self.buf[1] == b'['
            && (0x40..=0x7E).contains(&c)
        {
            done = true;
            self.do_csi(c, &mut scrolled_p);
        } else if idx == 2 && self.buf[0] as u32 == ESC && (0x40..=0x5F).contains(&c) {
            done = true;
            self.do_fe(c, &mut done, &mut scrolled_p);
        } else if idx == 2 && self.buf[0] as u32 == ESC && (0x60..=0x7E).contains(&c) {
            done = true;
            if c == b'c' as u32 {
                /* Full Reset */
                self.reset();
            }
        } else if !self.buf.is_empty() && self.buf[0] as u32 == ESC {
            // An ESC followed by something that matched none of the above.
            // Assume a two-byte sequence, which may not be true.
            done = true;
        }
        // Assemble UTF-8 into one code point.
        else if self.unicrud > 0 {
            self.unicrud -= 1;
            if self.unicrud == 0 {
                let c = utf8_decode(&self.buf);
                done = true;
                self.self_insert(c, &mut scrolled_p);
            }
        } else if c & 0xE0 == 0xC0 {
            self.unicrud = 1;
        } else if c & 0xF0 == 0xE0 {
            self.unicrud = 2;
        } else if c & 0xF8 == 0xF0 {
            self.unicrud = 3;
        } else if c & 0xFC == 0xF8 {
            self.unicrud = 4;
        } else if c & 0xFE == 0xFC {
            self.unicrud = 5;
        } else {
            done = true;
            self.self_insert(c, &mut scrolled_p);
        }

        self.clip_cursor(scrolled_p);
        if done {
            self.buf.clear();
        }
    }

    fn clip_cursor(&mut self, scrolled_p: bool) {
        self.x = self.x.clamp(0, self.width - 1);
        // Scrolling, or an insertion that changed y, clips to the scrolling
        // region. Cursor-positioning commands do not.
        if scrolled_p {
            self.y = self.y.clamp(self.scroll_y1, self.scroll_y2 - 1);
        } else {
            self.y = self.y.clamp(0, self.height - 1);
        }
    }

    /// The control codes, and printing an ordinary character.
    fn self_insert(&mut self, c: u32, scrolled_p: &mut bool) {
        match c {
            0 => {}
            0x07 => { /* BEL */ }
            0x08 => {
                /* BS */
                if self.x > 0 {
                    self.x -= 1;
                }
                self.lcf = false;
            }
            0x09 => {
                // Tabs are motion and do not alter what is under them.
                self.x += 1;
                while self.x < self.width && !self.tabs[self.x as usize] {
                    self.x += 1;
                }
                self.lcf = false;
            }
            0x0A..=0x0C => self.do_lf(scrolled_p),
            0x1A => self.lcf = false,
            0x0D => {
                self.x = 0;
                self.lcf = false;
            }
            0x0E => {
                /* SO: shift to the G1 character set */
                if self.g1 & TTY_SYMBOLS != 0 {
                    self.flags |= TTY_SYMBOLS;
                } else {
                    self.flags &= !TTY_SYMBOLS;
                }
            }
            0x0F => {
                /* SI: shift to the G0 character set */
                if self.g0 & TTY_SYMBOLS != 0 {
                    self.flags |= TTY_SYMBOLS;
                } else {
                    self.flags &= !TTY_SYMBOLS;
                }
            }
            _ if c >= b' ' as u32 => {
                if self.x >= self.width - 1 {
                    self.x = self.width - 1;
                    if !self.lcf {
                        // The character goes in the last column and the cursor
                        // stays on it; only the next one wraps.
                        self.lcf = true;
                    } else {
                        self.lcf = false;
                        if self.auto_wrap_p {
                            self.x = 0;
                            self.y += 1;
                            if self.y >= self.scroll_y2 {
                                let n = self.y - self.scroll_y2 + 1;
                                self.scroll(n);
                                self.y = self.scroll_y2 - 1;
                                *scrolled_p = true;
                            }
                        }
                    }
                }
                let at = (self.y * self.width + self.x) as usize;
                if at < self.grid.len() {
                    self.grid[at] = TtyChar {
                        c,
                        flags: self.flags,
                        fg: self.fg,
                        bg: self.bg,
                    };
                }
                self.x += 1;
            }
            _ => {}
        }
    }

    fn do_lf(&mut self, scrolled_p: &mut bool) {
        if self.linefeed_p {
            self.x = 0;
        }
        self.y += 1;
        self.lcf = false;
        *scrolled_p = true;
        if self.y >= self.scroll_y2 {
            let n = self.y - self.scroll_y2 + 1;
            self.scroll(n);
            self.y = self.scroll_y2 - 1;
        }
    }
}

/// The accumulated bytes of one UTF-8 sequence as a code point.
fn utf8_decode(buf: &[u8]) -> u32 {
    let n = buf.len();
    if n == 0 {
        return 0;
    }
    let (mut c, expect) = match buf[0] {
        b if b & 0xE0 == 0xC0 => (u32::from(b & 0x1F), 1),
        b if b & 0xF0 == 0xE0 => (u32::from(b & 0x0F), 2),
        b if b & 0xF8 == 0xF0 => (u32::from(b & 0x07), 3),
        b if b & 0xFC == 0xF8 => (u32::from(b & 0x03), 4),
        b if b & 0xFE == 0xFC => (u32::from(b & 0x01), 5),
        b => return u32::from(b),
    };
    for b in buf.iter().skip(1).take(expect) {
        c = (c << 6) | u32::from(b & 0x3F);
    }
    c
}

impl Tty {
    /// `ESC <intermediate>... <final>`: character-set designation and a few
    /// oddments. Almost all of these upstream does nothing for.
    fn do_nf(&mut self) {
        match self.buf.len() {
            2 if self.buf[1] == b'c' => self.reset(),
            3 => match self.buf[1] {
                b'#' => {
                    self.lcf = false;
                    if self.buf[2] == b'8' {
                        /* Screen Alignment: fill the screen with E */
                        for ch in self.grid.iter_mut() {
                            ch.c = b'E' as u32;
                        }
                    }
                }
                b'(' | b')' => {
                    // Which character set G0 or G1 stands for. Only the line
                    // drawing set is distinguished; upstream notes that
                    // mapping the others to Unicode would not help, because
                    // neither phosphor nor apple2 can show anything but Latin1.
                    let g0_p = self.buf[1] == b'(';
                    let v = if self.buf[2] == b'0' { TTY_SYMBOLS } else { 0 };
                    if g0_p {
                        self.g0 = v;
                    } else {
                        self.g1 = v;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// `ESC 7` and friends: saving and restoring the cursor.
    fn do_fp(&mut self, c: u32) {
        match c as u8 {
            b'7' => {
                self.saved = Saved {
                    x: self.x,
                    y: self.y,
                    lcf: self.lcf,
                    flags: self.flags,
                };
            }
            b'8' => {
                self.flags = self.saved.flags;
                self.lcf = self.saved.lcf;
                self.x = self.saved.x;
                self.y = self.saved.y;
            }
            _ => {}
        }
    }

    /// `ESC <final>`: the two-byte commands, including the VT52 set and the
    /// ones that swallow everything up to a string terminator.
    fn do_fe(&mut self, c: u32, done: &mut bool, scrolled_p: &mut bool) {
        match c as u8 {
            /* Device Control, System Command, and friends: await ST */
            b'P' | b']' | b'X' | b'^' | b'_' => self.awaiting_st = true,
            b'\\' => self.awaiting_st = false,
            b'[' => {
                /* CSI: not a command on its own */
                *done = false;
            }
            b'A' => self.y -= 1,
            b'B' => {
                self.y += 1;
                *scrolled_p = true;
            }
            b'C' => {
                self.x += 1;
                *scrolled_p = true;
            }
            b'D' => self.do_lf(scrolled_p),
            b'E' => {
                self.x = 0;
                self.do_lf(scrolled_p);
            }
            b'F' => self.flags |= TTY_SYMBOLS,
            b'G' => self.flags &= !TTY_SYMBOLS,
            b'H' => {
                let at = self.x.clamp(0, self.width - 1) as usize;
                self.tabs[at] = true;
            }
            b'I' | b'M' => {
                /* Reverse line feed, scrolling if it runs off the top */
                self.lcf = false;
                self.y -= 1;
                if self.y < self.scroll_y1 {
                    self.y = self.scroll_y1;
                    self.scroll(-1);
                    *scrolled_p = true;
                }
            }
            b'J' => {
                // Upstream passes x as both coordinates here, which erases
                // from (x, x) rather than from (x, y). Kept.
                let (x, y) = (self.x, self.x);
                self.erase(x, y, self.width - 1, self.height - 1);
            }
            b'K' => {
                let (x, y) = (self.x, self.y);
                self.erase(x, y, self.width - 1, y);
            }
            b'Z' => self.replies.push_str("\x1B/Z"),
            _ => {}
        }
    }

    /// `ESC [ <params> <final>`: the bulk of it.
    fn do_csi(&mut self, c: u32, scrolled_p: &mut bool) {
        // Parse the "80;24;365" arguments.
        let mut av = [UNDEF; 32];
        let mut ac = 0usize;
        {
            let mut any = false;
            for i in 2..self.buf.len() - 1 {
                let b = self.buf[i];
                if b == b';' {
                    if ac >= av.len() - 2 {
                        break;
                    }
                    ac += 1;
                    av[ac] = 0;
                    any = true;
                } else if b.is_ascii_digit() {
                    if av[ac] == UNDEF {
                        av[ac] = 0;
                    }
                    av[ac] = av[ac] * 10 + i32::from(b - b'0');
                    any = true;
                }
                // "<=>?" mark a private command, and "!" turns up too.
            }
            if any {
                ac += 1;
            }
        }
        let arg = |i: usize, dflt: i32| {
            if av[i] == UNDEF || av[i] == 0 {
                dflt
            } else {
                av[i]
            }
        };

        match c as u8 {
            b'A' => {
                self.y -= arg(0, 1);
                self.lcf = false;
                *scrolled_p = true;
            }
            b'B' => {
                self.y += arg(0, 1);
                self.lcf = false;
                *scrolled_p = true;
            }
            b'C' => {
                self.x += arg(0, 1);
                self.lcf = false;
                *scrolled_p = true;
            }
            b'D' => {
                self.x -= arg(0, 1);
                self.lcf = false;
                *scrolled_p = true;
            }
            b'E' => {
                self.x = 0;
                self.y += if av[0] == UNDEF { 1 } else { av[0] };
                *scrolled_p = true;
            }
            b'F' => {
                self.x = 0;
                self.y -= if av[0] == UNDEF { 1 } else { av[0] };
                *scrolled_p = true;
            }
            b'G' => self.x = arg(0, 1) - 1,
            b'H' | b'f' => {
                self.y = arg(0, 1) - 1;
                self.x = arg(1, 1) - 1;
                if c as u8 == b'H' && self.origin_relative_p {
                    self.y += self.scroll_y1;
                }
                self.lcf = false;
            }
            b'I' => {
                /* Forward N tab stops */
                for _ in 0..arg(0, 1) {
                    self.x += 1;
                    while self.x < self.width && !self.tabs[self.x as usize] {
                        self.x += 1;
                    }
                }
            }
            b'J' => {
                /* Erase in Display */
                self.lcf = false;
                let (x, y) = (self.x, self.y);
                match av[0] {
                    UNDEF | 0 => self.erase(x, y, self.width - 1, self.height - 1),
                    1 => self.erase(0, 0, x - 1, y),
                    2 | 3 => self.erase(0, 0, self.width - 1, self.height - 1),
                    _ => {}
                }
            }
            b'K' => {
                /* Erase in Line */
                self.lcf = false;
                let (x, y) = (self.x, self.y);
                match av[0] {
                    UNDEF | 0 => self.erase(x, y, self.width - 1, y),
                    1 => self.erase(0, y, x - 1, y),
                    2 | 3 => self.erase(0, y, self.width - 1, y),
                    _ => {}
                }
            }
            b'S' => {
                let n = if av[0] == UNDEF { 1 } else { av[0] };
                self.scroll(n);
                *scrolled_p = true;
            }
            b'T' => {
                let n = if av[0] == UNDEF { 1 } else { av[0] };
                self.scroll(-n);
                *scrolled_p = true;
            }
            b'W' => self.default_tabs(),
            b'Z' => {
                /* Back N tab stops */
                for _ in 0..arg(0, 1) {
                    self.x -= 1;
                    while self.x >= 0 && !self.tabs[self.x as usize] {
                        self.x -= 1;
                    }
                }
            }
            /* Report model: the base one */
            b'c' if av[0] == 0 || av[0] == UNDEF => self.replies.push_str("\x1B[?1;0c"),
            b'g' => {
                let at = self.x.clamp(0, self.width - 1) as usize;
                match if av[0] == UNDEF { 0 } else { av[0] } {
                    0 => self.tabs[at] = false,
                    1 => self.tabs[at] = true,
                    3 => self.tabs.fill(false),
                    _ => {}
                }
            }
            b'h' | b'l' => {
                let on_p = c as u8 == b'h';
                match av[0] {
                    5 => self.inverse_p = on_p,
                    6 => {
                        self.lcf = false;
                        self.origin_relative_p = on_p;
                        self.x = 0;
                        self.y = if on_p { self.scroll_y1 } else { 0 };
                    }
                    7 => {
                        self.auto_wrap_p = on_p;
                        if !on_p {
                            self.lcf = false;
                        }
                    }
                    20 => self.linefeed_p = on_p,
                    3 => self.lcf = false,
                    _ => {}
                }
            }
            b'm' => self.select_graphic_rendition(&av, ac.max(1)),
            b'n' => match av[0] {
                5 => self.replies.push_str("\x1B[0n"),
                6 => {
                    let s = format!("\x1B[{};{}R", self.y + 1, self.x + 1);
                    self.replies.push_str(&s);
                }
                _ => {}
            },
            b'r' => {
                /* Scrolling region: top and bottom lines, 1-based, inclusive */
                self.scroll_y1 = if av[0] == UNDEF { 0 } else { av[0] - 1 };
                self.scroll_y2 = if av[1] == UNDEF { self.height } else { av[1] };
                self.scroll_y1 = self.scroll_y1.clamp(0, self.height - 1);
                self.scroll_y2 = self.scroll_y2.clamp(self.scroll_y1 + 1, self.height);
                self.x = 0;
                self.y = self.scroll_y1;
                self.lcf = false;
            }
            b's' => {
                self.saved = Saved {
                    x: self.x,
                    y: self.y,
                    lcf: self.lcf,
                    flags: self.flags,
                };
            }
            b'u' => {
                self.flags = self.saved.flags;
                self.lcf = self.saved.lcf;
                self.x = self.saved.x;
                self.y = self.saved.y;
            }
            b'y' if av[0] == 2 => match if av[1] == UNDEF { 0 } else { av[1] } {
                0 => self.reset(),
                1 => self.replies.push_str("\x1B[0n"),
                _ => {}
            },
            _ => {}
        }
    }

    /// `CSI ... m`: bold, colours and the rest of the text properties.
    fn select_graphic_rendition(&mut self, av: &[i32; 32], ac: usize) {
        let mut i = 0;
        while i < ac {
            match av[i] {
                UNDEF | 0 => self.flags = 0,
                1 => self.flags |= TTY_BOLD,
                2 => self.flags |= TTY_DIM,
                3 => self.flags |= TTY_ITALIC,
                4 | 21 => self.flags |= TTY_UNDERLINE,
                5 | 6 => self.flags |= TTY_BLINK,
                7 => self.flags |= TTY_INVERSE,
                10 => self.flags &= !TTY_SYMBOLS,
                11 => self.flags |= TTY_SYMBOLS,
                22 => self.flags &= !TTY_DIM,
                23 => self.flags &= !(TTY_ITALIC | TTY_BOLD),
                24 => self.flags &= !TTY_UNDERLINE,
                25 => self.flags &= !TTY_BLINK,
                27 => self.flags &= !TTY_INVERSE,
                n @ 30..=37 => self.set_color(true, n - 30),
                38 | 48 => {
                    // Upstream tests the selector against the code it has
                    // already matched, so neither of these ever fires; the
                    // effect is that a 256-colour or true-colour request stops
                    // the rest of the arguments being read. Kept.
                    i = ac;
                }
                39 => self.set_color(true, 0),
                n @ 40..=47 => self.set_color(false, n - 40),
                49 => self.set_color(false, 0),
                _ => {}
            }
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a string, byte by byte, the way a hack does.
    fn feed(tty: &mut Tty, s: &str) {
        for b in s.bytes() {
            tty.print(u32::from(b));
        }
    }

    /// What is on one row, with never-written cells as spaces.
    fn row(tty: &Tty, y: i32) -> String {
        (0..tty.width)
            .map(|x| {
                let c = tty.grid[(y * tty.width + x) as usize].c;
                char::from_u32(if c == 0 { 32 } else { c }).unwrap_or('?')
            })
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn plain_text_lands_where_it_is_typed() {
        let mut tty = Tty::new(20, 5);
        feed(&mut tty, "hello\r\nworld");
        assert_eq!(row(&tty, 0), "hello");
        assert_eq!(row(&tty, 1), "world");
        assert_eq!((tty.x, tty.y), (5, 1));
    }

    #[test]
    fn the_cursor_can_be_put_anywhere_and_the_screen_erased() {
        let mut tty = Tty::new(20, 5);
        feed(&mut tty, "\x1B[3;5Hhi");
        assert_eq!(row(&tty, 2), "    hi");
        assert_eq!((tty.x, tty.y), (6, 2));

        // Home, then erase to the end of the screen.
        feed(&mut tty, "\x1B[H\x1B[J");
        assert_eq!((tty.x, tty.y), (0, 0));
        assert!(tty.grid.iter().all(|c| c.c == 0), "screen should be clear");
    }

    #[test]
    fn erase_in_line_only_touches_that_line() {
        let mut tty = Tty::new(10, 3);
        feed(&mut tty, "abcdef\r\nghijkl");
        feed(&mut tty, "\x1B[1;4H\x1B[K");
        assert_eq!(row(&tty, 0), "abc");
        assert_eq!(row(&tty, 1), "ghijkl");
    }

    /// The Last Column Flag: a character in the last column leaves the cursor
    /// on it, and only the character *after* that wraps.
    #[test]
    fn the_last_column_does_not_wrap_until_the_next_character() {
        let mut tty = Tty::new(5, 3);
        feed(&mut tty, "abcde");
        assert_eq!(row(&tty, 0), "abcde");
        assert_eq!((tty.x, tty.y), (4, 0), "cursor sits on the last column");
        feed(&mut tty, "f");
        assert_eq!(row(&tty, 1), "f");
        assert_eq!((tty.x, tty.y), (1, 1));

        // With auto-wrap off it overstrikes the last column instead.
        let mut tty = Tty::new(5, 3);
        feed(&mut tty, "\x1B[?7l");
        feed(&mut tty, "abcdefg");
        assert_eq!(row(&tty, 0), "abcdg");
        assert_eq!(row(&tty, 1), "");
    }

    #[test]
    fn the_screen_scrolls_when_it_runs_off_the_bottom() {
        let mut tty = Tty::new(10, 3);
        feed(&mut tty, "one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(row(&tty, 0), "two");
        assert_eq!(row(&tty, 1), "three");
        assert_eq!(row(&tty, 2), "four");
        assert_eq!(tty.y, 2, "cursor stays on the bottom line");
    }

    /// A scrolling region confines the scroll to part of the screen, which is
    /// how a full-screen program keeps a status line still.
    #[test]
    fn a_scrolling_region_leaves_the_rest_alone() {
        let mut tty = Tty::new(10, 5);
        feed(&mut tty, "top\r\n\x1B[2;4r");
        feed(&mut tty, "a\r\nb\r\nc\r\nd");
        assert_eq!(row(&tty, 0), "top", "outside the region");
        assert_eq!(row(&tty, 1), "b");
        assert_eq!(row(&tty, 2), "c");
        assert_eq!(row(&tty, 3), "d");
    }

    #[test]
    fn text_properties_and_the_line_drawing_set_are_remembered() {
        let mut tty = Tty::new(10, 3);
        feed(&mut tty, "\x1B[1;7mX\x1B[mY");
        assert_eq!(tty.grid[0].flags, TTY_BOLD | TTY_INVERSE);
        assert_eq!(tty.grid[1].flags, 0);

        // What terminfo's `enacs` does: G0 is ASCII, G1 is line drawing. Then
        // SO shifts into G1 and SI back to G0.
        feed(&mut tty, "\x1B(B\x1B)0");
        feed(&mut tty, "\x0EZ");
        assert_eq!(tty.grid[2].flags, TTY_SYMBOLS);
        feed(&mut tty, "\x0FW");
        assert_eq!(tty.grid[3].flags, 0);
        assert_eq!(GRAPHICS_UNICODE[b'q' as usize], 0x2500, "─");
    }

    #[test]
    fn colours_come_from_the_vga_palette() {
        let mut tty = Tty::new(10, 3);
        feed(&mut tty, "\x1B[31mR\x1B[42mG");
        assert_eq!(tty.grid[0].fg, CMAP[1], "red");
        assert_eq!(tty.grid[1].bg, CMAP[2], "green");
    }

    #[test]
    fn tabs_move_without_disturbing_what_is_under_them() {
        let mut tty = Tty::new(20, 3);
        feed(&mut tty, "ab\tc");
        assert_eq!(row(&tty, 0), "ab      c");
        feed(&mut tty, "\x1B[1;1H\x1B[2I");
        assert_eq!(tty.x, 16);
    }

    /// The terminal answers some questions, and the hack passes what it says
    /// back to whatever is feeding it.
    #[test]
    fn it_reports_where_the_cursor_is() {
        let mut tty = Tty::new(20, 5);
        feed(&mut tty, "\x1B[3;7H\x1B[6n");
        assert_eq!(tty.replies, "\x1B[3;7R");
        tty.replies.clear();
        feed(&mut tty, "\x1B[5n");
        assert_eq!(tty.replies, "\x1B[0n");
    }

    #[test]
    fn saving_and_restoring_the_cursor_round_trips() {
        let mut tty = Tty::new(20, 5);
        feed(&mut tty, "\x1B[2;3H\x1B7\x1B[5;9H\x1B8");
        assert_eq!((tty.x, tty.y), (2, 1));
    }

    /// Backspace inside an escape sequence is executed, and the sequence
    /// carries on assembling around it.
    #[test]
    fn a_control_code_inside_a_sequence_does_not_break_it() {
        let mut tty = Tty::new(20, 5);
        feed(&mut tty, "abc");
        feed(&mut tty, "\x1B[\x081;1H");
        assert_eq!((tty.x, tty.y), (0, 0), "the position command still ran");
        assert_eq!(row(&tty, 0), "abc");
    }

    #[test]
    fn multi_byte_utf8_becomes_one_cell() {
        let mut tty = Tty::new(10, 3);
        feed(&mut tty, "a\u{2500}b");
        assert_eq!(tty.grid[0].c, b'a' as u32);
        assert_eq!(tty.grid[1].c, 0x2500, "one cell, not three");
        assert_eq!(tty.grid[2].c, b'b' as u32);
    }

    #[test]
    fn a_resize_keeps_what_still_fits() {
        let mut tty = Tty::new(10, 4);
        feed(&mut tty, "hello\r\nthere");
        tty.resize(6, 3);
        assert_eq!(row(&tty, 0), "hello");
        assert_eq!(row(&tty, 1), "there");
        assert_eq!(tty.grid.len(), 18);
        tty.resize(20, 6);
        assert_eq!(row(&tty, 0), "hello");
    }

    /// Whatever it is fed, it never panics and never leaves the cursor off the
    /// screen. The text sources these hacks read are not always well behaved.
    #[test]
    fn any_byte_stream_at_all_is_survivable() {
        let mut tty = Tty::new(12, 5);
        let mut seed = 12345u32;
        for _ in 0..200_000 {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            tty.print((seed >> 16) & 0xFF);
            assert!((0..tty.width).contains(&tty.x), "x off screen: {}", tty.x);
            assert!((0..tty.height).contains(&tty.y), "y off screen: {}", tty.y);
        }
    }
}
