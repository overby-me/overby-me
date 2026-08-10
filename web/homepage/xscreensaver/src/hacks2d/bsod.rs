//! Port of `hacks/bsod.c`.
//!
//! ```text
//! xscreensaver, Copyright © 1998-2026 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Blue Screen of Death: the finest in personal computer emulation.
//! Concept cribbed from Stephen Martin <smartin@mks.com>;
//! this version written by jwz, 4-Jun-98.
//! Mostly rewritten by jwz, 20-Feb-2006.
//! ```
//!
//! Every mode is a little program for the machine below: a queue of commands
//! that print text at a delay per character and per line, move the cursor,
//! change colour, draw a box, blink a cursor, jump backwards. Nothing about a
//! mode is a special case in the drawing code, which is what lets thirty-nine
//! computers crash in thirty-nine different ways out of one page of machinery.
//!
//! A run lasts `delay` seconds, then another machine crashes.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, parse_color};
use crate::runtime::font::Font;
use crate::runtime::{
    About, Dpy, Fb, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent, frand,
    png, random, screenhack_event_helper,
};

/// Where a line of text is placed between the margins.
///
/// Upstream has a right-justified case too, which no machine uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Center,
}

/// One instruction of a mode.
///
/// Upstream keeps these in a struct of a type tag and six `void *`, which is
/// how a C program writes a variant type.
enum Ev {
    /// Print a string. `full` clears the whole line, margin to margin, before
    /// the first character of it lands.
    Text {
        align: Align,
        full: bool,
        s: String,
        /// How far through the string this event has got, in bytes.
        at: usize,
        /// Whether the position has been worked out yet, which happens once.
        started: bool,
    },
    Color(Pixel, Pixel),
    Invert,
    MoveTo(i32, i32),
    Margins(i32, i32),
    VertMargins(i32, i32),
    /// A blinking cursor: half a cycle in microseconds, and how many times.
    /// `block` is the inverted-space kind, otherwise it is an underscore.
    Cursor {
        block: bool,
        usec: i64,
        count: i64,
    },
    Rect {
        fill: bool,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    Line {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        thick: i32,
    },
    /// Copy a rectangle of the window to the window.
    Copy {
        srcx: i32,
        srcy: i32,
        w: i32,
        h: i32,
        tox: i32,
        toy: i32,
    },
    /// The same, from the mode's own image.
    Pixmap {
        srcx: i32,
        srcy: i32,
        w: i32,
        h: i32,
        tox: i32,
        toy: i32,
    },
    /// Put a photograph on the screen.
    Img,
    /// Switch between fonts A, B and C.
    Font(usize),
    Pause(i64),
    CharDelay(i64),
    LineDelay(i64),
    /// Jump this far in the queue, which is how a mode repeats itself.
    Loop(i32),
    /// Start again from the top, with every string rewound.
    Reset,
    Wrap,
    WordWrap,
    Truncate,
    Eof,
}

/// `~0`, which a mode passes as a destination to mean "where the text is".
const HERE: i32 = -1;

/// The machine a mode runs on.
struct Bst {
    font: Font,
    font_a: Font,
    font_b: Font,
    font_c: Font,
    fg: Pixel,
    bg: Pixel,
    gc: Gc,

    /// For text wrapping.
    left_margin: i32,
    right_margin: i32,
    /// For text scrolling and cropping.
    top_margin: i32,
    bottom_margin: i32,
    xoff: i32,
    yoff: i32,

    wrap_p: bool,
    word_wrap_p: bool,
    word_buf: String,
    scroll_p: bool,
    /// If set, chops off extra text vertically.
    crop_p: bool,

    /// Source image used by [`Ev::Pixmap`], and which of its pixels are opaque.
    pixmap: Option<Fb>,
    mask: Option<std::rc::Rc<Fb>>,

    /// Current text-drawing position.
    x: i32,
    y: i32,
    current_left: i32,
    last_nonwhite: i32,

    /// Position in the queue, or `None` once the mode has run off the end.
    pos: Option<usize>,
    queue: Vec<Ev>,

    /// Delay between printing characters, and between printing lines.
    char_delay: i64,
    line_delay: i64,

    macx_eol_kludge: bool,

    width: i32,
    height: i32,
}

impl Bst {
    fn new(d: &Dpy, fg: Pixel, bg: Pixel, f: &Fonts) -> Bst {
        let mut bst = Bst {
            font: f.a,
            font_a: f.a,
            font_b: f.b,
            font_c: f.c,
            fg,
            bg,
            gc: Gc::new(fg, bg),
            left_margin: 10,
            right_margin: 10,
            top_margin: 0,
            bottom_margin: 0,
            xoff: 0,
            yoff: 0,
            wrap_p: false,
            word_wrap_p: false,
            word_buf: String::new(),
            scroll_p: false,
            crop_p: false,
            pixmap: None,
            mask: None,
            x: 0,
            y: 0,
            current_left: 0,
            last_nonwhite: 0,
            pos: Some(0),
            queue: Vec::new(),
            char_delay: 0,
            line_delay: 0,
            macx_eol_kludge: false,
            width: d.width(),
            height: d.height(),
        };
        bst.x = bst.left_margin + bst.xoff;
        bst.y = bst.font.ascent() + bst.left_margin + bst.yoff;
        bst
    }

    fn line_height(&self) -> i32 {
        self.font.ascent() + self.font.descent()
    }

    /// `XClearWindow`, which fills the window with its background colour.
    ///
    /// Where a mode does this matters: the screen still holds whatever the last
    /// machine was showing when it stopped, and a mode that draws its picture
    /// before clearing would wipe it out again.
    fn clear(&self, d: &mut Dpy) {
        let bg = self.bg;
        d.win().clear(bg);
    }

    /* -- building a mode ------------------------------------------------ */

    fn text(&mut self, align: Align, s: &str) {
        self.queue.push(Ev::Text {
            align,
            full: false,
            s: s.to_string(),
            at: 0,
            started: false,
        });
    }

    fn text_full(&mut self, align: Align, s: &str) {
        self.queue.push(Ev::Text {
            align,
            full: true,
            s: s.to_string(),
            at: 0,
            started: false,
        });
    }

    fn invert(&mut self) {
        self.queue.push(Ev::Invert);
    }

    fn color(&mut self, fg: Pixel, bg: Pixel) {
        self.queue.push(Ev::Color(fg, bg));
    }

    fn moveto(&mut self, x: i32, y: i32) {
        self.queue.push(Ev::MoveTo(x, y));
    }

    fn pause(&mut self, usec: i64) {
        self.queue.push(Ev::Pause(usec));
    }

    fn char_delay(&mut self, usec: i64) {
        self.queue.push(Ev::CharDelay(usec));
    }

    fn line_delay(&mut self, usec: i64) {
        self.queue.push(Ev::LineDelay(usec));
    }

    fn margins(&mut self, left: i32, right: i32) {
        self.queue.push(Ev::Margins(left, right));
    }

    fn vert_margins(&mut self, top: i32, bottom: i32) {
        self.queue.push(Ev::VertMargins(top, bottom));
    }

    fn cursor(&mut self, block: bool, usec: i64, count: i64) {
        self.queue.push(Ev::Cursor { block, usec, count });
    }

    fn rect(&mut self, fill: bool, x: i32, y: i32, w: i32, h: i32) {
        self.queue.push(Ev::Rect { fill, x, y, w, h });
    }

    fn line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, thick: i32) {
        self.queue.push(Ev::Line {
            x1,
            y1,
            x2,
            y2,
            thick,
        });
    }

    fn copy(&mut self, srcx: i32, srcy: i32, w: i32, h: i32, tox: i32, toy: i32) {
        self.queue.push(Ev::Copy {
            srcx,
            srcy,
            w,
            h,
            tox,
            toy,
        });
    }

    fn pixmap_at(&mut self, srcx: i32, srcy: i32, w: i32, h: i32, tox: i32, toy: i32) {
        self.queue.push(Ev::Pixmap {
            srcx,
            srcy,
            w,
            h,
            tox,
            toy,
        });
    }

    fn img(&mut self) {
        self.queue.push(Ev::Img);
    }

    fn set_font(&mut self, n: usize) {
        self.queue.push(Ev::Font(n));
    }

    fn loop_back(&mut self, off: i32) {
        self.queue.push(Ev::Loop(off));
    }

    fn reset(&mut self) {
        self.queue.push(Ev::Reset);
    }

    fn wrap(&mut self) {
        self.queue.push(Ev::Wrap);
    }

    fn word_wrap(&mut self) {
        self.queue.push(Ev::WordWrap);
    }

    fn truncate(&mut self) {
        self.queue.push(Ev::Truncate);
    }

    /* -- running it ----------------------------------------------------- */

    /// Where the next string starts, given how it is to be aligned. Centring
    /// and right-justifying measure every line of it and use the widest.
    fn position_for_text(&mut self, align: Align, s: &str) {
        let mut max_width = 0;
        if align != Align::Left {
            for line in s.split(['\r', '\n']) {
                max_width = max_width.max(self.font.text_width(line));
            }
        }

        match align {
            Align::Left => {
                self.current_left = self.left_margin + self.xoff;
            }
            Align::Center => {
                let w = (self.width - self.left_margin - self.right_margin - max_width).max(0);
                self.x = self.left_margin + self.xoff + (w / 2);
                self.current_left = self.x;
            }
        }
    }

    /// Move to the start of the next line, scrolling the window up if this one
    /// is the last and the mode asked to scroll.
    fn crlf(&mut self, d: &mut Dpy) {
        let lh = self.line_height();
        self.x = self.current_left;
        if !self.scroll_p || self.y + lh < self.height - self.bottom_margin - self.yoff {
            self.y += lh;
        } else {
            let w = self.width - self.right_margin - self.left_margin;
            let h = self.height - self.top_margin - self.bottom_margin;
            let (sx, sy) = (self.left_margin + self.xoff, self.top_margin + self.yoff);
            let win = d.win();
            win.copy_area_self(&self.gc, sx, sy + lh, w, h - lh, sx, sy);
            let bg = self.bg;
            win.clear_area(bg, sx, sy + h - lh, w, lh);
        }
    }

    fn swap_colors(&mut self) {
        std::mem::swap(&mut self.fg, &mut self.bg);
        self.gc.set_foreground(self.fg);
        self.gc.set_background(self.bg);
    }

    /// Draw one character at the text position, and advance it.
    fn draw_char(&mut self, d: &mut Dpy, c: char) {
        if self.crop_p && self.y >= self.height - self.bottom_margin - self.yoff {
            /* reached the bottom of the drawing area, and crop_p = True */
            return;
        }

        if c == '\r' {
            self.x = self.current_left;
            self.last_nonwhite = self.x;
            return;
        }

        if c == '\n' {
            if self.macx_eol_kludge {
                /* Special case for the weird way OSX crashes print newlines... */
                let (x, y) = (self.x, self.y);
                let (w, h) = (self.font.char_width(), self.line_height() * 2);
                let (bg, a) = (self.bg, self.font.ascent());
                self.gc.set_foreground(bg);
                d.win().fill_rectangle(&self.gc, x, y - a, w, h);
                let fg = self.fg;
                self.gc.set_foreground(fg);
            }
            self.crlf(d);
            return;
        }

        if c == '\u{8}' {
            /* backspace -- assumes fixed width */
            self.x -= self.font.char_width();
            self.x = self.x.max(self.left_margin + self.xoff);
            return;
        }

        /* We render the character ESC as an inverted space (block cursor). */
        let cursorp = c == '\u{1b}';
        let c = if cursorp { ' ' } else { c };

        let cw = self.font.char_width();
        if self.x < self.left_margin {
            self.x = self.left_margin;
        }

        if (self.wrap_p || self.word_wrap_p)
            && self.x + cw > self.width - self.right_margin - self.xoff
        {
            let word: String = if self.word_wrap_p {
                std::mem::take(&mut self.word_buf)
            } else {
                String::new()
            };
            let ww = self.font.text_width(&word);

            if !word.is_empty() {
                /* Erase the truncated wrapped word */
                let (x, y) = (self.last_nonwhite, self.y - self.font.ascent());
                let (bg, h) = (self.bg, self.line_height());
                self.gc.set_foreground(bg);
                d.win().fill_rectangle(&self.gc, x, y, ww, h);
                let fg = self.fg;
                self.gc.set_foreground(fg);
            }

            self.crlf(d);

            if !word.is_empty() {
                /* Draw wrapped partial word on the next line, no delay */
                let (x, y) = (self.x, self.y);
                let (bg, a, h) = (self.bg, self.font.ascent(), self.line_height());
                self.gc.set_foreground(bg);
                d.win().fill_rectangle(&self.gc, x, y - a, ww, h);
                let fg = self.fg;
                self.gc.set_foreground(fg);
                let font = self.font;
                d.win().draw_string(&self.gc, &font, x, y, &word);
                self.x += ww;
                self.last_nonwhite = self.x;
            }
            self.word_buf = word;
        }

        if cursorp {
            self.swap_colors();
        }

        let (x, y) = (self.x, self.y);
        let (a, h) = (self.font.ascent(), self.line_height());
        let bg = self.bg;
        self.gc.set_foreground(bg);
        d.win().fill_rectangle(&self.gc, x, y - a, cw, h);
        let fg = self.fg;
        self.gc.set_foreground(fg);
        let font = self.font;
        d.win()
            .draw_string(&self.gc, &font, x, y, c.encode_utf8(&mut [0; 4]));

        if cursorp {
            self.swap_colors();
        }

        self.x += cw;

        if self.word_wrap_p {
            if c == ' ' || c == '\t' {
                self.word_buf.clear();
                self.last_nonwhite = self.x;
            } else {
                self.word_buf.push(c);
            }
        }
    }

    /// Run one instruction. Returns how long to wait afterwards, or `None`
    /// when the mode has finished.
    fn pop(&mut self, d: &mut Dpy) -> Option<i64> {
        let pos = self.pos?;

        // Take the event out of the queue so the machine can be handed to the
        // drawing code, and put it back afterwards.
        let mut ev = std::mem::replace(&mut self.queue[pos], Ev::Eof);
        let delay = self.run(d, &mut ev);
        self.queue[pos] = ev;
        delay
    }

    fn run(&mut self, d: &mut Dpy, ev: &mut Ev) -> Option<i64> {
        let pos = self.pos?;
        match ev {
            Ev::Text {
                align,
                full,
                s,
                at,
                started,
            } => {
                if *at >= s.len() {
                    /* Reset the string back to the beginning, in case we loop. */
                    *at = 0;
                    *started = false;
                    self.pos = Some(pos + 1);
                    self.current_left = self.left_margin + self.xoff;
                    return Some(self.line_delay);
                }

                if !*started {
                    *started = true;
                    self.position_for_text(*align, s);
                    self.word_buf.clear();
                    self.last_nonwhite = self.x;

                    if *full {
                        let (bg, fg) = (self.bg, self.fg);
                        let (y, w, h) =
                            (self.y - self.font.ascent(), self.width, self.line_height());
                        self.gc.set_foreground(bg);
                        d.win().fill_rectangle(&self.gc, 0, y, w, h);
                        self.gc.set_foreground(fg);
                    }
                }

                let c = s[*at..].chars().next().unwrap_or(' ');
                let delay = if c == '\r' || c == '\n' {
                    self.line_delay
                } else {
                    self.char_delay
                };
                self.draw_char(d, c);
                *at += c.len_utf8();
                Some(delay)
            }

            Ev::Invert => {
                self.swap_colors();
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Color(fg, bg) => {
                self.fg = *fg;
                self.bg = *bg;
                self.gc.set_foreground(self.fg);
                self.gc.set_background(self.bg);
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::MoveTo(x, y) => {
                self.x = *x;
                self.y = *y;
                self.word_buf.clear();
                self.last_nonwhite = self.x;
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Rect { fill, x, y, w, h } => {
                if *fill {
                    d.win().fill_rectangle(&self.gc, *x, *y, *w, *h);
                } else {
                    d.win().draw_rectangle(&self.gc, *x, *y, *w, *h);
                }
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Line {
                x1,
                y1,
                x2,
                y2,
                thick,
            } => {
                self.gc.set_line_width(*thick);
                d.win().draw_line(&self.gc, *x1, *y1, *x2, *y2);
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Copy {
                srcx,
                srcy,
                w,
                h,
                tox,
                toy,
            } => {
                let (tox, toy) = self.destination(*tox, *toy);
                d.win()
                    .copy_area_self(&self.gc, *srcx, *srcy, *w, *h, tox, toy);
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Pixmap {
                srcx,
                srcy,
                w,
                h,
                tox,
                toy,
            } => {
                let (tox, toy) = self.destination(*tox, *toy);
                if let Some(p) = self.pixmap.take() {
                    if let Some(m) = self.mask.clone() {
                        self.gc.set_clip_mask(m);
                        self.gc.set_clip_origin(tox, toy);
                    }
                    d.win()
                        .copy_area(&self.gc, &p, *srcx, *srcy, *w, *h, tox, toy);
                    self.gc.set_clip_none();
                    self.pixmap = Some(p);
                }
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Img => {
                // Upstream starts an asynchronous load here and the driver
                // waits for it. Ours draws whatever the host has, or the test
                // card, and is done with it.
                let _ = d.load_image_async_simple(None);
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Font(n) => {
                self.font = match n {
                    0 => self.font_a,
                    1 => self.font_b,
                    _ => self.font_c,
                };
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Pause(usec) => {
                let usec = *usec;
                self.pos = Some(pos + 1);
                Some(usec)
            }

            Ev::CharDelay(usec) => {
                self.char_delay = *usec;
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::LineDelay(usec) => {
                self.line_delay = *usec;
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Margins(l, r) => {
                self.left_margin = *l;
                self.right_margin = *r;
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::VertMargins(t, b) => {
                self.top_margin = *t;
                self.bottom_margin = *b;
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Cursor { block, usec, count } => {
                let ox = self.x;

                if *block {
                    self.swap_colors();
                    let (x, y) = (self.x, self.y - self.font.ascent());
                    let (w, h) = (self.font.char_width(), self.line_height());
                    d.win().fill_rectangle(&self.gc, x, y, w, h);
                    self.draw_char(d, ' ');
                } else {
                    self.draw_char(d, if *count & 1 != 0 { ' ' } else { '_' });
                    self.draw_char(d, ' ');
                }

                self.x = ox;

                *count -= 1;
                if *count <= 0 {
                    self.pos = Some(pos + 1);
                }
                Some(*usec)
            }

            Ev::Wrap => {
                self.wrap_p = true;
                self.word_wrap_p = false;
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::WordWrap => {
                self.wrap_p = false;
                self.word_wrap_p = true;
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Truncate => {
                self.wrap_p = false;
                self.word_wrap_p = false;
                self.pos = Some(pos + 1);
                Some(0)
            }

            Ev::Loop(off) => {
                let next = pos as i32 + *off;
                if next < 0 || next as usize >= self.queue.len() {
                    // Upstream aborts; there is nowhere to jump to, so stop.
                    self.pos = None;
                    return None;
                }
                self.pos = Some(next as usize);
                Some(0)
            }

            Ev::Reset => {
                for e in self.queue.iter_mut() {
                    if let Ev::Text { at, started, .. } = e {
                        *at = 0;
                        *started = false;
                    }
                }
                self.pos = Some(0);
                Some(0)
            }

            Ev::Eof => {
                self.pos = None;
                None
            }
        }
    }

    /// A destination of [`HERE`] means the current text position.
    fn destination(&self, tox: i32, toy: i32) -> (i32, i32) {
        (
            if tox == HERE { self.x } else { tox },
            if toy == HERE {
                self.y - self.font.ascent()
            } else {
                toy
            },
        )
    }
}

/* ---------------------------------------------------------------- modes */

/// Every machine that can crash, in the order upstream lists them.
#[derive(Clone, Copy)]
struct Mode {
    name: &'static str,
    fun: fn(&mut Dpy, &Fonts) -> Bst,
    /// What upstream's `bsod_defaults` gives this mode for `<name>.font`,
    /// `.bigFont`, `.fontB` and `.fontC`. Empty means it does not name one, and
    /// falls back to the collection's own default, or for the second pair to
    /// whichever font the mode is already using.
    fonts: [&'static str; 4],
}

/// The three fonts a mode can switch between with [`Ev::Font`].
struct Fonts {
    a: Font,
    b: Font,
    c: Font,
}

/// The font every mode gets if it does not name one.
const DEFAULT_FONT: &str = "Classic Console 12, Courier Bold 12";
const DEFAULT_BIG_FONT: &str = "Classic Console 24, Courier Bold 24";

impl Fonts {
    /// Resolve a mode's fonts. The window is small or it is big, and that picks
    /// between the first two.
    fn resolve(m: &Mode, height: i32) -> Fonts {
        let (small, big) = (m.fonts[0], m.fonts[1]);
        let spec = if height < 640 {
            if small.is_empty() {
                DEFAULT_FONT
            } else {
                small
            }
        } else if big.is_empty() {
            DEFAULT_BIG_FONT
        } else {
            big
        };
        let a = Font::load(spec);
        Fonts {
            a,
            b: if m.fonts[2].is_empty() {
                a
            } else {
                Font::load(m.fonts[2])
            },
            c: if m.fonts[3].is_empty() {
                a
            } else {
                Font::load(m.fonts[3])
            },
        }
    }
}

fn color(spec: &str, fallback: Pixel) -> Pixel {
    parse_color(spec).unwrap_or(fallback)
}

const WHITE: Pixel = crate::runtime::color::WHITE;
const BLACK: Pixel = crate::runtime::color::BLACK;

/// Windows 3.1 and 95, which crashed in five different ways between them.
fn windows_31(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(
        d,
        WHITE,
        color("#0000AA", BLACK), /* EGA color 0x01 */
        f,
    );
    let lines = 9;

    bst.xoff = 0;
    bst.left_margin = 0;
    bst.right_margin = 0;

    match random() % 8 {
        0..=2 => {
            /* Windows 3.1 */
            bst.invert();
            bst.text(Align::Center, "Windows\n");
            bst.invert();
            bst.text(
                Align::Center,
                "A fatal exception 0E has occurred at F0AD:42494C4C\n\
                 the current application will be terminated.\n\
                 \n\
                 * Press any key to terminate the current application.\n\
                 * Press CTRL+ALT+DELETE again to restart your computer.\n\
                 \x20 You will lose any unsaved information in all applications.\n\
                 \n\
                 \n",
            );
            bst.text(Align::Center, "Press any key to continue ");
            bst.cursor(false, 120_000, 999_999);
        }

        3 | 4 => {
            /* Windows 3.1 */
            bst.invert();
            bst.text(Align::Center, "NETSCAPE.EXE\n");
            bst.invert();
            bst.text(
                Align::Center,
                "   This windows application has stopped responding to the system.\n\
                 \n\
                 *  Press ESC to cancel and return to Windows.\n\
                 *  Press ENTER to close this application that is not responding.\n\
                 \x20  You will lose any unsaved information in this application.\n\
                 *  Press CTRL+ALT+DEL again to restart your computer. You will\n\
                 \x20  lose any unsaved information in all applications.\n\
                 \n",
            );
            bst.text(
                Align::Center,
                "Press ENTER for OK or ESC to Cancel: OK\u{8}\u{8}",
            );
            bst.cursor(false, 120_000, 999_999);
        }

        5 => {
            /* Windows 95 */
            bst.invert();
            bst.text(Align::Center, "Windows\n");
            bst.invert();
            bst.text(
                Align::Center,
                "An exception 00 has occurred at 0028:C18580AE in VxD HSFLOP(03) +\n\
                 0000156E.  This was called from 0028:C1858AED in VxD HSFLOP(03) +\n\
                 0000F0AD.  It may be possible to continue normally.\n\
                 \n\
                 *  Press any key to attempt to continue.\n\
                 *  Press CTRL+ALT+DEL to restart your computer.  You will\n\
                 \x20  lose any unsaved information in all applications.\n\
                 \n",
            );
            bst.text(Align::Center, "Press any key to continue ");
            bst.cursor(false, 120_000, 999_999);
        }

        6 => {
            /* Windows 95 */
            bst.invert();
            bst.text(Align::Center, "Windows\n");
            bst.invert();
            bst.text(
                Align::Center,
                "A fatal exception 0E has occurred at F0AD:011747F3.  The current\n\
                 application will be terminated.\n\
                 \n\
                 *  Press any key to terminate the current application.\n\
                 *  Press CTRL+ALT+DEL again to restart your computer. You will\n\
                 \x20  lose any unsaved information in all applications.\n\
                 \n",
            );
            bst.text(Align::Center, "Press any key to continue ");
            bst.cursor(false, 120_000, 999_999);
        }

        _ => {
            /* Windows 95 */
            bst.invert();
            bst.text(Align::Center, "WARNING!\n");
            bst.invert();
            bst.text(
                Align::Center,
                "The system is either busy or has become unstable. You can wait and\n\
                 see if it becomes available again, or you can restart your computer.\n\
                 \n\
                 *  Press any key to return to Windows and wait.\n\
                 *  Press CTRL+ALT+DEL again to restart your computer. You will\n\
                 \x20  lose unsaved information in any programs that are running.\n\
                 \n",
            );
            bst.text(Align::Center, "Press any key to continue ");
            bst.cursor(false, 120_000, 999_999);
        }
    }

    bst.y = (bst.height - bst.yoff - bst.line_height() * lines) / 2;
    bst.clear(d);
    bst
}

/// VMware ESX Server, dumping core to disk with a countdown that stalls.
fn vmware(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#a700a8", BLACK), f);
    let fg = bst.fg;
    let bg = bst.bg;
    let fg2 = color("Yellow", WHITE);

    bst.color(fg2, bg);
    bst.text(Align::Left, "VMware ESX Server [Releasebuild-98103]\n");
    bst.color(fg, bg);
    bst.text(
        Align::Left,
        "PCPU 1 locked up. Failed to ack TLB invalidate.\n\
         frame=0x3a37d98 ip=0x625e94 cr2=0x0 cr3=0x40c66000 cr4=0x16c\n\
         es=0xffffffff ds=0xffffffff fs=0xffffffff gs=0xffffffff\n\
         eax=0xffffffff ebx=0xffffffff ecx=0xffffffff edx=0xffffffff\n\
         ebp=0x3a37ef4 esi=0xffffffff edi=0xffffffff err=-1 eflags=0xffffffff\n\
         *0:1037/helper1-4 1:1107/vmm0:Fagi 2:1121/vmware-vm 3:1122/mks:Franc\n\
         0x3a37ef4:[0x625e94]Panic+0x17 stack: 0x833ab4, 0x3a37f10, 0x3a37f48\n\
         0x3a37f04:[0x625e94]Panic+0x17 stack: 0x833ab4, 0x1, 0x14a03a0\n\
         0x3a37f48:[0x64bfa4]TLBDoInvalidate+0x38f stack: 0x3a37f54, 0x40, 0x2\n\
         0x3a37f70:[0x66da4d]XMapForceFlush+0x64 stack: 0x0, 0x4d3a, 0x0\n\
         0x3a37fac:[0x652b8b]helpFunc+0x2d2 stack: 0x1, 0x14a4580, 0x0\n\
         0x3a37ffc:[0x750902]CpuSched_StartWorld+0x109 stack: 0x0, 0x0, 0x0\n\
         0x3a38000:[0x0]blk_dev+0xfd76461f stack: 0x0, 0x0, 0x0\n\
         VMK uptime: 7:05:43:45.014 TSC: 1751259712918392\n\
         Starting coredump to disk\n",
    );
    bst.char_delay(10_000);
    bst.text(Align::Left, "using slot 1 of 1... ");
    bst.char_delay(300_000);
    bst.text(Align::Left, "9876");
    bst.char_delay(3_000_000);
    bst.text(Align::Left, "66665");
    bst.char_delay(100_000);
    bst.text(Align::Left, "4321");
    bst.char_delay(0);
    bst.text(
        Align::Left,
        "Disk dump successful.\n\
         Waiting for Debugger (world 1037)\n\
         Debugger is listening on serial port ...\n",
    );
    bst.char_delay(10_000);
    bst.text(Align::Left, "Press Escape to enter local debugger\n");
    bst.char_delay(10_000);
    bst.text(
        Align::Left,
        "Remote debugger activated. Local debugger no longer available.\n",
    );
    bst.clear(d);
    bst
}

/// As seen in Portal 2. By jwz.
fn glados(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#0000AA", BLACK), f);
    const PANICSTR: &[&str] = &[
        "\n",
        "MOLTEN CORE WARNING\n",
        "\n",
        "An operator error exception has occurred at FISSREAC0020093:09\n",
        "FISSREAC0020077:14 FISSREAC0020023:17 FISSREAC0020088:22\n",
        "neutron multiplication rate at spikevalue 99999999\n",
        "\n",
        "* Press any key to vent radiological emissions into atmosphere.\n",
        "* Consult reactor core manual for instructions on proper reactor core\n",
        "maintenance and repair.\n",
        "\n",
        "Press any key to continue\n",
    ];

    bst.xoff = 0;
    bst.left_margin = 0;
    bst.right_margin = 0;

    bst.y = (bst.height - bst.yoff - bst.line_height() * PANICSTR.len() as i32) / 2;

    let y = bst.y;
    bst.moveto(0, y);
    bst.invert();
    bst.text(Align::Center, "OPERATOR ERROR\n");
    bst.invert();
    for s in PANICSTR {
        bst.text(Align::Center, s);
    }
    bst.pause(1_000_000);
    bst.invert();
    let (w, h) = (bst.width, bst.height);
    bst.rect(true, 0, 0, w, h);
    bst.invert();
    bst.pause(250_000);
    bst.reset();
    bst.clear(d);
    bst
}

/// SCO OpenServer 5 panic, by Tom Kelly.
fn sco(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);

    bst.text(
        Align::Left,
        "Unexpected trap in kernel mode:\n\
         \n\
         cr0 0x80010013     cr2  0x00000014     cr3 0x00000000  tlb  0x00000000\n\
         ss  0x00071054    uesp  0x00012055     efl 0x00080888  ipl  0x00000005\n\
         cs  0x00092585     eip  0x00544a4b     err 0x004d4a47  trap 0x0000000E\n\
         eax 0x0045474b     ecx  0x0042544b     edx 0x57687920  ebx  0x61726520\n\
         esp 0x796f7520     ebp  0x72656164     esi 0x696e6720  edi  0x74686973\n\
         ds  0x3f000000     es   0x43494c48     fs  0x43525343  gs   0x4f4d4b53\n\
         \n\
         PANIC: k_trap - kernel mode trap type 0x0000000E\n\
         Trying to dump 5023 pages to dumpdev hd (1/41), 63 pages per '.'\n",
    );
    bst.char_delay(100_000);
    bst.text(
        Align::Left,
        "..............................................................................\n",
    );
    bst.char_delay(0);
    bst.text(Align::Left, "5023 pages dumped\n\n\n");
    bst.pause(2_000_000);
    bst.text(
        Align::Left,
        "**   Safe to Power Off   **\n\
         \x20          - or -\n\
         ** Press Any Key to Reboot **\n",
    );

    bst.y = bst.height - bst.yoff - bst.line_height() * 18;
    bst.clear(d);
    bst
}

/// Linux (sparc) panic, by Tom Kelly.
fn sparc_linux(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);
    bst.scroll_p = true;
    bst.y = bst.height - bst.yoff - bst.line_height();

    bst.text(
        Align::Left,
        "\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\
         Unable to handle kernel paging request at virtual address f0d4a000\n\
         tsk->mm->context = 00000014\n\
         tsk->mm->pgd = f26b0000\n\
         \x20             \\|/ ____ \\|/\n\
         \x20             \"@'/ ,. \\`@\"\n\
         \x20             /_| \\__/ |_\\\n\
         \x20                \\__U_/\n\
         gawk(22827): Oops\n\
         PSR: 044010c1 PC: f001c2cc NPC: f001c2d0 Y: 00000000\n\
         g0: 00001000 g1: fffffff7 g2: 04401086 g3: 0001eaa0\n\
         g4: 000207dc g5: f0130400 g6: f0d4a018 g7: 00000001\n\
         o0: 00000000 o1: f0d4a298 o2: 00000040 o3: f1380718\n\
         o4: f1380718 o5: 00000200 sp: f1b13f08 ret_pc: f001c2a0\n\
         l0: efffd880 l1: 00000001 l2: f0d4a230 l3: 00000014\n\
         l4: 0000ffff l5: f0131550 l6: f012c000 l7: f0130400\n\
         i0: f1b13fb0 i1: 00000001 i2: 00000002 i3: 0007c000\n\
         i4: f01457c0 i5: 00000004 i6: f1b13f70 i7: f0015360\n\
         Instruction DUMP:\n",
    );
    bst.clear(d);
    bst
}

/// BSD Panic by greywolf@starwolf.com, modeled after the Linux panic above.
fn bsd(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, color("#c0c0c0", WHITE), BLACK, f);

    const PANICSTR: &[&str] = &[
        "panic: ifree: freeing free inode\n",
        "panic: blkfree: freeing free block\n",
        "panic: improbability coefficient below zero\n",
        "panic: cgsixmmap\n",
        "panic: crazy interrupts\n",
        "panic: nmi\n",
        "panic: attempted windows install\n",
        "panic: don't\n",
        "panic: free inode isn't\n",
        "panic: cpu_fork: curproc\n",
        "panic: malloc: out of space in kmem_map\n",
        "panic: vogon starship detected\n",
        "panic: teleport chamber: out of order\n",
        "panic: Brain fried - core dumped\n",
    ];

    let mut i = (random() as usize) % PANICSTR.len();
    bst.text(Align::Left, PANICSTR[i]);
    bst.text(Align::Left, "Syncing disks: ");

    let mut b = (random() % 40) as i32;
    let mut n = 0;
    while n < 20 && b > 0 {
        if i != 0 {
            i = (random() & 0x7) as usize;
            b -= ((random() & 0xff) % 20) as i32;
            b = b.max(0);
        }
        bst.text(Align::Left, &format!("{b} "));
        bst.pause(1_000_000);
        n += 1;
    }

    bst.text(Align::Left, "\n");
    bst.text(Align::Left, if b != 0 { "damn!" } else { "sunk!" });
    bst.text(Align::Left, "\nRebooting\n");

    bst.y = bst.height - bst.yoff - bst.line_height() * 4;
    bst.clear(d);
    bst
}

/// A picture and the shape of it: which of its pixels are opaque.
struct Art {
    image: Fb,
    mask: Option<Fb>,
}

impl Art {
    fn load(bytes: &[u8]) -> Option<Art> {
        let (image, mask) = png::decode(bytes)?;
        Some(Art { image, mask })
    }

    fn width(&self) -> i32 {
        self.image.width()
    }

    fn height(&self) -> i32 {
        self.image.height()
    }

    /// `double_pixmap`: nearest-neighbour to twice the size, which is how
    /// upstream fits these small pictures to a big screen.
    fn doubled(&self) -> Art {
        Art {
            image: double_fb(&self.image),
            mask: self.mask.as_ref().map(double_fb),
        }
    }

    /// Blit it, letting the background through wherever it is transparent.
    fn draw(&self, d: &mut Dpy, gc: &mut Gc, x: i32, y: i32) {
        if let Some(m) = &self.mask {
            gc.set_clip_mask(std::rc::Rc::new(m.clone()));
            gc.set_clip_origin(x, y);
        }
        let (w, h) = (self.width(), self.height());
        d.win().copy_area(gc, &self.image, 0, 0, w, h, x, y);
        gc.set_clip_none();
    }
}

fn double_fb(src: &Fb) -> Fb {
    let (w, h) = (src.width(), src.height());
    let mut out = if src.depth() == 1 {
        Fb::new_bitmap(w * 2, h * 2)
    } else {
        Fb::new(w * 2, h * 2)
    };
    for y in 0..h {
        for x in 0..w {
            let p = src.get_pixel(x, y);
            out.put_pixel(x * 2, y * 2, p);
            out.put_pixel(x * 2 + 1, y * 2, p);
            out.put_pixel(x * 2, y * 2 + 1, p);
            out.put_pixel(x * 2 + 1, y * 2 + 1, p);
        }
    }
    out
}

/// The Amiga's Guru Meditation, which flashes a red border above the screen it
/// interrupted.
fn amiga(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, color("#FF0000", WHITE), BLACK, f);

    let guru1 = "Software failure.  Press left mouse button to continue.";
    let guru2 = "Guru Meditation #00000003.00C01570";
    let lw = bst.line_height();
    let (fg, bg) = (bst.fg, bst.bg);
    let bg2 = WHITE;

    bst.yoff = 0;
    bst.top_margin = 0;
    bst.bottom_margin = 0;

    let mut art = Art::load(crate::images::bsod::AMIGA);
    if art.is_some() {
        let mut n = 0;
        if bst.width.min(bst.height) > 600 {
            n += 1;
        }
        if bst.width > 2560 || bst.height > 2560 {
            n += 1; /* Retina displays */
        }
        for _ in 0..n {
            art = art.map(|a| a.doubled());
        }
    }

    bst.gc.set_line_width(lw);

    let height = lw * 5;
    let (w, h) = (bst.width, bst.height);

    bst.char_delay = 0;
    bst.line_delay = 0;

    bst.pause(2_000_000);
    bst.copy(0, 0, w, h - height, 0, height);

    bst.color(fg, bg);
    bst.rect(true, 0, 0, w, height); /* red */
    bst.color(bg, fg);
    bst.rect(true, lw / 2, lw / 2, w - lw, height - lw); /* black */
    bst.color(fg, bg);
    bst.moveto(0, lw * 2);
    bst.text(Align::Center, guru1);
    bst.moveto(0, lw * 7 / 2);
    bst.text(Align::Center, guru2);
    bst.pause(1_000_000);

    bst.color(bg, fg);
    bst.rect(true, 0, 0, w, height); /* black */
    bst.color(fg, bg);
    bst.moveto(0, lw * 2);
    bst.text(Align::Center, guru1);
    bst.moveto(0, lw * 7 / 2);
    bst.text(Align::Center, guru2);
    bst.pause(1_000_000);

    bst.loop_back(-17);

    // The screen it interrupted: upstream leaves whatever was on the display,
    // which on a desktop is the workbench and here is white.
    d.win().clear(bg2);

    if let Some(art) = &art {
        let x = (bst.width - art.width()) / 2;
        let y = (bst.height - art.height()) / 2;
        let mut gc = Gc::default();
        art.draw(d, &mut gc, x, y);
    }

    bst.y += lw;
    bst
}

/// Atari ST, by Marcus Herbert, who had this to say:
///
/// > Though I still have my Atari somewhere, I hardly remember the meaning of
/// > the bombs. I think 9 bombs was "bus error" or something like that. And you
/// > often had a few bombs displayed quickly and then the next few ones coming
/// > up step by step. Perhaps somebody else can tell you more about it.. its
/// > just a quick hack :-}
fn atari(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, BLACK, WHITE, f);

    let mut art = Art::load(crate::images::bsod::ATARI);
    for _ in 0..3 {
        art = art.map(|a| a.doubled());
    }
    let (pix_w, pix_h) = art.as_ref().map_or((16, 16), |a| (a.width(), a.height()));

    let offset = pix_w;
    let x = 0;
    let y = (bst.height / 2).max(0);

    for i in 1..7 {
        bst.copy(x, y, pix_w, pix_h, x + i * offset, y);
    }
    for i in 7..10 {
        bst.pause(1_000_000);
        bst.copy(x, y, pix_w, pix_h, x + i * offset, y);
    }

    d.win().clear(bst.bg);
    if let Some(art) = &art {
        let mut gc = Gc::default();
        art.draw(d, &mut gc, x, y);
    }
    bst
}

/// The sad Mac, and the code it died of.
fn mac(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);

    let string = "0 0 0 0 0 0 0 F\n0 0 0 0 0 0 0 3";

    let mut art = Art::load(crate::images::bsod::MAC);
    // Four times the height of the picture *before* it is doubled, which is
    // how far down the window the whole thing sits.
    let offset = art.as_ref().map_or(32, |a| a.height()) * 4;
    for _ in 0..2 {
        art = art.map(|a| a.doubled());
    }
    let (pix_w, pix_h) = art.as_ref().map_or((100, 128), |a| (a.width(), a.height()));

    bst.xoff = 0;
    bst.left_margin = 0;
    bst.right_margin = 0;

    bst.x = (bst.width - pix_w) / 2;
    bst.y = (((bst.height + offset) / 2) - pix_h - bst.line_height() * 2).max(0);

    d.win().clear(bst.bg);
    if let Some(art) = &art {
        let (x, y) = (bst.x, bst.y);
        let mut gc = Gc::default();
        art.draw(d, &mut gc, x, y);
    }

    bst.y += offset + bst.line_height();
    bst.text(Align::Center, string);
    bst
}

/// HVX ("High-performance Virtual System on Unix") is an AIX application which
/// emulates GCOS6 hardware on RS6000-like machines. By Andrew Reid.
fn hvx(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);

    bst.scroll_p = true;
    bst.wrap_p = true;
    bst.y = bst.height - bst.bottom_margin - bst.yoff - bst.font.ascent();

    bst.char_delay(10_000);
    bst.text(
        Align::Left,
        "(TP) Trap no E   Effective address 00000000   Instruction D7DE\n\
         (TP)  Registers :\n\
         (TP)  B1 -> B7  03801B02  00000000  03880D45  038BABDB  0388AFFD\
         \x20 0389B3F8  03972317\n\
         (TP)  R1 -> R7  0001  0007  F10F  090F  0020  0106  0272\n\
         (TP)  P I Z M1  0388A18B  3232  0000 FF00\n\
         (TP) Program counter is at offset 0028 from string YTPAD\n\
         (TP) User id of task which trapped is LT 626\n\
         (TP)?\n",
    );
    bst.pause(1_000_000);

    bst.char_delay(100_000);
    bst.text(Align::Left, " TP CLOSE ALL");

    bst.char_delay(10_000);
    bst.text(Align::Left, "\n(TP)?\n");
    bst.pause(1_000_000);

    bst.char_delay(100_000);
    bst.text(Align::Left, " TP ABORT -LT ALL");

    bst.char_delay(10_000);
    bst.text(Align::Left, "\n(TP)?\n");
    bst.pause(1_000_000);

    bst.char_delay(100_000);
    bst.text(Align::Left, "  TP STOP KILL");

    bst.char_delay(10_000);
    bst.text(
        Align::Left,
        "\n\
         (TP)?\n\
         Core dumps initiated for selected HVX processes ...\n\
         Core dumps complete.\n\
         Fri Jul 19 15:53:09 2002\n\
         Live registers for cp 0:\n\
         \x20P    =     7de3  IW=0000     I=32    CI=30000000   S=80006013\
         \x20  IV=aa0      Level=13\n\
         \x20R1-7 =       1f      913       13        4        8        0        0\n\
         \x20B1-7 =   64e71b      a93      50e   64e73c     6c2c     7000      b54\n\
         Memory dump starting to file /var/hvx/dp01/diag/Level2 ...\n\
         Memory dump complete.\n",
    );
    bst.clear(d);
    bst
}

/// HPUX panic, by Tobias Klausmann.
///
/// Upstream puts the machine's own hostname in the banner; a browser tab has
/// none, so this is the name it falls back to when `uname` fails.
fn hpux(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);

    bst.scroll_p = true;
    bst.y = bst.height - bst.bottom_margin - bst.yoff - bst.font.ascent();

    bst.text(
        Align::Left,
        "                                                       \
         \x20                                                      \
         \x20                                                      \n",
    );
    bst.text(Align::Left, "HPUX [HP Release B.11.00] (see /etc/issue)\n");
    bst.pause(1_000_000);
    bst.text(
        Align::Left,
        "Console Login:\n\
         \n\
         \x20    ******* Unexpected HPMC/TOC. Processor HPA FFFFFFFF'\
         FFFA0000 *******\n\
         \x20                             GENERAL REGISTERS:\n\
         r00/03 00000000'00000000 00000000'00000000 00000000'00000000 00000000'\
         006C76C0\n\
         r04/07 00000000'00000001 00000000'0126E328 00000000'00000000 00000000'\
         0122B640\n\
         r08/11 00000000'00000000 00000000'0198CFC0 00000000'000476FE 00000000'\
         00000001\n\
         r12/15 00000000'40013EE8 00000000'08000080 00000000'4002530C 00000000'\
         4002530C\n\
         r16/19 00000000'7F7F2A00 00000000'00000001 00000000'00000000 00000000'\
         00000000\n\
         r20/23 00000000'006C8048 00000000'00000001 00000000'00000000 00000000'\
         00000000\n\
         r24/27 00000000'00000000 00000000'00000000 00000000'00000000 00000000'\
         00744378\n\
         r28/31 00000000'00000000 00000000'007DD628 00000000'0199F2B0 00000000'\
         00000000\n\
         \x20                             CONTROL REGISTERS:\n\
         sr0/3  00000000'0F3B4000 00000000'0C2A2000 00000000'016FF800 00000000'\
         00000000\n\
         sr4/7  00000000'00000000 00000000'016FF800 00000000'0DBF1400 00000000'\
         00000000\n\
         pcq =  00000000'00000000.00000000'00104950 00000000'00000000.00000000'\
         00104A14\n\
         isr =  00000000'10240006 ior = 00000000'67D9E220 iir = 08000240 rctr = \
         7FF10BB6\n\
         \n\
         pid reg cr8/cr9    00007700'0000B3A9 00000000'0000C5D8\n\
         pid reg cr12/cr13  00000000'00000000 00000000'00000000\n\
         ipsw = 000000FF'080CFF1F iva = 00000000'0002C000 sar = 3A ccr = C0\n\
         tr0/3  00000000'006C76C0 00000000'00000001 00000000'00000000 00000000'\
         7F7CE000\n\
         tr4/7  00000000'03790000 0000000C'4FB68340 00000000'C07EE13F 00000000'\
         0199F2B0\n\
         eiem = FFFFFFF0'FFFFFFFF eirr = 80000000'00000000 itmr = 0000000C'\
         4FD8EDE1\n\
         cr1/4  00000000'00000000 00000000'00000000 00000000'00000000 00000000'\
         00000000\n\
         cr5/7  00000000'00000000 00000000'00000000 00000000'\
         00000000\n\
         \x20                          MACHINE CHECK PARAMETERS:\n\
         Check Type = 00000000 CPU STATE = 9E000001 Cache Check = 00000000\n\
         TLB Check = 00000000 Bus Check = 00000000 PIM State = ? SIU \
         Status = ????????\n\
         Assists = 00000000 Processor = 00000000\n\
         Slave Addr = 00000000'00000000 Master Addr = 00000000'00000000\n\
         \n\
         \n\
         TOC,    pcsq.pcoq = 0'0.0'104950   , isr.ior = 0'10240006.0'67d9e220\n\
         @(#)B2352B/9245XB HP-UX (B.11.00) #1: Wed Nov  5 22:38:19 PST 1997\n\
         Transfer of control: (display==0xd904, flags==0x0)\n\
         \n\
         \n\
         \n\
         *** A system crash has occurred.  (See the above messages for details.)\n\
         *** The system is now preparing to dump physical memory to disk, for use\n\
         *** in debugging the crash.\n\
         \n\
         *** The dump will be a SELECTIVE dump:  40 of 256 megabytes.\n\
         *** To change this dump type, press any key within 10 seconds.\n\
         *** Proceeding with selective dump.\n\
         \n\
         *** The dump may be aborted at any time by pressing ESC.\n",
    );

    let steps = 11;
    let size = 40;
    for i in 0..=steps {
        bst.text(
            Align::Left,
            &format!(
                "*** Dumping: {:3}% complete ({} of 40 MB) (device 64:0x2)\r",
                i * 100 / steps,
                i * size / steps
            ),
        );
        bst.pause(1_500_000);
    }

    bst.text(Align::Left, "\n*** System rebooting.\n");
    bst.clear(d);
    bst
}

/// IBM OS/390 aka MVS aka z/OS. Text from Dan Espen. Apparently this isn't
/// actually a crash, just a random session... But who can tell.
fn os390(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, color("Red", WHITE), BLACK, f);

    bst.scroll_p = true;
    bst.y = bst.height - bst.bottom_margin - bst.yoff - bst.font.ascent();

    bst.line_delay(100_000);
    bst.text(
        Align::Left,
        "\n*** System rebooting.\n\
         * ISPF Subtask abend *\n\
         SPF      ENDED DUE TO ERROR+\n\
         READY\n\
         \n\
         IEA995I SYMPTOM DUMP OUTPUT\n\
         \x20 USER COMPLETION CODE=0222\n\
         \x20TIME=23.00.51  SEQ=03210  CPU=0000  ASID=00AE\n\
         \x20PSW AT TIME OF ERROR  078D1000   859DAF18  ILC 2  INTC 0D\n\
         \x20  NO ACTIVE MODULE FOUND\n\
         \x20  NAME=UNKNOWN\n\
         \x20  DATA AT PSW  059DAF12 - 00181610  0A0D9180  70644710\n\
         \x20  AR/GR 0: 00000000/80000000   1: 00000000/800000DE\n\
         \x20        2: 00000000/196504DC   3: 00000000/00037A78\n\
         \x20        4: 00000000/00037B78   5: 00000000/0003351C\n\
         \x20        6: 00000000/0000F0AD   7: 00000000/00012000\n\
         \x20        8: 00000000/059DAF10   9: 00000000/0002D098\n\
         \x20        A: 00000000/059D9F10   B: 00000000/059D8F10\n\
         \x20        C: 00000000/859D7F10   D: 00000000/00032D60\n\
         \x20        E: 00000000/00033005   F: 01000002/00000041\n\
         \x20END OF SYMPTOM DUMP\n\
         ISPS014 - ** Logical screen request failed - abend 0000DE **\n\
         ISPS015 - ** Contact your system programmer or dialog developer.**\n\
         *** ISPF Main task abend ***\n\
         IEA995I SYMPTOM DUMP OUTPUT\n\
         \x20 USER COMPLETION CODE=0222\n\
         \x20TIME=23.00.52  SEQ=03211  CPU=0000  ASID=00AE\n\
         \x20PSW AT TIME OF ERROR  078D1000   8585713C  ILC 2  INTC 0D\n\
         \x20  ACTIVE LOAD MODULE           ADDRESS=05855000  OFFSET=0000213C\n\
         \x20  NAME=ISPMAIN\n\
         \x20  DATA AT PSW  05857136 - 00181610  0A0D9180  D3304770\n\
         \x20  GR 0: 80000000   1: 800000DE\n\
         \x20     2: 00015260   3: 00000038\n\
         \x20     4: 00012508   5: 00000000\n\
         \x20     6: 000173AC   7: FFFFFFF8\n\
         \x20     8: 05858000   9: 00012CA0\n\
         \x20     A: 05857000   B: 05856000\n\
         \x20     C: 85855000   D: 00017020\n\
         \x20     E: 85857104   F: 00000000\n\
         \x20END OF SYMPTOM DUMP\n\
         READY\n\
         ***",
    );
    bst.cursor(false, 240_000, 999_999);
    bst.clear(d);
    bst
}

/// Compaq Tru64 Unix panic, by jwz as described by Tobias Klausmann.
fn tru64(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#0000AA", BLACK), f);

    bst.scroll_p = true;
    bst.y = bst.height - bst.bottom_margin - bst.yoff - bst.font.ascent();

    bst.text(
        Align::Left,
        "Compaq Tru64 UNIX V5.1B (Rev. 2650) (127.0.0.1) console\n\n\
         login: ",
    );
    bst.pause(6_000_000);

    bst.text(
        Align::Left,
        "panic (cpu 0): trap: illegal instruction\n\
         kernel inst fault=gentrap, ps=0x5, pc=0xfffffc0000593878, inst=0xaa\n\
         kernel inst fault=gentrap, ps=0x5, pc=0xfffffc0000593878, inst=0xaa\n\
         \x20                                                                  \n\
         DUMP: blocks available:  1571600\n\
         DUMP: blocks wanted:      100802 (partial compressed dump) [OKAY]\n\
         DUMP: Device     Disk Blocks Available\n\
         DUMP: ------     ---------------------\n\
         DUMP: 0x1300023  1182795 - 1571597 (of 1571598) [primary swap]\n\
         DUMP.prom: Open: dev 0x5100041, block 2102016: SCSI 0 11 0 2 200 0 0\n\
         DUMP: Writing header... [1024 bytes at dev 0x1300023, block 1571598]\n\
         DUMP: Writing data",
    );

    let steps = 4 + (random() % 8);
    bst.char_delay(1_000_000);
    for _ in 0..steps {
        bst.text(Align::Left, ".");
    }
    bst.char_delay(0);
    bst.text(Align::Left, &format!("[{steps}MB]\n"));

    bst.text(
        Align::Left,
        "DUMP: Writing header... [1024 bytes at dev 0x1300023, block 1571598]\n\
         DUMP: crash dump complete.\n\
         kernel inst fault=gentrap, ps=0x5, pc=0xfffffc0000593878, inst=0xaa\n\
         \x20                                                                  \n\
         DUMP: second crash dump skipped: 'dump_savecnt' enforced.\n",
    );
    bst.pause(4_000_000);

    bst.text(
        Align::Left,
        "\n\
         halted CPU 0\n\
         \n\
         halt code = 5\n\
         HALT instruction executed\n\
         PC = fffffc00005863b0\n",
    );
    bst.pause(3_000_000);

    bst.text(Align::Left, "\nCPU 0 booting\n\n\n\n");
    bst.clear(d);
    bst
}

/// MS-DOS, by jwz.
fn msdos(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);

    bst.char_delay(10_000);
    bst.text(Align::Left, "C:\\WINDOWS>");
    bst.cursor(false, 200_000, 8);

    bst.char_delay(200_000);
    bst.text(Align::Left, "dir a:");
    bst.pause(1_000_000);

    bst.char_delay(10_000);
    bst.text(
        Align::Left,
        "\nNot ready reading drive A\nAbort, Retry, Fail?",
    );

    bst.cursor(false, 200_000, 10);
    bst.char_delay(200_000);
    bst.text(Align::Left, "f");
    bst.pause(1_000_000);

    bst.char_delay(10_000);
    bst.text(
        Align::Left,
        "\n\n\nNot ready reading drive A\nAbort, Retry, Fail?",
    );

    bst.cursor(false, 200_000, 10);
    bst.char_delay(200_000);
    bst.text(Align::Left, "f");
    bst.pause(1_000_000);

    bst.char_delay(10_000);
    bst.text(
        Align::Left,
        "\nVolume in drive A has no label\n\n\
         Not ready reading drive A\nAbort, Retry, Fail?",
    );

    bst.cursor(false, 200_000, 12);
    bst.char_delay(200_000);
    bst.text(Align::Left, "a");
    bst.pause(1_000_000);

    bst.char_delay(10_000);
    bst.text(Align::Left, "\n\nC:\\WINDOWS>");

    bst.cursor(false, 200_000, 999_999);
    bst.clear(d);
    bst
}

/// A crash spotted on a cash machine circa 2006, by jwz. He didn't note what
/// model it was; probably a Tranax Mini-Bank 1000 or similar vintage.
fn atm(d: &mut Dpy, f: &Fonts) -> Bst {
    let bst = Bst::new(d, BLACK, color("#FF6600", WHITE), f);
    let scale = 0.48;

    bst.clear(d);

    let mut art = Art::load(crate::images::bsod::ATM);
    let (mut pix_w, mut pix_h) = art.as_ref().map_or((64, 64), |a| (a.width(), a.height()));
    let mut i = 0;
    while f64::from(pix_w) <= f64::from(bst.width) * scale
        && f64::from(pix_h) <= f64::from(bst.height) * scale
    {
        art = art.map(|a| a.doubled());
        pix_w *= 2;
        pix_h *= 2;
        i += 1;
    }

    let x = (bst.width - pix_w) / 2;
    let y = ((bst.height - pix_h) / 2).max(0);

    if let Some(art) = &mut art {
        if i > 0 {
            // Rule the enlarged picture back into pixels, so it still looks
            // like the low-resolution screen it came off.
            let gc = Gc::new(bst.bg, bst.bg);
            let mut j = -1;
            while j < pix_w {
                art.image.draw_line(&gc, j, 0, j, pix_h);
                j += i + 1;
            }
            let mut j = -1;
            while j < pix_h {
                art.image.draw_line(&gc, 0, j, pix_w, j);
                j += i + 1;
            }
        }
        let mut gc = Gc::default();
        art.draw(d, &mut gc, x, y);
    }
    bst
}

/// Gnome SOD. Truly 2020 will be the year of the Linux Desktop.
fn gnome(d: &mut Dpy, f: &Fonts) -> Bst {
    let which = random() & 1 != 0;
    let (art, fg, bg) = if which {
        (
            Art::load(crate::images::bsod::GNOME2),
            color("#2E3436", BLACK),
            color("#F0F0F0", WHITE),
        )
    } else {
        (
            Art::load(crate::images::bsod::GNOME1),
            color("#E2E2E2", WHITE),
            BLACK,
        )
    };
    let mut bst = Bst::new(d, fg, bg, f);
    let lh = bst.line_height();

    let (pix_w, pix_h) = art.as_ref().map_or((64, 64), |a| (a.width(), a.height()));
    let x = (bst.width - pix_w) / 2;
    let y = ((bst.height - pix_h) / 2).max(0);

    bst.clear(d);
    if let Some(art) = &art {
        let mut gc = Gc::default();
        art.draw(d, &mut gc, x, y);
    }

    bst.moveto(0, y + pix_h + lh * 2);
    bst.color(fg, bg);
    bst.set_font(0);
    bst.text(Align::Center, "Oh no!  Something has gone wrong!\n\n");
    bst.set_font(1);
    bst.text(
        Align::Center,
        "A problem has occurred and the system can't recover.\n",
    );
    bst.text(Align::Center, "Please log out and try again.");
    bst.pause(60 * 1_000_000);
    bst
}

/// MacsBug, the debugger a Macintosh dropped into: the register window down
/// the left, the disassembly along the bottom, and the call chain filling the
/// rest, with a cursor that inverts the whole page rather than blinking.
fn macsbug(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, BLACK, WHITE, f);

    let left = "    SP     \n\
                \x2004EB0A58  \n\
                58 00010000\n\
                5C 00010000\n\
                \x20  ........\n\
                60 00000000\n\
                64 000004EB\n\
                \x20  ........\n\
                68 0000027F\n\
                6C 2D980035\n\
                \x20  ....-..5\n\
                70 00000054\n\
                74 0173003E\n\
                \x20  ...T.s.>\n\
                78 04EBDA76\n\
                7C 04EBDA8E\n\
                \x20  .S.L.a.U\n\
                80 00000000\n\
                84 000004EB\n\
                \x20  ........\n\
                88 00010000\n\
                8C 00010000\n\
                \x20  ...{3..S\n\
                \n\
                \n\
                \x20CurApName \n\
                \x20 Finder   \n\
                \n\
                \x2032-bit VM \n\
                SR Smxnzvc0\n\
                D0 04EC0062\n\
                D1 00000053\n\
                D2 FFFF0100\n\
                D3 00010000\n\
                D4 00010000\n\
                D5 04EBDA76\n\
                D6 04EBDA8E\n\
                D7 00000001\n\
                \n\
                A0 04EBDA76\n\
                A1 04EBDA8E\n\
                A2 A0A00060\n\
                A3 027F2D98\n\
                A4 027F2E58\n\
                A5 04EC04F0\n\
                A6 04EB0A86\n\
                A7 04EB0A58";
    let bottom = "  _A09D\n\
                  \x20    +00884    40843714     #$0700,SR         \
                  \x20                 ; A973        | A973\n\
                  \x20    +00886    40843765     *+$0400           \
                  \x20                               | 4A1F\n\
                  \x20    +00888    40843718     $0004(A7),([0,A7[)\
                  \x20                 ; 04E8D0AE    | 66B8";
    let body = "PowerPC unmapped memory exception at 003AFDAC \
                BowelsOfTheMemoryMgr+04F9C\n\
                \x20Calling chain using A6/R1 links\n\
                \x20 Back chain  ISA  Caller\n\
                \x20 00000000    PPC  28C5353C  __start+00054\n\
                \x20 24DB03C0    PPC  28B9258C  main+0039C\n\
                \x20 24DB0350    PPC  28B9210C  MainEvent+00494\n\
                \x20 24DB02B0    PPC  28B91B40  HandleEvent+00278\n\
                \x20 24DB0250    PPC  28B83DAC  DoAppleEvent+00020\n\
                \x20 24DB0210    PPC  FFD3E5D0  AEProcessAppleEvent+00020\n\
                \x20 24DB0132    68K  00589468\n\
                \x20 24DAFF8C    68K  00589582\n\
                \x20 24DAFF26    68K  00588F70\n\
                \x20 24DAFEB3    PPC  00307098  EmToNatEndMoveParams+00014\n\
                \x20 24DAFE40    PPC  28B9D0B0  DoScript+001C4\n\
                \x20 24DAFDD0    PPC  28B9C35C  RunScript+00390\n\
                \x20 24DAFC60    PPC  28BA36D4  run_perl+000E0\n\
                \x20 24DAFC10    PPC  28BC2904  perl_run+002CC\n\
                \x20 24DAFA80    PPC  28C18490  Perl_runops+00068\n\
                \x20 24DAFA30    PPC  28BE6CC0  Perl_pp_backtick+000FC\n\
                \x20 24DAF9D0    PPC  28BA48B8  Perl_my_popen+00158\n\
                \x20 24DAF980    PPC  28C5395C  sfclose+00378\n\
                \x20 24DAF930    PPC  28BA568C  free+0000C\n\
                \x20 24DAF8F0    PPC  28BA6254  pool_free+001D0\n\
                \x20 24DAF8A0    PPC  FFD48F14  DisposePtr+00028\n\
                \x20 24DAF7C9    PPC  00307098  EmToNatEndMoveParams+00014\n\
                \x20 24DAF780    PPC  003AA180  __DisposePtr+00010";

    let body_lines = 1 + body.matches('\n').count() as i32;

    let (fg, bg) = (bst.fg, bst.bg);
    let bc = color("#AAAAAA", WHITE);

    bst.xoff = 0;
    bst.left_margin = 0;
    bst.right_margin = 0;

    let char_width = bst.font.char_width();
    let line_height = bst.line_height();

    let col_right = char_width * 12; /* number of columns in `left' */
    let mut page_bottom = (line_height * 47).min(bst.height - bst.yoff); /* lines in `left' */

    let row_bottom = page_bottom - line_height;
    let row_top = row_bottom - line_height * 4;
    let page_right = col_right + char_width * 88;
    let mut body_top = row_top - line_height * body_lines;

    page_bottom += 2;
    let row_bottom = row_bottom + 2;
    body_top -= 4;
    body_top = body_top.min(4);

    let xoff = ((bst.width - page_right) / 2).max(0);
    let yoff = ((bst.height - page_bottom) / 2).max(0);

    bst.margins(xoff, xoff);

    bst.color(bc, bg);
    let (w, h) = (bst.width, bst.height);
    bst.rect(true, 0, 0, w, h);
    bst.color(bg, bg);
    bst.rect(true, xoff - 2, yoff, page_right + 4, page_bottom);
    bst.color(fg, bg);

    bst.moveto(xoff, yoff + line_height);
    bst.text(Align::Left, left);
    bst.moveto(xoff + col_right, yoff + row_top + line_height);
    bst.text(Align::Left, bottom);

    bst.rect(true, xoff + col_right, yoff, 2, page_bottom);
    bst.rect(
        true,
        xoff + col_right,
        yoff + row_top,
        page_right - col_right,
        1,
    );
    bst.rect(
        true,
        xoff + col_right,
        yoff + row_bottom,
        page_right - col_right,
        1,
    );
    bst.rect(false, xoff - 2, yoff, page_right + 4, page_bottom);

    bst.line_delay(500);
    bst.moveto(xoff + col_right + char_width, yoff + body_top + line_height);
    bst.margins(xoff + col_right + char_width, xoff + col_right + char_width);
    bst.text(Align::Left, body);

    bst.rect(false, xoff - 2, yoff, page_right + 4, page_bottom); /* again */

    bst.rect(
        false,
        xoff + col_right + char_width / 2 + 2,
        yoff + row_bottom + 2,
        0,
        page_bottom - row_bottom - 4,
    );

    bst.pause(666_666);
    bst.invert();
    bst.loop_back(-3);

    bst.clear(d);
    bst
}

/// The original Macintosh's bomb.
fn mac1(d: &mut Dpy, f: &Fonts) -> Bst {
    let bst = Bst::new(d, BLACK, WHITE, f);

    let mut art = Art::load(crate::images::bsod::MACBOMB);
    let (mut pix_w, mut pix_h) = art.as_ref().map_or((32, 32), |a| (a.width(), a.height()));
    if art.is_some() && pix_w < bst.width / 2 && pix_h < bst.height / 2 {
        let mut n = 1;
        if bst.width > 2560 || bst.height > 2560 {
            n += 1; /* Retina displays */
        }
        for _ in 0..n {
            art = art.map(|a| a.doubled());
            pix_w *= 2;
            pix_h *= 2;
        }
    }

    let x = (bst.width - pix_w) / 2;
    let y = ((bst.height - pix_h) / 2).max(0);

    bst.clear(d);
    if let Some(art) = &art {
        let mut gc = Gc::default();
        art.draw(d, &mut gc, x, y);
    }
    bst
}

/// This is what kernel panics looked like on MacOS X 10.0 through 10.1.5. In
/// later releases, it's a graphic of a power button with text in English,
/// French, German and Japanese overlayed transparently.
fn macx_10_0(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#888888", BLACK), f);
    let fg2 = WHITE;
    let bg2 = BLACK;

    bst.clear(d);
    {
        let art = Art::load(crate::images::bsod::HMAC).map(|a| a.doubled());
        let (pix_w, pix_h) = art.as_ref().map_or((64, 64), |a| (a.width(), a.height()));
        let x = (bst.width - pix_w) / 2;
        let y = ((bst.height - pix_h) / 2).max(0);
        if let Some(art) = &art {
            let mut gc = Gc::default();
            art.draw(d, &mut gc, x, y);
        }
    }

    bst.left_margin = 0;
    bst.right_margin = 0;
    bst.y = bst.font.ascent();
    bst.macx_eol_kludge = true;
    bst.wrap_p = true;

    bst.pause(3_000_000);
    bst.color(fg2, bg2);
    bst.text(
        Align::Left,
        "panic(cpu 0): Unable to find driver for this platform: \
         \"PowerMac 3,5\".\n\
         \n\
         backtrace: 0x0008c2f4 0x0002a7a0 0x001f0204 0x001d4e4c 0x001d4c5c \
         0x001a56cc 0x01d5dbc 0x001c621c 0x00037430 0x00037364\n\
         \n\
         \n\
         \n\
         No debugger configured - dumping debug information\n\
         \n\
         version string : Darwin Kernel Version 1.3:\n\
         Thu Mar  1 06:56:40 PST 2001; root:xnu/xnu-123.5.obj~1/RELEASE_PPC\n\
         \n\
         \n\
         \n\
         \n\
         DBAT0: 00000000 00000000\n\
         DBAT1: 00000000 00000000\n\
         DBAT2: 80001FFE 8000003A\n\
         DBAT3: 90001FFE 9000003A\n\
         MSR=00001030\n\
         backtrace: 0x0008c2f4 0x0002a7a0 0x001f0204 0x001d4e4c 0x001d4c5c \
         0x001a56cc 0x01d5dbc 0x001c621c 0x00037430 0x00037364\n\
         \n\
         panic: We are hanging here...\n",
    );
    bst
}

/// 10.2 and 10.3, which say it in four languages over a picture of the power
/// button, on top of whatever you were doing.
fn macx_10_2(d: &mut Dpy, f: &Fonts, v10_3_p: bool) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#888888", BLACK), f);

    let mut art = Art::load(if v10_3_p {
        crate::images::bsod::OSX_10_3
    } else {
        crate::images::bsod::OSX_10_2
    });
    if bst.height > 600 {
        /* scale up the bitmap */
        art = art.map(|a| a.doubled());
    }
    let (pix_w, pix_h) = art.as_ref().map_or((512, 512), |a| (a.width(), a.height()));

    bst.img();
    bst.pause(2_000_000);

    if let Some(a) = art {
        bst.mask = a.mask.map(std::rc::Rc::new);
        bst.pixmap = Some(a.image);
    }

    let x = (bst.width - pix_w) / 2;
    let y = (bst.height - pix_h) / 2;
    bst.pixmap_at(0, 0, pix_w, pix_h, x, y);
    bst
}

/// 2006 Mac Mini with MacOS 10.6 failing with a bad boot drive. By jwz.
fn mac_diskfail(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);

    let cw = bst.font.char_width();
    let h = bst.line_height();
    let l = ((bst.width - cw * 80) / 2).max(0);
    let t = ((bst.height - h * 10) / 2).max(0);

    let (fg, bg) = (bst.fg, bst.bg);
    let bg2 = color("#888888", BLACK);

    bst.wrap_p = true;
    bst.scroll_p = true;

    let (w, hh) = (bst.width, bst.height);
    bst.color(bg2, bg);
    bst.rect(true, 0, 0, w, hh);
    bst.pause(3_000_000);

    bst.color(bg, fg);
    bst.rect(true, 0, 0, w, hh);
    bst.color(fg, bg);

    bst.margins(l, l);
    bst.moveto(l, t);

    bst.text(
        Align::Left,
        "efiboot loaded from device: Acpi(PNP0A03,0)/Pci*1F|2)/Ata\
         (Primary,Slave)/HD(Part\n\
         2,Sig8997E427-064E-4FE7-8CB9-F27A784B232C)\n\
         boot file path: \\System\\Library\\CoreServices\\boot.efi\n\
         .Loading kernel cache file 'System\\Library\\Caches\\\
         com.apple.kext.caches\\Startup\\\n\
         kernelcache_i386.2A14EC2C'\n\
         Loading 'mach_kernel'...\n",
    );
    bst.char_delay(7000);
    bst.text(Align::Left, ".....................\n");
    bst.char_delay(0);
    bst.text(
        Align::Left,
        "root device uuid is 'B62181B4-6755-3C27-BFA1-49A0E053DBD6\n\
         Loading drivers...\n\
         Loading System\\Library\\Caches\\com.apple.kext.caches\\\
         Startup\\Extensions.mkext....\n",
    );
    bst.char_delay(7000);
    bst.text(
        Align::Left,
        "................................................................\
         ................\n\
         ................................................................\
         ................\n\
         ..............\n",
    );
    bst.invert();
    bst.rect(true, 0, 0, w, hh);
    bst.invert();

    bst.margins(0, 0);
    bst.moveto(0, h);

    bst.char_delay(0);
    bst.line_delay(5000);
    bst.text(
        Align::Left,
        "npvhash=4095\n\
         PRE enabled\n\
         Darwin Kernel Version 10.8.9: Tue Jun  7 16:33:36 PDT 2011;\
         \x20root:xnu-1504.15.3~1/RELEASE_I386\n\
         vm_page_bootstrap: 508036 free pages and 16252 wired pages\n\
         standard timeslicing quantum is 10000 us\n\
         mig_table_max_displ = 73\n\
         AppleACPICPU: ProcessorId=0 LocalApicId=0 Enabled\n\
         AppleACPICPU: ProcessorId=1 LocalApicId=1 Enabled\n\
         calling npo_policy_init for Quarantine\n\
         Security policy loaded: Quaantine policy (Quarantine)\n\
         calling npo_policy_init for Sandbox\n\
         Security policy loaded: Seatbelt sandbox policy (Sandbox)\n\
         calling npo_policy_init for TMSafetyNet\n\
         Security policy loaded: Safety net for Time Machine (TMSafetyNet)\n\
         Copyright (c) 1982, 1986, 1989, 1991, 1993\n\
         The Regents of the University of California. All rights reserved.\n\
         \n\
         MAC Framework successfully initialized\n\
         using 10485 buffer headers and 4096 cluster IO buffer headers\n\
         IOAPIC: Version 0x20 Vectors 64:87\n\
         ACPI: System State [S0 S3 S4 S5] (S3)\n\
         PFM64 0x10000000, 0xf0000000\n\
         [ PCI configuration begin ]\n\
         PCI configuration changed (bridge=1 device=1 cardbus=0)\n\
         [ PCI configuration end, bridges 4 devices 17 ]\n\
         nbinit: done (64 MB memory set for nbuf pool)\n\
         rooting via boot-uuid from /chosen: \
         B62181B4-6755-3C27-BFA1-49A0E053DBD6\n\
         Waiting on <dict ID=\"0\"><key>IOProviderClass</key>\
         <string ID=\"1\">IOResources</string><key>IOResourceMatch</key>\
         <string ID=\"2\">boot-uuid-nedia</string></dict>\n\
         com.apple.AppleFSCCompressionTypeZlib kmod start\n\
         com.apple.AppleFSCCompressionTypeZlib kmod succeeded\n\
         AppleIntelCPUPowerManagementClient: ready\n\
         FireWire (OHCI) Lucent ID 5811  built-in now active, GUID \
         0019e3fffe97f8b4; max speed s400.\n\
         Got boot device = IOService:/AppleACPIPlatformExpert/PCI000/\
         AppleACPIPCI/SATA@1F,2/AppleAHCI/PRI202/IOAHCIDevice@0/\
         AppleAHCIDiskDriver/IOAHCIBlockStorageDevice/\
         IOBlockStorageDriver/ST96812AS Media/IOGUIDPartitionScheme/\
         Customer02\n",
    );
    bst.pause(1_000_000);
    bst.text(
        Align::Left,
        "BSD root: Disk0s, major 14, minor 2\n\
         [Bluetooth::CSRHIDTransition] switchtoHCIMode (legacy)\n\
         [Bluetooth::CSRHIDTransition] transition complete.\n\
         CSRUSBBluetoothHCIController::setupHardware super returned 0\n",
    );
    bst.pause(3_000_000);
    bst.text(
        Align::Left,
        "disk0s2: I/O error.\n\
         0 [Level 3] [ReadUID 0] [Facility com.apple.system.fs] \
         [ErrType IO] [ErrNo 5] [IOType Read] [PBlkNum 48424] \
         [LBlkNum 1362] [FSLogMsgID 2009724291] [FSLogMsgOrder First]\n\
         0 [Level 3] [ReadUID 0] [Facility com.apple.system.fs] \
         [DevNode root_device] [MountPt /] [FSLogMsgID 2009724291] \
         [FSLogMsgOrder Last]\n\
         panic(cpu 0 caller 0x47f5ad): \"Process 1 exec of /sbin/launchd\
         \x20failed, errno 5\\n\"0/SourceCache/xnu/xnu-1504.15.3/bsd/kern/\
         kern_exec.c:3145\n\
         Debugger called: <panic>\n\
         Backtrace (CPU 0), Frame : Return Address (4 potential args on stack)\n\
         0x34bf3e48 : 0x21b837 (0x5dd7fc 0x34bf3e7c 0x223ce1 0x0)\n\
         0x34bf3e98 : 0x47f5ad (0x5cf950 0x831c08 0x5 0x0)\n\
         0x34bf3ef8 : 0x4696d2 (0x4800d20 0x1fe 0x45a69a0 0x80000001)\n\
         0x34bf3f38 : 0x48fee5 (0x46077a8 0x84baa0 0x34bf3f88 0x34bf3f94)\n\
         0x34bf3f68 : 0x219432 (0x46077a8 0xffffff7f 0x0 0x227c4b)\n\
         0x34bf3fa8 : 0x2aacb4 (0xffffffff 0x1 0x22f8f5 0x227c4b)\n\
         0x34bf3fc8 : 0x2a1976 (0x0 0x0 0x2a17ab 0x4023ef0)\n\
         \n\
         BSD process name corresponding to current thread: init\n\
         \n\
         Mac OS version:\n\
         Not yet set\n\
         \n\
         Kernel version:\n\
         Darwin Kernel version 10.8.0: Tue Jun  7 16:33:36 PDT 2011; \
         root:xnu-1504.15-3~1/RELEASE_I386\n\
         System model name: Macmini1,1 (Mac-F4208EC0)\n\
         \n\
         System uptime in nanoseconds: 13239332027\n",
    );
    bst.cursor(true, 500_000, 999_999);

    bst.clear(d);
    bst
}

/// A Mac installing a software update, on a progress bar that never gets past
/// ninety per cent and an estimate that goes up as often as down.
fn macx_install(d: &mut Dpy, f: &Fonts) -> Bst {
    let fg = WHITE;
    let bg = BLACK;
    let fg2 = color("#C0C0C0", WHITE);
    let bg2 = color("#888888", WHITE);
    let mut bst = Bst::new(d, fg, bg, f);

    let lh = bst.line_height();

    let mut art = Art::load(crate::images::bsod::APPLE);
    if bst.width > 2560 || bst.height > 2560 {
        art = art.map(|a| a.doubled()); /* Retina displays */
    }
    let (pix_w, pix_h) = art.as_ref().map_or((64, 64), |a| (a.width(), a.height()));

    bst.xoff = 0;
    bst.left_margin = 0;
    bst.right_margin = 0;

    if let Some(a) = art {
        bst.mask = a.mask.map(std::rc::Rc::new);
        bst.pixmap = Some(a.image);
    }

    let mut x = (bst.width - pix_w) / 2;
    let mut y = (bst.height / 2 - pix_h).max(0);

    bst.gc.set_line_width(1);

    let (w, h) = (bst.width, bst.height);
    bst.color(bg, bg);
    bst.rect(true, 0, 0, w, h);
    bst.color(fg, bg);
    bst.pixmap_at(0, 0, pix_w, pix_h, x, y);
    y += pix_h * 2 - lh;

    /* progress bar */
    let bw1 = pix_w * 5 / 2;
    let bh1 = ((f64::from(lh) * 0.66) as i32).max(8);

    x = (bst.width - bw1) / 2;
    bst.color(fg2, bg);
    bst.line(x, y, x + bw1, y, bh1);

    let bw2 = bw1 - 1;
    let bh2 = bh1 - 4;
    bst.color(bg2, bg);
    bst.line(x + 1, y, x + bw2, y, bh2);

    bst.color(fg, bg);
    bst.line(x, y, x + 1, y, bh1);

    let mut pct = 5.0 + f64::from(random() % 40);
    let mut min = 5 + (random() % 40) as i32;

    for _ in 0..100 {
        pct += frand(0.3);
        min += (random() % 3) as i32 - 1; /* sometimes down, mostly up */

        pct = pct.min(90.0);
        bst.rect(
            true,
            x,
            y - bh1 / 2,
            (f64::from(bw1) * pct / 100.0) as i32,
            bh1,
        );

        bst.y = y + lh * 3;
        bst.text(
            Align::Center,
            &format!("  Installing Software Update: about {min} minutes.  "),
        );
        bst.pause(1_000_000);
    }
    bst
}

/// Five ways for a Mac OS X machine to stop.
fn macx(d: &mut Dpy, f: &Fonts) -> Bst {
    match random() % 5 {
        0 => macx_10_0(d, f),
        1 => macx_10_2(d, f, false),
        2 => macx_10_2(d, f, true),
        3 => mac_diskfail(d, f),
        _ => macx_install(d, f),
    }
}

/// BitLocker, which would like a key you do not have, in five different ways.
fn bitlocker(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#1070AA", BLACK), f);

    let top = bst.font_b.ascent() + bst.font_b.descent();
    let left = top * 2;
    let right = (left + bst.font_b.ascent() * 22).min(bst.width);
    let mut bottom = "Press Enter to reboot and try again\n\
                      Press ESC for BitLocker recovery";

    let w = bst.width;
    bst.margins(left, w - right);
    bst.word_wrap();
    bst.set_font(1);
    bst.moveto(0, top * 2);

    match random() % 5 {
        0 => {
            bst.text(Align::Left, "BitLocker\n\n");
            bst.set_font(0);
            bst.text(
                Align::Left,
                "Plug in the USB drive that has the BitLocker key\n",
            );
        }

        1 => {
            bst.text(Align::Left, "BitLocker recovery\n\n");
            bst.set_font(0);
            bst.text(
                Align::Left,
                "To recover this drive, plug in the USB drive that has \
                 the BitLocker recovery key\n\n",
            );
            bst.set_font(2);
            bst.text(
                Align::Left,
                "Bitlocker needs your recovery key to unlock your drive \
                 because Secure Boot policy has unexpectedly changed.\n\
                 For more information on how to retrieve this key, go to\n\
                 https://www.jwz.org/xscreensaver/ from another PC \
                 or mobile device.",
            );
            bottom = "Press Enter to reboot and try again\n\
                      Press Esc or the Windows key for more recovery options";
        }

        2 => {
            bst.text(Align::Left, "BitLocker\n\n");
            bst.set_font(0);
            bst.text(Align::Left, "Enter the PIN to unlock this drive\n");
            bst.invert();
            bst.truncate();
            bst.text(Align::Left, &" ".repeat(96));
            bst.invert();
            bst.word_wrap();
            bst.text(
                Align::Left,
                "\n\n\n\
                 Use the number keys or function keys F1-F10 (use F10 for 0).\
                 \n\n\n\
                 Press the Insert key to see the PIN as you type.",
            );
        }

        3 => {
            bst.text(Align::Left, "BitLocker recovery\n\n");
            bst.set_font(0);
            bst.text(Align::Left, "Enter the recovery key for this drive\n");
            bst.invert();
            bst.truncate();
            bst.text(Align::Left, &" ".repeat(96));
            bst.invert();
            bst.word_wrap();

            bst.set_font(2);
            bst.text(
                Align::Left,
                "\n\n\n\
                 Use the number keys or function keys F1-F10 (use F10 for 0).\n",
            );
            bst.text(
                Align::Left,
                &format!(
                    "Recovery key ID (to identify your key): \
                     {:08X}-{:04X}-{:04X}-{:04X}{:08X}\n\n",
                    random(),
                    random() & 0xFFFF,
                    random() & 0xFFFF,
                    random() & 0xFFFF,
                    random()
                ),
            );

            match random() % 4 {
                0 => bst.text(
                    Align::Left,
                    "Bitlocker needs your recovery key to unlock your \
                     drive because Secure Boot policy has unexpectedly \
                     changed.\n\n",
                ),
                1 => bst.text(
                    Align::Left,
                    "Bitlocker needs your recovery key to unlock your \
                     PC's configuration has changed. This may have happened \
                     because a disc or USB device ws inserted. Removing it \
                     and restarting your PC may fix this problem.\n\n",
                ),
                3 => bst.text(
                    Align::Left,
                    "Bitlocker needs your recovery key to unlock your drive \
                     because Secure Boot has been disabled. Either Secure \
                     Boot must be re-enabled, or BitLocker must be suspended \
                     for Windows to start normally.\n\n",
                ),
                _ => {}
            }

            bst.text(
                Align::Left,
                "Here's how to find your key:\n\
                 - Sign in on another device and go to: https://www.jwz.org/\
                 \n- For more information go to: https://www.jwz.org/xscreensaver/",
            );
        }

        _ => {
            bst.text(Align::Left, "Recovery\n\n");
            bst.set_font(0);
            bst.text(
                Align::Left,
                "There are no more BitLocker recovery options on your PC\n\n",
            );
            bst.set_font(2);
            bst.text(
                Align::Left,
                "You'll need to use the recovery tools on your installation \
                 media. If you don't have any installation media (like a \
                 disc or USB device), contact your system administrator or \
                 PC manufacturer.",
            );
            bottom = "Press Enter to try again\n\
                      Press F8 for Startup Settings\n\
                      Press Esc for UEFI Firmware Settings";
        }
    }

    bst.set_font(0);
    let y = bst.height - top - (bst.font_b.ascent() + bst.font_b.descent());
    bst.moveto(0, y);
    bst.text(Align::Left, bottom);

    bst.clear(d);
    bst
}

/// Windows NT 3.1 to 4.0.
fn windows_nt(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#0000AA", BLACK), f);

    match random() % 4 {
        0..=2 => {
            bst.text(
                Align::Left,
                "*** STOP: 0x0000001E (0x80000003,0x80106fc0,0x8025ea21,0xfd6829e8)\n\
                 Unhandled Kernel exception c0000047 from fa8418b4 (8025ea21,fd6829e8)\n\
                 \n\
                 Dll Base Date Stamp - Name             Dll Base Date Stamp - Name\n\
                 80100000 2be154c9 - ntoskrnl.exe       80400000 2bc153b0 - hal.dll\n\
                 80258000 2bd49628 - ncrc710.sys        8025c000 2bd49688 - SCSIPORT.SYS \n\
                 80267000 2bd49683 - scsidisk.sys       802a6000 2bd496b9 - Fastfat.sys\n\
                 fa800000 2bd49666 - Floppy.SYS         fa810000 2bd496db - Hpfs_Rec.SYS\n\
                 fa820000 2bd49676 - Null.SYS           fa830000 2bd4965a - Beep.SYS\n\
                 fa840000 2bdaab00 - i8042prt.SYS       fa850000 2bd5a020 - SERMOUSE.SYS\n\
                 fa860000 2bd4966f - kbdclass.SYS       fa870000 2bd49671 - MOUCLASS.SYS\n\
                 fa880000 2bd9c0be - Videoprt.SYS       fa890000 2bd49638 - NCC1701E.SYS\n\
                 fa8a0000 2bd4a4ce - Vga.SYS            fa8b0000 2bd496d0 - Msfs.SYS\n\
                 fa8c0000 2bd496c3 - Npfs.SYS           fa8e0000 2bd496c9 - Ntfs.SYS\n\
                 fa940000 2bd496df - NDIS.SYS           fa930000 2bd49707 - wdlan.sys\n\
                 fa970000 2bd49712 - TDI.SYS            fa950000 2bd5a7fb - nbf.sys\n\
                 fa980000 2bd72406 - streams.sys        fa9b0000 2bd4975f - ubnb.sys\n\
                 fa9c0000 2bd5bfd7 - usbser.sys         fa9d0000 2bd4971d - netbios.sys\n\
                 fa9e0000 2bd49678 - Parallel.sys       fa9f0000 2bd4969f - serial.SYS\n\
                 faa00000 2bd49739 - mup.sys            faa40000 2bd4971f - SMBTRSUP.SYS\n\
                 faa10000 2bd6f2a2 - srv.sys            faa50000 2bd4971a - afd.sys\n\
                 faa60000 2bd6fd80 - rdr.sys            faaa0000 2bd49735 - bowser.sys\n\
                 \n\
                 Address dword dump Dll Base                                      - Name\n\
                 801afc20 80106fc0 80106fc0 00000000 00000000 80149905 : \
                 fa840000 - i8042prt.SYS\n\
                 801afc24 80149905 80149905 ff8e6b8c 80129c2c ff8e6b94 : \
                 8025c000 - SCSIPORT.SYS\n\
                 801afc2c 80129c2c 80129c2c ff8e6b94 00000000 ff8e6b94 : \
                 80100000 - ntoskrnl.exe\n\
                 801afc34 801240f2 80124f02 ff8e6df4 ff8e6f60 ff8e6c58 : \
                 80100000 - ntoskrnl.exe\n\
                 801afc54 80124f16 80124f16 ff8e6f60 ff8e6c3c 8015ac7e : \
                 80100000 - ntoskrnl.exe\n\
                 801afc64 8015ac7e 8015ac7e ff8e6df4 ff8e6f60 ff8e6c58 : \
                 80100000 - ntoskrnl.exe\n\
                 801afc70 80129bda 80129bda 00000000 80088000 80106fc0 : \
                 80100000 - ntoskrnl.exe\n\
                 \n\
                 Kernel Debugger Using: COM2 (Port 0x2f8, Baud Rate 19200)\n\
                 Restart and set the recovery options in the system control panel\n\
                 or the /CRASHDEBUG system start option. If this message reappears,\n\
                 contact your system administrator or technical support group.",
            );
            bst.line_delay = 750;
        }
        _ => {
            bst.text(
                Align::Center,
                "Microsoft (R) Windows NT (R) Version 5.0 (Build 1796)\n\
                 1 System Processor [128 MB Memory] MultiProcessor Kernel\n\
                 \n\
                 *** STOP: 0x0000006B (0xC000003A, 0x00000002,0x00000000,0x00000000)\n\
                 PROCESS1_INITIALIZATION_FAILED\n\
                 \n\
                 If this is the first time you[ve seen this Stop error screen,\n\
                 restart your computer. If this screen appears again, follow\n\
                 these steps:\n\
                 \n\
                 Check to make sure any new hardware or software is properly installed.\n\
                 If this is a new installation, ask your hardware or software\
                 \x20manufacturer\n\
                 for any Windows NT updates you might need.\n\
                 \n\
                 If problems continue, disable or remove any newly installed hardware\n\
                 or software. Disable BIOS memory options such as caching or shadowing.\n\
                 If you need to use Safe Mode to remove or disable components, restart\n\
                 your computer, press F8 to select Advanced Startup Options, and then\n\
                 select Safe Mode.\n\
                 \n\
                 Refer to your Getting Started manual for more information on\n\
                 troubleshooting Stop errors.\n\
                 \n",
            );
        }
    }

    bst.clear(d);
    bst
}

fn windows_2k(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#0000AA", BLACK), f);

    match random() % 4 {
        0..=2 => {
            bst.text(
                Align::Left,
                "*** STOP: 0x000000D1 (0xE1D38000,0x0000001C,0x00000000,0xF09D42DA)\n\
                 DRIVER_IRQL_NOT_LESS_OR_EQUAL \n\
                 \n\
                 *** Address F09D42DA base at F09D4000, DateStamp 39f459ff - CRASHDD.SYS\n\
                 \n\
                 Beginning dump of physical memory\n",
            );
            bst.pause(4_000_000);
            bst.text(
                Align::Left,
                "Physical memory dump complete. Contact your system administrator or\n\
                 technical support group.\n",
            );

            bst.left_margin = 40;
            bst.y = bst.line_height() * 10;
            bst.line_delay = 750;
        }
        _ => {
            bst.text(
                Align::Center,
                "\n\n\n\
                 *** STOP: 0x0000007B (0xF641F84C,0xC00000034,0x00000000,0x00000000)\n\
                 INACCESSIBLE_BOOT_DEVICE\n\
                 \n\
                 If this is the first time you[ve seen this Stop error screen,\n\
                 restart your computer. If this screen appears again, follow\n\
                 these steps:\n\
                 \n\
                 Check for viruses on your computer. Remove any newly installed\n\
                 hard drives or hard drive controllers. Chcek your hard drive\n\
                 to make sure it is properly configured and terminated.\n\
                 Run CHKDSK /F to check for hard drive corruption, and then\n\
                 restart your computer.\n\
                 \n\
                 Refer to your Getting Started manual for more information on\n\
                 troubleshooting Stop errors.\n\
                 \n\
                 \n",
            );
        }
    }

    bst.clear(d);
    bst
}

fn windows_me(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#0000AA", BLACK), f);

    match random() % 3 {
        0 | 1 => {
            bst.text(
                Align::Left,
                "Windows protection error.  You need to restart your computer.\n\n\
                 System halted.",
            );
            bst.cursor(false, 120_000, 999_999);

            bst.left_margin = 40;
            bst.y = (bst.height - bst.yoff - bst.line_height() * 3) / 2;
        }
        _ => {
            bst.invert();
            bst.text(Align::Center, "Windows\n");
            bst.invert();
            bst.text(
                Align::Center,
                "\n\
                 An error has occurred. To continue:\n\
                 \n\
                 Press Enter to return to Windows, or\n\
                 \n\
                 Press CTRL+ALT+DEL to restart your computer. If you do this,\n\
                 you will lose any unsaved information in all open applications.\n\
                 \n\
                 Error: 0E : 015F : FOAD0D0D\n\
                 \n",
            );
            bst.text(Align::Center, "Press any key to continue ");
            bst.cursor(false, 120_000, 999_999);
            bst.y = (bst.height - bst.yoff - bst.line_height() * 11) / 2;
        }
    }

    bst.clear(d);
    bst
}

fn windows_xp(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#0000AA", BLACK), f);

    match random() % 6 {
        /* From Wm. Rhodes */
        0..=2 => {
            bst.text(
                Align::Left,
                "A problem has been detected and windows has been shut down to prevent \
                 damage\n\
                 to your computer.\n\
                 \n\
                 If this is the first time you've seen this Stop error screen,\n\
                 restart your computer. If this screen appears again, follow\n\
                 these steps:\n\
                 \n\
                 Check to be sure you have adequate disk space. If a driver is\n\
                 identified in the Stop message, disable the driver or check\n\
                 with the manufacturer for driver updates. Try changing video\n\
                 adapters.\n\
                 \n\
                 Check with your hardware vendor for any BIOS updates. Disable\n\
                 BIOS memory options such as caching or shadowing. If you need\n\
                 to use Safe Mode to remove or disable components, restart your\n\
                 computer, press F8 to select Advanced Startup Options, and then\n\
                 select Safe Mode.\n\
                 \n\
                 Technical information:\n\
                 \n\
                 *** STOP: 0x0000007E (0xC0000005,0xF88FF190,0x0xF8975BA0,0xF89758A0)\n\
                 \n\
                 \n\
                 ***  EPUSBDSK.sys - Address F88FF190 base at FF88FE000, datestamp \
                 3b9f3248\n\
                 \n\
                 Beginning dump of physical memory\n",
            );
            bst.pause(4_000_000);
            bst.text(
                Align::Left,
                "Physical memory dump complete.\n\
                 Contact your system administrator or technical support group for \
                 further\n\
                 assistance.\n",
            );
        }
        /* Windows XP/Vista/7 */
        3 => {
            bst.text(
                Align::Left,
                "STOP: C0000021a {Fatal System Error}\n\
                 The Session Manager Initialization system process terminated\
                 \x20unexpectedly\n\
                 with a status of 0x00000001 (0x00000000 0x00000000).\n\
                 The system has been shut down.\n",
            );
        }
        /* Windows CE */
        4 => {
            bst.text(
                Align::Left,
                "A error has occurred and Windows CE has been shut down to prevent\n\
                 damage to your computer.\n\
                 If you will try to restart your computer, press Ctrl+Alt+Delete.\n\
                 \n\
                 Technical information:\n\
                 \n\
                 *** STOP: 0x0004c2 (inaccessible embedded device)\n\
                 \n\
                 \n\
                 The computer will restart automatically\n\
                 after 23 seconds.\n",
            );
        }
        /* Windows 8 */
        _ => {
            bst.text(
                Align::Left,
                "A problem has been detected and windows has been shut down to prevent\n\
                 damage to your computer.\n\
                 \n\
                 SYSTEM_SERVICE_EXCEPTION\n\
                 \n\
                 If this is the first time you[ve seen this Stop error screen,\n\
                 restart your computer. If this screen appears again, follow\n\
                 these steps:\n\
                 \n\
                 Check to make sure any new hardware or software is properly installed.\n\
                 If this is a new installation, ask your hardware or software\
                 \x20manufacturer\n\
                 for any Windows NT updates you might need.\n\
                 \n\
                 If problems continue, disable or remove any newly installed hardware\n\
                 or software. Disable BIOS memory options such as caching or shadowing.\n\
                 If you need to use Safe Mode to remove or disable components, restart\n\
                 your computer, press F8 to select Advanced Startup Options, and then\n\
                 select Safe Mode.\n\
                 \n\
                 Technical information:\n\
                 \n\
                 *** STOP: 0x0000003B (0x00000000c000005,0xFFFFF880041C9062,\
                 0xFFFFF88002E22F60,0x0000000000000000(\n\
                 \n\
                 ***   dxgmms1.sys - Address FFFFF880041C9062 base at FFFFF8800418F000,\
                 \x20DateStamp 4cdb7409\n\
                 \n\
                 Collecting data for crash dump ...\n",
            );
            bst.pause(4_000_000);
            bst.text(Align::Left, "Initializing disk for for crash dump ...\n");
        }
    }

    bst.clear(d);
    bst
}

/// The "RSOD" that appeared with "Windows Longhorn 5048.050401-0536_x86fre",
/// as reported by <http://joi.ito.com/RedScreen.jpg>.
fn windows_lh(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, color("#AA0000", BLACK), f);

    let (fg, bg) = (bst.fg, bst.bg);
    let bg2 = color("#AAAAAA", WHITE);

    bst.color(bg, bg2);
    bst.text_full(Align::Center, "Windows Boot Error\n");
    bst.color(fg, bg);
    bst.text(
        Align::Center,
        "\n\
         Windows Boot Manager has experienced a problem.\n\
         \n\
         \n\
         \x20   Status: 0xc000000f\n\
         \n\
         \n\
         \n\
         \x20   Info: An error occurred transferring exectuion.\n\
         \n\
         \n\
         You can try to recover the system with the Microsoft Windows \
         System Recovery\n\
         Tools. (You might need to restart the system manually.)\n\
         \n\
         If the problem continues, please contact your system administrator \
         or computer\n\
         manufacturer.\n",
    );
    let (x, y) = (
        bst.left_margin + bst.xoff,
        bst.height - bst.yoff - bst.font.descent(),
    );
    bst.moveto(x, y);
    bst.color(bg, bg2);
    bst.text_full(Align::Left, " SPACE=Continue\n");

    bst.y = bst.font.ascent();

    bst.clear(d);
    bst
}

/// Windows XP's safe-mode menu, on a machine with a dead bus line: every
/// fourth character has had one of its bits knocked out.
fn windows_safe(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);

    const LINES: &[&str] = &[
        "We apologize for the inconvenience, but Windows did not start successfully.  A\n",
        "recent hardware or software change might have caused this.\n",
        "\n",
        "If your computer stopped responding, restarted unexpectedly, or was\n",
        "automatically shut down to protect your files and folders, choose Last Known\n",
        "Good Configuration to refert to the most recent settings that worked.\n",
        "\n",
        "If a previous startup attempt was interrupted due to a power failure or because\n",
        "the Power or Reset button was pressed, or if you aren't sure what caused the\n",
        "problem, choose Start Windows Normally.\n",
        "\n",
        "    Safe Mode\n",
        "    Safe Mode with Networking\n",
        "    Safe Mode with Command Prompt\n",
        "\n",
        "    Last Known Good Configuration (your most recent settings that worked)\n",
        "\n",
        "*    Start Windows Normally\n",
        "\n",
        "Use the up and down arrow keys to move the highlight to your choice.\n",
        "Seconds until Windows starts:  ",
    ];

    let mut bit = random() % 8; /* Dead bus line */
    if bit != 0 && !random().is_multiple_of(4) {
        bit = 3;
    }

    /* 1: Stapt Windous Nmrmally
       2: Start Windoss Nkrmahly
       3: Start Wandows Ngrmadly
       4: Stabt Windogs Normally
       5: StaRt WIndoWs NOrmaLly
       6: Sta2t W)ndo7s N/rma,ly
    */
    let (fg, bg) = (bst.fg, bst.bg);
    bst.color(fg, bg);
    for line in LINES {
        let inv = line.starts_with('*');
        let l: String = line[usize::from(inv)..]
            .bytes()
            .enumerate()
            .map(|(i, c)| {
                let col = i + 1;
                if c > b' ' && col.is_multiple_of(4) {
                    let c = c & !(1 << bit);
                    char::from(if c == 0 { b' ' } else { c })
                } else {
                    char::from(c)
                }
            })
            .collect();
        if inv {
            bst.invert();
        }
        bst.text(Align::Left, &l);
        if inv {
            bst.invert();
        }
    }

    for i in (0..=9).rev() {
        bst.text(Align::Left, &format!("\u{8}{i}"));
        bst.pause(1_000_000);
    }

    bst.clear(d);
    bst
}

/// A Secure Boot violation, in a box.
///
/// Upstream draws the box out of the Unicode box-drawing block and wonders in
/// a comment how likely it is that the font has those; ours certainly does
/// not, so this is the ASCII box upstream keeps beside it for that case.
fn windows_sb(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);

    let (fg, bg) = (bst.fg, bst.bg);
    let bg2 = color("#FF0000", WHITE);
    let line_height = bst.font_a.ascent() + bst.font_a.descent();
    let top = (bst.height - line_height * 8) / 2;
    let left = (bst.width - bst.font.char_width() * 46) / 2;

    bst.moveto(left, top);
    bst.color(fg, bg2);
    let w = bst.width;
    bst.margins(left, w);

    bst.text(
        Align::Left,
        "|----------- Secure Boot Violation ----------|\n\
         |                                            |\n\
         |  Invalid signature detected. Check Secure  |\n\
         |            Boot Policy in Setup            |\n\
         |                                            |\n\
         |--------------------------------------------|\n\
         |                     ",
    );
    bst.color(fg, bg);
    bst.text(Align::Left, "Ok");
    bst.color(fg, bg2);
    bst.text(
        Align::Left,
        "                     |\n\
         |--------------------------------------------|\n",
    );

    bst.clear(d);
    bst
}

/// Lump all of the 2K-ish crashes together and select them randomly.
fn windows_other(d: &mut Dpy, f: &Fonts) -> Bst {
    match random() % 6 {
        0 => windows_2k(d, f),
        1 => windows_me(d, f),
        2 => windows_xp(d, f),
        3 => windows_lh(d, f),
        4 => windows_safe(d, f),
        _ => windows_sb(d, f),
    }
}

/// What a graphics card does to whatever was on the screen when it gives up:
/// a random plane mask and the same block copied over and over at an offset.
fn blitdamage(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);

    let (w, h) = (bst.width, bst.height);

    bst.gc.set_plane_mask(random());

    let steps = 50;
    // Upstream divides these by `random() % 1 + 1`, which is always one.
    let chunk_w = w;
    let chunk_h = h;
    let mut delta_x = 0;
    let mut delta_y = 0;
    if random() & 0x1000 != 0 {
        delta_y = (random() % 600) as i32;
    }
    if delta_y == 0 || random() & 0x2000 != 0 {
        delta_x = (random() % 600) as i32;
    }
    let (src_x, src_y) = (0, 0);
    let (mut x, mut y) = (0, 0);

    bst.img();
    for _ in 0..steps {
        if x + chunk_w > w {
            x -= w;
        } else {
            x += delta_x;
        }

        if y + chunk_h > h {
            y -= h;
        } else {
            y += delta_y;
        }

        bst.copy(src_x, src_y, chunk_w, chunk_h, x, y);
        bst.pause(1000);
    }
    bst
}

/// VMware ESXi 7.0 on a 64-bit Arm host (actually ESXi-Arm Fling), by
/// Andrei E. Warkentin.
///
/// The log line is stamped with the time of day, which the host supplies; it
/// has no idea what the date is, so that part is made up.
fn vmware_arm(d: &mut Dpy, f: &Fonts) -> Bst {
    let fg = WHITE;
    let fg2 = color("#FFFF00", WHITE);
    let mut bst = Bst::new(d, fg, color("#555555", BLACK), f);

    let font_height = bst.line_height();
    let psod_bg = color("#a700a8", BLACK);
    let term_bg = color("#555555", BLACK);
    let term_fg2 = color("#AAAAAA", WHITE);
    let term_fg3 = color("#FF5757", WHITE);
    let dbg_bg = BLACK;
    let dbg_fg = color("#ABABAB", WHITE);

    bst.wrap();
    bst.margins(0, 0);
    bst.vert_margins(0, 0);

    /* statusterm. */
    bst.truncate();
    bst.color(term_bg, fg);
    let (w, h) = (bst.width, bst.height);
    bst.rect(true, 0, 0, w, h);
    bst.moveto(0, font_height * 6);
    bst.color(fg, term_bg);
    bst.text(
        Align::Left,
        "                VMware ESXi 7.0.0 (VMKernel Release Build 19076756)",
    );
    bst.moveto(0, font_height * 8);
    bst.color(fg, term_bg);
    bst.text(Align::Left, "                PINE64 Quartz64 Model A");
    bst.moveto(0, font_height * 10);
    bst.color(term_fg2, term_bg);
    bst.text(Align::Left, "                ARM Limited Cortex-A55 r2p0");
    bst.moveto(0, font_height * 11);
    bst.color(term_fg2, term_bg);
    bst.text(Align::Left, "                7.7 GiB Memory");

    let msg_row = 1 + bst.height / (font_height * 2);
    if msg_row > 11 {
        bst.wrap();
        bst.moveto(0, font_height * msg_row);
        bst.color(term_fg3, term_bg);
        let t = d.wall_clock() as i64;
        bst.text(
            Align::Left,
            &format!(
                "2026-01-01T{:02}:{:02}:{:02}.000Z",
                t / 3600 % 24,
                t / 60 % 60,
                t % 60
            ),
        );
        bst.text(
            Align::Left,
            " cpu0:65802)Failed to verify signatures of the following vib(s):\
             \x20[bnxtnet bnxtroce brcmfcoe brcmnvmefc elx-esx-libelxima.so\
             \x20eslxiscsi elxnet ena esx base esx-dvfilter-generic-fastpath\
             \x20esx-ui esx-update i40en i40iwn iavmd igbn iser$",
        );
    }

    bst.pause(10_000_000);

    /* Now the PSOD. */
    bst.truncate();
    bst.color(psod_bg, fg);
    bst.rect(true, 0, 0, w, h);
    let ascent = bst.font.ascent();
    bst.moveto(0, ascent);
    bst.color(fg2, psod_bg);
    bst.text(
        Align::Left,
        "VMware ESX 7.0.0 [Releasebuild-19076756 aarch64]\n",
    );
    bst.color(fg, psod_bg);
    bst.line_delay(1000);
    bst.text(
        Align::Left,
        "EXCVEC_CUREL_SP_EL0_SYNCH Exception 0 in world 131126:HELPER_UPLIN\
         \x20(ec 0x25 il 1 iss 0x47 far_el1 0x315d3541b8 far_el2 0x4501843b1000)\
         \nTTB=0x12ade8000\
         \nCurrentEL=2 SP_EL0 DAIF\
         \nSCTLR_EL2=0x30c0180d sa0 SA C a M\
         \n[ 0]     4501843b0000 [ 1]                0 [ 2]\
         \x20            1000 [ 3]     4501843b1000\
         \n[ 4]     451a01b1be20 [ 5]                0 [ 6]\
         \x20    4305be607a5a [ 7]     4200400001c0\
         \n[ 8]                0 [ 9]     451a01b1be20 [10]\
         \x20               1 [11]                1\
         \n[12]         ffffed40 [13]     420040000080 [14]\
         \x20    41fffa5d7000 [15]     41fffa5d7c20\
         \n[16]                1 [17]                4 [18]\
         \x20    41fffa5d7c08 [19]     451a01b1be70\
         \n[20]     43024240b680 [21]     4305be601900 [22]\
         \x20    4305be6012c0 [23]     41ffd0c00000\
         \n[24]     4305be601220 [25]          bad0001 [26]\
         \x20               0 [27]     43006fc01220\
         \n[28]     4303d5001220 [29]                0 [30]     42003b3ceb6c\
         \n[pc]     42003a344d54 [sp]     451a01b1bdd0 [psr]        20000248\
         \n*PCPU0:131126/HELPER_UPLINK_ASYNC_CALL_QUEUE\
         \nPCPU  0: SUUU\
         \nCode start: 0x42003a200000 VMK uptime: 0:00:05:35.425\
         \n0x451a01b1bdd0:[0x42003a344d54]vmk_Memset@vmkernel#nover+0x28\
         \x20stack: 0x42003b3cfa48\
         \n0x451a01b1bdd0:[0x42003b3ceb68]EQOSEnable@(eqos)#<None>+0xe0\
         \x20stack: 0x42003b3cfa48\
         \n0x451a01b1be20:[0x42003b3c9220]SETHUplAssociate@(eqos)#<None>+0x88\
         \x20stack: 0x43024240b680\
         \n0x451a01b1be80:[0x42003a48a078]UplinkDeviceAssociateAsyncCB\
         @vmkernel#nover+0x50 stack: 0x43024240bb08\
         \n0x451a01b1bed0:[0x42003a555a6c]UplinkAsyncProcessCallsHelperCB\
         @vmkernel#nover+0x12c stack: 0x451a01b21000\
         \n0x451a01b1bf20:[0x42003a2fd510]HelperQueueFunc@vmkernel#nover+0x174\
         \x20stack: 0x451a01b21100\
         \n0x451a01b1bfe0:[0x42003a59e4fc]CpuSched_StartWorld@vmkernel#nover+0x70\
         \x20stack: 0x0\
         \n0x451a01b1c000:[0x42003a5ec610]CpuSched_UseMwaitCallback\
         @vmkernel#nover+0x8 stack: 0x0\
         \nNo place on disk to dump data.\
         \nCoredump to file: /vmfs/volumes/3a4fcb25-5f6ca096-c940-70886b86100c\
         /vmkdump/00000000-0000-0000-0000-000000000000.dumpfile.\
         \nFaulting world regs (01/15)",
    );
    for (msg, usec) in [
        ("\nVmm code/data (02/15)", 300_000),
        ("\nVmk code/rodata/stack (03/15)", 300_000),
        ("\nVmk data/heap (04/15)", 300_000),
        ("\nPCPU (05/15)", 300_000),
        ("\nWorld-specific data (06/15)", 300_000),
        ("\nVASpace (08/15)", 300_000),
        ("\nPFrame (09/15)", 1_000_000),
        ("\nMemXferFs (11/15)", 1_000_000),
        ("\nDump Files (13/15)", 300_000),
        ("\nCollecting userworld dumps (14/15)", 600_000),
        (
            "\nFinalized dump header (15/15) FileDump: Successful.",
            600_000,
        ),
        (
            "\nNo port for remote debugger. Press \"Escape\" for local debugger.",
            10_000,
        ),
    ] {
        bst.pause(usec);
        bst.text(Align::Left, msg);
    }
    bst.pause(1_000_000);

    /* Local debugger. */
    bst.color(dbg_bg, dbg_fg);
    bst.rect(true, 0, 0, w, h);
    bst.moveto(0, ascent);
    bst.text(Align::Left, "vmkernel debugger (h for help)");
    let descent = bst.font.descent();
    bst.rect(true, 0, ascent + descent / 2, w, ascent);
    bst.color(dbg_fg, dbg_bg);
    bst.text(Align::Left, "\n[PCPU2] VMKDBG> _");
    for _ in 0..2 {
        bst.pause(1_000_000);
        bst.text(Align::Left, "\u{8}h_");
        bst.pause(10_000);
        bst.text(
            Align::Left,
            "\u{8} \nh       : help\
             \nreboot  : reboot\
             \nlivedump: Create live coredump without crashing system\
             \nl       : display vmkernel log\
             \np       : display content of symbol\
             \ns       : display storage dump+boot info\
             \nx       : display 32-bit content of address\
             \nx/N     : display N bytes of content at address\
             \ng port  : bind remote debugger to port (com1 or com2)\
             \nbt N    : show backtrace for CPU N\
             \nq       : quit debug terminal\
             \n[PCPU2] VMKDBG> _",
        );
    }
    for (msg, usec) in [
        ("\u{8}re_", 100_000),
        ("\u{8}b_", 500_000),
        ("\u{8}o_", 50_000),
        ("\u{8}o_", 50_000),
        ("\u{8}t_", 50_000),
    ] {
        bst.pause(usec);
        bst.text(Align::Left, msg);
    }
    bst.pause(2_000_000);
    bst.color(dbg_bg, dbg_fg);
    bst.rect(true, 0, 0, w, h);
    bst.pause(1_000_000);

    bst.clear(d);
    bst
}

/// The DVD player's idle screen, bouncing its logo about and inverting the
/// colours every time it turns.
fn dvd(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, WHITE, BLACK, f);
    let scale = 0.15;
    let steps = 10000;
    let mut speed = 1;

    bst.clear(d);

    let mut art = Art::load(crate::images::bsod::DVD);
    let (mut pix_w, mut pix_h) = art.as_ref().map_or((64, 32), |a| (a.width(), a.height()));
    while f64::from(pix_w) <= f64::from(bst.width) * scale
        && f64::from(pix_h) <= f64::from(bst.height) * scale
    {
        art = art.map(|a| a.doubled());
        pix_w *= 2;
        pix_h *= 2;
        speed *= 2;
    }

    if let Some(a) = art {
        bst.mask = a.mask.map(std::rc::Rc::new);
        bst.pixmap = Some(a.image);
    }
    let mut x = (random() % (bst.width - pix_w).max(1) as u32) as i32;
    let mut y = (random() % (bst.height - pix_h).max(1) as u32) as i32;
    let mut dx = speed * if random() & 1 != 0 { 1 } else { -1 };
    let mut dy = speed * if random() & 1 != 0 { 1 } else { -1 };

    bst.invert();
    for _ in 0..steps {
        bst.rect(true, x, y, pix_w, pix_h);
        if x + dx < 0 || x + dx + pix_w > bst.width {
            dx = -dx;
        }
        if y + dy < 0 || y + dy + pix_h > bst.height {
            dy = -dy;
        }
        x += dx;
        y += dy;
        bst.pixmap_at(0, 0, pix_w, pix_h, x, y);
        bst.pause(1_000_000 / 30);
    }
    bst
}

/// The TiVo, which would like you to leave it alone for three hours.
fn tivo(d: &mut Dpy, f: &Fonts) -> Bst {
    let mut bst = Bst::new(d, color("#B8E6BA", WHITE), color("#339020", BLACK), f);
    let line_height = bst.line_height();
    let char_width = bst.font.char_width();

    let left = ((bst.width - char_width * 44) / 2).max(0);
    let top = ((bst.height - line_height * 15) / 2).max(0);

    bst.clear(d);

    bst.margins(left, left);
    bst.moveto(left, top);

    bst.set_font(1);
    bst.text(Align::Left, "\nA severe error has occurred.\n\n");
    bst.set_font(0);
    bst.text(
        Align::Left,
        "Please leave the Receiver plugged in and connected\n\
         to the phone line for the next three hours while the\n\
         Receiver attempts to repair itself.",
    );
    bst.set_font(1);
    bst.text(
        Align::Left,
        "\n\nDO NOT UNPLUG OR RESTART\nTHE RECEIVER.\n\n",
    );
    bst.set_font(0);
    bst.text(
        Align::Left,
        "If, after three hours, the Receiver does not restart\n\
         itself, call Customer Care.",
    );

    bst.pause(1_000_000 * 60);
    bst
}

/// Error message for corrupted (and therefore presumed bootleg) cartridges.
///
/// A variant crash has a second box above the English text warning you in
/// Japanese, which upstream leaves out because the font it uses has no
/// Japanese in it. Neither has this one.
fn nintendo(d: &mut Dpy, f: &Fonts) -> Bst {
    let bg = color("#F76D0A", BLACK);
    let bg2 = color("#085C89", BLACK);
    let fg = color("#EEAACF", WHITE);
    let mut bst = Bst::new(d, fg, bg, f);

    let line_height = bst.line_height();
    let char_width = bst.font.char_width();
    let left = ((bst.width - char_width * 30) / 2).max(0);
    let top = ((bst.height - line_height * 9) / 2).max(0);
    let left2 = (left - char_width * 4).clamp(0, char_width * 8);
    let top2 = (top - line_height).clamp(0, line_height * 10);

    bst.clear(d);

    bst.color(bg2, bg);
    bst.rect(
        true,
        left2,
        top2 - line_height * 2,
        bst.width - left2 * 2,
        bst.height - top2 * 2 + line_height * 2,
    );

    bst.margins(left, left);
    bst.moveto(left, top - line_height / 2);

    bst.set_font(1);
    bst.color(bg, bg2);
    bst.text(Align::Center, "WARNING");
    bst.set_font(0);
    bst.color(fg, bg2);
    bst.text(
        Align::Left,
        "\n\n\
         IT IS A SERIOUS CRIME\n\
         TO COPY VIDEO GAMES\n\
         ACCORDING TO COPYRIGHT LAW.\n\
         PLEASE REFER TO\n\
         YOUR NINTENDO GAME\n\
         INSTRUCTION BOOKLET\n\
         FOR FURTHER INFORMATION.",
    );

    bst.pause(1_000_000 * 60);
    bst
}

const MODES: &[Mode] = &[
    Mode {
        name: "Windows",
        fun: windows_31,
        fonts: NONE,
    },
    Mode {
        name: "VMware",
        fun: vmware,
        fonts: NONE,
    },
    Mode {
        name: "VMwareArm",
        fun: vmware_arm,
        fonts: NONE,
    },
    Mode {
        name: "NT",
        fun: windows_nt,
        fonts: NONE,
    },
    Mode {
        name: "Win2K",
        fun: windows_other,
        fonts: NONE,
    },
    Mode {
        name: "Bitlocker",
        fun: bitlocker,
        fonts: [
            "Arial 24, Helvetica 24",
            "Arial 24, Helvetica 24",
            "Arial 36, Helvetica 36",
            "Arial 18, Helvetica 18",
        ],
    },
    Mode {
        name: "GLaDOS",
        fun: glados,
        fonts: NONE,
    },
    Mode {
        name: "SCO",
        fun: sco,
        fonts: NONE,
    },
    Mode {
        name: "SparcLinux",
        fun: sparc_linux,
        fonts: NONE,
    },
    Mode {
        name: "BSD",
        fun: bsd,
        fonts: NONE,
    },
    Mode {
        name: "BlitDamage",
        fun: blitdamage,
        fonts: NONE,
    },
    Mode {
        name: "HVX",
        fun: hvx,
        fonts: NONE,
    },
    Mode {
        name: "HPUX",
        fun: hpux,
        fonts: NONE,
    },
    Mode {
        name: "OS390",
        fun: os390,
        fonts: NONE,
    },
    Mode {
        name: "Tru64",
        fun: tru64,
        fonts: NONE,
    },
    Mode {
        name: "MSDOS",
        fun: msdos,
        fonts: NONE,
    },
    Mode {
        name: "Amiga",
        fun: amiga,
        fonts: NONE,
    },
    Mode {
        name: "Atari",
        fun: atari,
        fonts: NONE,
    },
    Mode {
        name: "Mac",
        fun: mac,
        fonts: NONE,
    },
    Mode {
        name: "MacsBug",
        fun: macsbug,
        fonts: [
            "Monaco 8, Courier Bold 8",
            "Monaco 14, Courier Bold 14",
            "",
            "",
        ],
    },
    Mode {
        name: "Mac1",
        fun: mac1,
        fonts: NONE,
    },
    Mode {
        name: "MacX",
        fun: macx,
        fonts: ["Courier Bold 10", "Courier Bold 14", "", ""],
    },
    Mode {
        name: "ATM",
        fun: atm,
        fonts: NONE,
    },
    Mode {
        name: "Gnome",
        fun: gnome,
        fonts: ["Helvetica Bold 13", "Helvetica Bold 13", "Helvetica 13", ""],
    },
    Mode {
        name: "DVD",
        fun: dvd,
        fonts: NONE,
    },
    Mode {
        name: "Tivo",
        fun: tivo,
        fonts: ["Helvetica Bold 16", "Helvetica Bold 28", "", ""],
    },
    Mode {
        name: "Nintendo",
        fun: nintendo,
        fonts: [
            "Classic Console 18, Courier Bold 18",
            "Classic Console 40, Courier Bold 40",
            "",
            "",
        ],
    },
];

/// A mode that does not name a font of its own.
const NONE: [&str; 4] = ["", "", "", ""];

/* --------------------------------------------------------------- driver */

struct Bsod {
    mode_duration: f64,
    /// The one mode to show, when the panel asks for one.
    only: Option<usize>,
    which: Option<usize>,
    next_one: Option<usize>,
    /// When the running mode was launched, or `None` between modes. Upstream
    /// keeps a `time_t` and uses zero for this, which works because the clock
    /// it compares against is the wall clock and so never near zero.
    start: Option<f64>,
    delay_remaining: i64,
    bst: Option<Bst>,
}

impl Screenhack for Bsod {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // Upstream loops here rather than returning, for every instruction that
        // asks for no delay at all.
        for _ in 0..10_000 {
            let time_left = match self.start {
                Some(start) => start + self.mode_duration - d.time,
                None => f64::NEG_INFINITY,
            };

            if self.delay_remaining > 0 {
                /* Rather than returning a multi-second delay from draw(),
                meaning "don't call us again for N seconds", quantize that down
                to 1/10th second intervals so that it's more responsive to
                rotate/reshape events. */
                let inc = 10_000;
                let this_delay = self.delay_remaining.min(inc);
                self.delay_remaining = (self.delay_remaining - inc).max(0);
                if time_left < 0.0 && self.start.is_some() {
                    self.delay_remaining = 0;
                }
                return this_delay as u32;
            }

            if self.bst.is_none() && time_left > 0.0 {
                /* run completed; wait out the delay */
                self.start = None;
                let time_left = time_left.min(5.0); /* Boooored now */
                self.delay_remaining = (1_000_000.0 * time_left) as i64;
                continue;
            }

            if self.bst.is_some() {
                /* sub-mode currently running */
                let this_delay = if time_left > 0.0 {
                    self.bst.as_mut().and_then(|b| b.pop(d))
                } else {
                    None
                };

                match this_delay {
                    Some(0) => continue, /* no delay, not expired: stay here */
                    Some(n) => {
                        self.delay_remaining = n;
                        continue;
                    }
                    None => {
                        /* sub-mode run completed or expired */
                        self.bst = None;
                        return 0;
                    }
                }
            }

            self.launch(d);
            return 0;
        }
        0
    }

    fn reshape(&mut self, d: &mut Dpy, _width: i32, _height: i32) {
        /* just restart this mode when the window is resized. */
        self.bst = None;
        self.start = None;
        self.delay_remaining = 0;
        self.next_one = self.which;
        d.win().clear(BLACK);
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        /* pick a new mode and restart when mouse clicked, or certain keys
        typed. */
        if !screenhack_event_helper(event) {
            return false;
        }
        self.bst = None;
        self.start = None;
        self.delay_remaining = 0;
        d.win().clear(BLACK);
        true
    }
}

impl Bsod {
    /// Launch a new sub-mode.
    fn launch(&mut self, d: &mut Dpy) {
        let n = MODES.len();
        let which = if let Some(next) = self.next_one.take() {
            next
        } else if let Some(only) = self.only {
            only
        } else if n < 2 {
            0
        } else {
            let mut i = self.which;
            while i == self.which {
                i = Some((random() as usize) % n);
            }
            i.unwrap_or(0)
        };
        self.which = Some(which);

        let fonts = Fonts::resolve(&MODES[which], d.height());
        let mut bst = (MODES[which].fun)(d, &fonts);
        self.start = Some(d.time);

        /* Reset the structure run state to the beginning, and do some
        sanitization of the cursor position before the first run. */
        bst.pos = Some(0);
        bst.x = bst.left_margin + bst.xoff;
        bst.current_left = bst.x;
        let top = bst.top_margin + bst.yoff + bst.font.ascent();
        if bst.y < top {
            bst.y = top;
        }
        bst.queue.push(Ev::Eof);
        self.bst = Some(bst);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mode_duration = f64::from(d.res.int("delay").max(3));

    let want = d.res.string("doOnly").to_string();
    let only = MODES
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(&want));

    Box::new(Bsod {
        mode_duration,
        only,
        which: None,
        next_one: None,
        start: None,
        delay_remaining: 0,
        bst: None,
    })
}

const DEFAULTS: &[&str] = &[
    "*delay:		   45",
    "*doOnly:		   ",
    ".foreground:	   White",
    ".background:	   Black",
    "*font:		   PxPlus IBM VGA8 12",
    "*bigFont:		   PxPlus IBM VGA8 22",
    "*fontB:		   ",
    "*fontC:		   ",
];

const ONLY: &[SelectItem] = &[
    SelectItem {
        value: "",
        label: "All of them",
    },
    SelectItem {
        value: "Windows",
        label: "Windows",
    },
    SelectItem {
        value: "VMware",
        label: "VMware",
    },
    SelectItem {
        value: "VMwareArm",
        label: "VMware on Arm",
    },
    SelectItem {
        value: "NT",
        label: "Windows NT",
    },
    SelectItem {
        value: "Win2K",
        label: "Windows 2000 and later",
    },
    SelectItem {
        value: "Bitlocker",
        label: "BitLocker",
    },
    SelectItem {
        value: "GLaDOS",
        label: "GLaDOS",
    },
    SelectItem {
        value: "SCO",
        label: "SCO",
    },
    SelectItem {
        value: "SparcLinux",
        label: "Linux (SPARC)",
    },
    SelectItem {
        value: "BSD",
        label: "BSD",
    },
    SelectItem {
        value: "BlitDamage",
        label: "Blit damage",
    },
    SelectItem {
        value: "HVX",
        label: "HVX",
    },
    SelectItem {
        value: "HPUX",
        label: "HP-UX",
    },
    SelectItem {
        value: "OS390",
        label: "OS/390",
    },
    SelectItem {
        value: "Tru64",
        label: "Tru64",
    },
    SelectItem {
        value: "MSDOS",
        label: "MS-DOS",
    },
    SelectItem {
        value: "Amiga",
        label: "Amiga",
    },
    SelectItem {
        value: "Atari",
        label: "Atari ST",
    },
    SelectItem {
        value: "Mac",
        label: "Macintosh",
    },
    SelectItem {
        value: "MacsBug",
        label: "MacsBug",
    },
    SelectItem {
        value: "Mac1",
        label: "Macintosh bomb",
    },
    SelectItem {
        value: "MacX",
        label: "Mac OS X",
    },
    SelectItem {
        value: "ATM",
        label: "Cash machine",
    },
    SelectItem {
        value: "Gnome",
        label: "GNOME",
    },
    SelectItem {
        value: "DVD",
        label: "DVD player",
    },
    SelectItem {
        value: "Tivo",
        label: "TiVo",
    },
    SelectItem {
        value: "Nintendo",
        label: "Nintendo",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Duration", 3.0, 300.0, 1.0, 0, "45"),
    Opt::select("doOnly", "Show", ONLY, ""),
];

pub static DEF: SaverDef = SaverDef {
    slug: "bsod",
    label: "BSOD",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=YIqbMCfR3r0"),
        blurb: "Blue Screen of Death: the finest in personal computer emulation.",
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

    /// The collection-wide tests only ever see whichever machine the dice
    /// picked, so this puts every one of them on the screen in turn.
    #[test]
    fn every_machine_crashes() {
        for m in MODES {
            let query = format!("doOnly={}", m.name);
            let mut r = start(StartArgs::new(1024, 768, &query, 20260809));
            let mut best = 0;
            for i in 0..1500 {
                r.step();
                if i % 25 == 0 {
                    best = best.max(lit(&r));
                    if best > 500 {
                        break;
                    }
                }
            }
            assert!(best > 500, "{} drew almost nothing ({best} pixels)", m.name);
        }
    }

    /// Every mode has to survive a window too small to lay itself out in: the
    /// margins and the row it starts on are worked out from the height, and
    /// several of them come out negative.
    #[test]
    fn a_tiny_window_is_survivable() {
        for m in MODES {
            let query = format!("doOnly={}", m.name);
            for (w, h) in [(1, 1), (16, 8), (200, 40)] {
                let mut r = start(StartArgs::new(w, h, &query, 7));
                for _ in 0..200 {
                    r.step();
                }
            }
        }
    }

    /// How many pixels are not the background, counted against the first row
    /// so a mode that fills the screen with its own colour still counts as
    /// having drawn.
    fn lit(r: &Runner) -> usize {
        let fb = r.dpy.win_ref();
        let bg = fb.get_pixel(0, 0);
        fb.pixels().iter().filter(|p| **p != bg).count()
    }
}
