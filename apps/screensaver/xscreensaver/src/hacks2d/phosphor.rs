//! Port of `hacks/phosphor.c`.
//!
//! ```text
//! xscreensaver, Copyright © 1999-2025 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Phosphor -- simulate a glass tty with long-sustain phosphor.
//! Written by Jamie Zawinski
//! ```
//!
//! A terminal on a tube that will not let go of what it was told. Every cell
//! has a state: blank, flaring, lit, or one of twenty stages of fading, and
//! each frame moves the fading ones one stage further down a ramp of colours
//! between the foreground and the background. Text that has scrolled away is
//! still there for a second afterwards, in green going to black.
//!
//! Nothing is drawn with a font. Each character is turned into a bitmap once,
//! at startup, by reading the compiled-in glyph sheet and drawing every
//! horizontal run of ink as a thick line at `scale` times the size. That is
//! what makes the letters look like they were drawn by a beam rather than
//! printed: the strokes are round-capped and they bleed. It is done twice per
//! character, once with a thicker pen and once with a thinner one, and the two
//! are drawn in different colours from the fade ramp, which is the glow around
//! the stroke.
//!
//! What it is displaying comes through [`crate::runtime::tty`], so escape
//! sequences do what they should: a program that clears the screen and draws a
//! box gets a cleared screen and a box.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, rgb_to_hsv, unrgb};
use crate::runtime::tty::{TTY_BOLD, TTY_INVERSE, TTY_ITALIC, TTY_SYMBOLS, Tty};
use crate::runtime::{
    About, Dpy, Fb, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent,
    color, png,
};
use std::rc::Rc;

const BLANK: usize = 0;
const FLARE: usize = 1;
const NORMAL: usize = 2;
const FADE: usize = 3;
const STATE_MAX: usize = FADE;

/// The glyph sheet is 256 characters of seven pixels by ten.
const FONT_CELL_W: i32 = 7;
const FONT_H: i32 = 10;

/// One cell of the screen.
#[derive(Clone, Copy, Default)]
struct Cell {
    c: u8,
    state: usize,
    changed: bool,
    invert_p: bool,
    symbol_p: bool,
}

/// One character at one size: the stroke, and the slightly fatter stroke drawn
/// underneath it that becomes the glow.
struct Glyph {
    /// The fatter one, which is also what says where the cell is lit at all.
    pixmap: Pixmap,
    /// The thinner one, used as a clip mask for the brighter colour.
    pixmap2: Rc<Fb>,
    blank_p: bool,
}

struct Phosphor {
    tty: Tty,
    grid_width: i32,
    grid_height: i32,
    char_width: i32,
    char_height: i32,
    scale: i32,
    ticks: usize,
    cells: Vec<Cell>,

    /// Every character, plain and inverted, in the text font and the line
    /// drawing one: four sets of 256.
    chars: Vec<Glyph>,
    ichars: Vec<Glyph>,
    schars: Vec<Glyph>,
    sichars: Vec<Glyph>,

    /// One GC per state of the fade, from the flare down to the background.
    gcs: Vec<Gc>,

    cursor_on: bool,
    cursor_x: i32,
    cursor_y: i32,
    /// Upstream blinks the cursor on a toolkit timer; here it is read off the
    /// clock, which is the same thing without one.
    cursor_blink: f64,
    cursor_phase: f64,

    delay: u32,
}

impl Phosphor {
    fn cell_at(&self, x: i32, y: i32) -> usize {
        (self.grid_width * y + x) as usize
    }

    fn glyph(&self, c: u8, invert_p: bool, symbol_p: bool) -> &Glyph {
        let set = match (symbol_p, invert_p) {
            (true, true) => &self.sichars,
            (true, false) => &self.schars,
            (false, true) => &self.ichars,
            (false, false) => &self.chars,
        };
        &set[c as usize]
    }

    /// Move every fading cell one stage further down, and turn a flare into
    /// steady light.
    fn decay(&mut self) {
        let ticks = self.ticks;
        for cell in self.cells.iter_mut() {
            if cell.state == FLARE {
                cell.state = NORMAL;
                cell.changed = true;
            } else if cell.state >= FADE {
                cell.state += 1;
                if cell.state >= ticks {
                    cell.state = BLANK;
                    cell.c = b' ';
                }
                cell.changed = true;
            }
        }
    }

    /// One character into the terminal, then copy whatever it did to the grid
    /// on to the tube.
    fn print_char(&mut self, c: u32) {
        self.tty.print(c);
        // Anything the terminal wants to say back would go to the program on
        // the other end; there is none, so it is dropped.
        self.tty.replies.clear();

        for y in 0..self.tty.height.min(self.grid_height) {
            for x in 0..self.tty.width.min(self.grid_width) {
                let tcell = self.tty.grid[(self.tty.width * y + x) as usize];
                let at = self.cell_at(x, y);

                let mut inv_p = tcell.flags & (TTY_BOLD | TTY_ITALIC | TTY_INVERSE) != 0;
                let sym_p = tcell.flags & TTY_SYMBOLS != 0;
                if self.tty.inverse_p {
                    inv_p = !inv_p;
                }

                if self.cells[at].c == 0 {
                    self.cells[at].c = b' ';
                }
                let tc = if tcell.c == 0 { b' ' as u32 } else { tcell.c };

                // Neither this nor apple2 can show anything but Latin1, so
                // that is what a code point above it is folded to.
                let latin1 = if tc < 256 { tc as u8 } else { b' ' };

                let old = self.cells[at];
                let blank_p = self.glyph(old.c, old.invert_p, old.symbol_p).blank_p;
                if !blank_p && latin1 == b' ' && !inv_p {
                    // Replacing a character with a blank: fade out what was
                    // there rather than dropping it.
                    if old.state == FLARE || old.state == NORMAL {
                        self.cells[at].state = FADE;
                    }
                    self.cells[at].changed = true;
                } else if u32::from(old.c) != tc
                    || old.state >= FADE
                    || old.invert_p != inv_p
                    || old.symbol_p != sym_p
                {
                    self.cells[at].invert_p = inv_p;
                    self.cells[at].symbol_p = sym_p;
                    // Upstream: FLARE here "looks bad when scrolling".
                    self.cells[at].state = NORMAL;
                    self.cells[at].changed = true;
                    self.cells[at].c = latin1;
                }
            }
        }

        // If the cursor has moved, light the new cell and flare it.
        if self.cursor_x != self.tty.x || self.cursor_y != self.tty.y {
            let old = self.cell_at(self.cursor_x, self.cursor_y);
            let new = self.cell_at(
                self.tty.x.clamp(0, self.grid_width - 1),
                self.tty.y.clamp(0, self.grid_height - 1),
            );
            self.cells[old].changed = true;
            self.cells[new].changed = true;
            // Do not bring a fading character back under the new cursor.
            if self.cells[new].state >= FADE {
                self.cells[new].c = b' ';
            }
            self.cells[new].state = FLARE;
            self.cursor_x = self.tty.x.clamp(0, self.grid_width - 1);
            self.cursor_y = self.tty.y.clamp(0, self.grid_height - 1);
        }
    }

    fn update_display(&mut self, d: &mut Dpy, changed_only: bool) {
        let width = self.char_width * self.scale;
        let height = self.char_height * self.scale;

        for y in 0..self.grid_height {
            for x in 0..self.grid_width {
                let at = self.cell_at(x, y);
                let cell = self.cells[at];
                if changed_only && !cell.changed {
                    continue;
                }

                let cursor_p = x == self.cursor_x && y == self.cursor_y;
                let mut st = cell.state;
                let mut inv_p = cell.invert_p;
                let mut c = cell.c;
                if cursor_p {
                    if self.cursor_on {
                        inv_p = !inv_p;
                    }
                    if st >= FADE {
                        c = b' ';
                    }
                    if st == BLANK || st >= FADE {
                        st = NORMAL;
                    }
                }
                if c == 0 {
                    c = b' ';
                }

                let (tx, ty) = (x * width, y * height);
                let blank_p = self.glyph(c, inv_p, cell.symbol_p).blank_p;

                if blank_p || (cell.state == BLANK && !cursor_p) {
                    let gc = self.gcs[BLANK].clone();
                    d.win().fill_rectangle(&gc, tx, ty, width, height);
                } else {
                    // The fatter stroke goes down in a dimmer colour and the
                    // thinner one over it in the brighter, which is the glow.
                    let gc2 = if st + 2 < self.ticks {
                        Some(self.gcs[st + 2].clone())
                    } else {
                        None
                    };
                    let gc3 = gc2.clone().unwrap_or_else(|| self.gcs[st].clone());
                    let g = self.glyph(c, inv_p, cell.symbol_p);
                    let (pix, mask) = (g.pixmap.clone(), Rc::clone(&g.pixmap2));
                    d.win().copy_plane(&gc3, &pix, 0, 0, width, height, tx, ty);
                    if gc2.is_some() {
                        let mut gc1 = self.gcs[st].clone();
                        gc1.set_clip_mask(mask).set_clip_origin(tx, ty);
                        d.win().fill_rectangle(&gc1, tx, ty, width, height);
                    }
                }
                self.cells[at].changed = false;
            }
        }
    }

    /// Rebuild the grid for a new window size. Returns whether anything moved.
    fn resize_grid(&mut self, d: &Dpy) -> bool {
        let ow = self.grid_width;
        let oh = self.grid_height;
        let gw = (d.width() / (self.char_width * self.scale)).max(2);
        let gh = (d.height() / (self.char_height * self.scale)).max(2);
        if gw == ow && gh == oh {
            return false;
        }

        let mut ncells = vec![Cell::default(); (gw * gh) as usize];
        for y in 0..oh.min(gh) {
            for x in 0..ow.min(gw) {
                ncells[(gw * y + x) as usize] = self.cells[(ow * y + x) as usize];
            }
        }
        for c in ncells.iter_mut() {
            c.changed = true;
        }
        self.cells = ncells;
        self.grid_width = gw;
        self.grid_height = gh;
        self.cursor_x = self.cursor_x.min(gw - 1);
        self.cursor_y = self.cursor_y.min(gh - 1);
        true
    }
}

impl Screenhack for Phosphor {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // The cursor is on for two thirds of its cycle and off for one.
        let phase = (d.time - self.cursor_phase).rem_euclid(self.cursor_blink * 3.0);
        let on = phase < self.cursor_blink * 2.0;
        if on != self.cursor_on {
            self.cursor_on = on;
            let at = self.cell_at(self.cursor_x, self.cursor_y);
            self.cells[at].changed = true;
        }

        self.update_display(d, true);
        self.decay();

        if let Some(c) = d.text_getc() {
            self.print_char(u32::from(c));
        }
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, _width: i32, _height: i32) {
        if !self.resize_grid(d) {
            return;
        }
        self.tty.resize(self.grid_width, self.grid_height);
        d.text_reshape(self.grid_width, self.grid_height);
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

/// Read the compiled-in glyph sheet into one bit per pixel: set where there is
/// ink, which is where the sheet is both opaque and black.
///
/// For the line-drawing set, upstream shifts the DEC Special Graphics
/// characters up from the control range where the sheet keeps them, into the
/// positions the terminal asks for.
fn capture_font_bits(symbol_p: bool) -> Vec<bool> {
    let w = (FONT_CELL_W * 256) as usize;
    let h = FONT_H as usize;
    let mut ink = vec![false; w * h];
    let Some((im, mask)) = png::decode(crate::images::FONT_6X10) else {
        return ink;
    };
    if im.width() != w as i32 || im.height() != h as i32 {
        return ink;
    }

    for y in 0..h {
        for x in 0..w {
            let opaque = mask
                .as_ref()
                .is_none_or(|m| m.get_pixel(x as i32, y as i32) != 0);
            let black = im.get_pixel(x as i32, y as i32) & 0x00FF_FFFF == 0;
            ink[y * w + x] = opaque && black;
        }
    }

    if symbol_p {
        let cw = FONT_CELL_W as usize;
        let from = cw;
        let to = cw * 0x60;
        let len = cw * 31;
        for y in 0..h {
            for x in to..to + len {
                ink[y * w + x] = ink[y * w + x - (to - from)];
            }
        }
    }
    ink
}

/// Turn one character of the sheet into the pair of strokes it is drawn with.
///
/// Every horizontal run of ink becomes one thick round-capped line, which is
/// why the letters look drawn rather than printed.
fn char_to_glyph(ink: &[bool], c: usize, invert_p: bool, scale: i32) -> Glyph {
    let sheet_w = (FONT_CELL_W * 256) as usize;
    let width = scale * (FONT_CELL_W - 1);
    let height = scale * FONT_H;
    let mut p = Fb::new_bitmap(width, height);
    let mut p2 = Fb::new_bitmap(width, height);

    let mut gc = Gc::new(1, 0);
    gc.set_line_width({
        let w = (f64::from(scale) * 1.3) as i32;
        if w == scale { w + 1 } else { w }
    });
    let mut gc2 = Gc::new(1, 0);
    gc2.set_line_width({
        let mut w = (f64::from(scale) * 0.8) as i32;
        if w >= scale {
            w = scale - 1;
        }
        w.max(1)
    });

    let from = FONT_CELL_W as usize * c;
    let to = FONT_CELL_W as usize * (c + 1);
    let mut blank_p = !invert_p;

    for y in 0..FONT_H as usize {
        let mut x1 = from;
        while x1 < to {
            let mut pix = ink[y * sheet_w + x1];
            if invert_p {
                pix = !pix;
            }
            if pix {
                let xoff = scale / 2;
                let mut x2 = x1;
                while x2 < to {
                    let mut p = ink[y * sheet_w + x2];
                    if invert_p {
                        p = !p;
                    }
                    if !p {
                        break;
                    }
                    x2 += 1;
                }
                x2 -= 1;
                let (ax, ay) = ((x1 - from) as i32 * scale + xoff, y as i32 * scale);
                let bx = (x2 - from) as i32 * scale + xoff;
                p.draw_line(&gc, ax, ay, bx, ay);
                p2.draw_line(&gc2, ax, ay, bx, ay);
                x1 = x2;
                blank_p = false;
            }
            x1 += 1;
        }
    }

    Glyph {
        pixmap: p,
        pixmap2: Rc::new(p2),
        blank_p,
    }
}

/// Blend the foreground most of the way toward the background, which is where
/// the fade ramp starts.
fn scale_color_channel(ch1: u16, ch2: u16) -> u16 {
    ((u32::from(ch1) * 100 + u32::from(ch2) * 156) >> 8) as u16
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut scale = d.res.int("phosphorScale").max(1);
    let mut ticks = STATE_MAX + d.res.int("ticks").max(1) as usize;
    if d.width() > 2560 || d.height() > 2560 {
        scale *= 2; /* Retina displays */
    }

    let char_width = FONT_CELL_W - 1;
    let char_height = FONT_H;
    let grid_width = (d.width() / (char_width * scale)).max(2);
    let grid_height = (d.height() / (char_height * scale)).max(2);

    let fg: Pixel = d.res.pixel("foreground");
    let bg: Pixel = d.res.pixel("background");
    let (fr, fg_, fb) = unrgb(fg);
    let (br, bg_, bb) = unrgb(bg);
    let to16 = |v: u8| u16::from(v) * 257;

    // A ramp from most of the way to the background, down to it.
    let (h1, s1, v1) = rgb_to_hsv(
        scale_color_channel(to16(fr), to16(br)),
        scale_color_channel(to16(fg_), to16(bg_)),
        scale_color_channel(to16(fb), to16(bb)),
    );
    let (h2, s2, v2) = rgb_to_hsv(to16(br), to16(bg_), to16(bb));
    // Avoid rainbow effects when fading to black, grey or white.
    let h2 = if s2 < 0.003 { h1 } else { h2 };
    let h1 = if s1 < 0.003 { h2 } else { h1 };

    let ncolors = (ticks - STATE_MAX).max(1);
    let colors = color::make_color_ramp(h1, s1, v1, h2, s2, v2, ncolors, false);
    ticks = colors.len() + STATE_MAX;

    // If the foreground is brighter than the background the flare is white;
    // otherwise there is no flare.
    let (_, _, fv) = rgb_to_hsv(to16(fr), to16(fg_), to16(fb));
    let flare = if v2 <= fv { color::WHITE } else { fg };

    let mut gcs = vec![Gc::new(bg, bg); ticks];
    gcs[FLARE] = Gc::new(flare, bg);
    gcs[NORMAL] = Gc::new(fg, bg);
    for (i, c) in colors.iter().enumerate() {
        gcs[STATE_MAX + i] = Gc::new(c.pixel, bg);
    }

    let text_ink = capture_font_bits(false);
    let sym_ink = capture_font_bits(true);
    let build = |ink: &[bool], invert: bool| -> Vec<Glyph> {
        (0..256)
            .map(|c| char_to_glyph(ink, c, invert, scale))
            .collect()
    };

    let mut st = Phosphor {
        tty: Tty::new(grid_width, grid_height),
        grid_width,
        grid_height,
        char_width,
        char_height,
        scale,
        ticks,
        cells: vec![Cell::default(); (grid_width * grid_height) as usize],
        chars: build(&text_ink, false),
        ichars: build(&text_ink, true),
        schars: build(&sym_ink, false),
        sichars: build(&sym_ink, true),
        gcs,
        cursor_on: true,
        cursor_x: 0,
        cursor_y: 0,
        cursor_blink: f64::from(d.res.int("cursor").max(1)) / 1000.0,
        cursor_phase: 0.0,
        delay: d.res.int("delay").max(0) as u32,
    };
    st.cells[0].changed = true;
    d.text_reshape(grid_width, grid_height);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    ".background:		   Black",
    ".foreground:		   #00FF00",
    "*fpsSolid:		   true",
    "*phosphorScale:	   6",
    "*ticks:		   20",
    "*delay:		   50000",
    "*cursor:		   333",
    "*program:		   xscreensaver-text",
    "*font:		   (builtin)",
];

const COLORS: &[SelectItem] = &[
    SelectItem {
        value: "#00FF00",
        label: "Green",
    },
    SelectItem {
        // DarkOrange is probably the closest named colour.
        value: "#FF7900",
        label: "Amber",
    },
    SelectItem {
        value: "white",
        label: "White",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "50000").inverted(),
    Opt::spin("phosphorScale", "Font scale", 1.0, 20.0, "6"),
    Opt::slider("ticks", "Fade", 1.0, 100.0, 1.0, 0, "20").inverted(),
    Opt::select("foreground", "Colour", COLORS, "#00FF00"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "phosphor",
    label: "Phosphor",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=G6ZWTrl7pV0"),
        blurb: "A glass teletype with a tube that will not let go of the text.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
