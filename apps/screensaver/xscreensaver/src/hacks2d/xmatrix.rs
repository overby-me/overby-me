//! Port of `hacks/xmatrix.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1999-2018 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Matrix -- simulate the text scrolls from the movie "The Matrix".
//!
//! In 1999, the movie people released their own Mac OS 9 / Windows screen
//! saver, but it was very inaccurate and did not match what the computer
//! screens in the movie did.  Mine is better.
//!
//! See also my `glmatrix' program, which does a 3D rendering of the similar
//! effect that appeared in the title sequence of the movies.
//!
//!     ==========================================================
//!
//!         NOTE:
//!
//!         People just love to hack on this one.  I get sent
//!         patches to this all the time saying, ``here, I made
//!         it better!''  Mostly this hasn't been true.
//!
//!         In particular, note that the characters in the movie
//!         were, in fact, low resolution and somewhat blurry/
//!         washed out.  They also definitely scrolled a
//!         character at a time, not a pixel at a time.
//!
//!         And keep in mind that this program emulates the
//!         behavior of the computer screens that were visible
//!         in the movies -- not the behavior of the effects in
//!         the title sequences.  "GLMatrix" does that.
//!
//!     ==========================================================
//! ```
//!
//! The glyphs are a bitmap, sixteen by thirteen of them, in two sheets: one
//! plain and one glowing. Nothing is rendered; a character is a rectangle
//! copied out of a sheet, which is why the letters are blurry in exactly the
//! way the film's were.
//!
//! Each column has a feeder that pushes glyphs in from the top or the bottom
//! with a randomised throttle, so the columns fall at different speeds and stop
//! and start. In Matrix mode the sheet is mirrored, because the katakana in the
//! film were reversed, and a few cells are marked as spinners and reroll their
//! glyph every frame.
//!
//! Then there are the set pieces: the trace program counting out a phone
//! number, SYSTEM FAILURE in its bevelled box, Neo being told to wake up, and
//! Trinity's nmap session against the power grid. Each is a script typed a
//! character at a time, with its own delays, and a control byte at the start of
//! a line for "type this one hesitantly" or "type this one in bold".

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, SelectItem, StartArgs,
    XEvent, color::rgb, png, random, random_below, screenhack_event_helper,
};

const CHAR_COLS: i32 = 16;
const CHAR_ROWS: i32 = 13;
const CURSOR_GLYPH: u16 = 97;

/// Larger numbers mean more variability between columns.
const BUF_SIZE: usize = 200;

#[derive(Clone, Copy, Default)]
struct Cell {
    glow: i32,
    /// Note: nine bit characters, and zero means empty.
    glyph: u16,
    changed: bool,
    spinner: bool,
}

#[derive(Clone, Copy, Default)]
struct Feeder {
    pipe_loc: usize,
    remaining: i32,
    throttle: i32,
    y: i32,
}

const MATRIX_ENCODING: &[usize] = &[
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202,
    203, 204, 205, 206, 207,
];
const DECIMAL_ENCODING: &[usize] = &[16, 17, 18, 19, 20, 21, 22, 23, 24, 25];
const HEX_ENCODING: &[usize] = &[
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 33, 34, 35, 36, 37, 38,
];
const BINARY_ENCODING: &[usize] = &[16, 17];
const DNA_ENCODING: &[usize] = &[33, 35, 39, 52];

/// Where each byte lives in the glyph sheet. The control range and the gap
/// above ASCII map to 3, which is a space.
const CHAR_MAP: [u8; 256] = {
    let mut m = [3u8; 256];
    let mut i = 32;
    while i < 128 {
        m[i] = (i - 32) as u8;
        i += 1;
    }
    let mut i = 160;
    while i < 256 {
        m[i] = (i - 64) as u8;
        i += 1;
    }
    // One deliberate exception upstream: 0xF3 shares a glyph with 0xC3.
    m[243] = 195;
    m
};

/// ASCII mode maps each printable byte to its own glyph and everything else to
/// the first one.
const ASCII_ENCODING: [usize; 128] = {
    let mut m = [0usize; 128];
    let mut i = 33;
    while i < 128 {
        m[i] = i - 32;
        i += 1;
    }
    m
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    DrainTraceA,
    TraceTextA, /* Call trans opt: received. */
    TraceA,     /* (31_) 5__-0_9_ */
    TraceDone,
    DrainTraceB,
    TraceTextB, /* Call trans opt: received. */
    TraceB,
    TraceFail, /* System Failure */
    DrainKnock,
    Knock, /* Wake up, Neo... */
    DrainNmap,
    Nmap, /* Starting nmap V. 2.54BETA25 */
    DrainMatrix,
    Matrix,
    Dna,
    Binary,
    Dec,
    Hex,
    Ascii,
}

impl Mode {
    fn is_drain(self) -> bool {
        matches!(
            self,
            Mode::DrainTraceA
                | Mode::DrainTraceB
                | Mode::DrainKnock
                | Mode::DrainNmap
                | Mode::DrainMatrix
        )
    }

    fn is_trace(self) -> bool {
        matches!(self, Mode::TraceA | Mode::TraceB | Mode::TraceDone)
    }
}

struct XMatrix {
    draw_gc: Gc,
    erase_gc: Gc,
    scratch_gc: Gc,
    grid_width: i32,
    grid_height: i32,
    char_width: i32,
    char_height: i32,
    cells: Vec<Cell>,
    background: Vec<Cell>,
    feeders: Vec<Feeder>,
    nspinners: i32,
    knock_knock_p: bool,
    insert_top_p: bool,
    insert_bottom_p: bool,
    use_pipe_p: bool,
    mode: Mode,
    /// What to fall back to after a set piece.
    def_mode: Mode,

    /// A ring of characters read from the text source.
    buf: Vec<u8>,
    do_fill_buff: bool,
    buf_done: usize,
    buf_pos: usize,
    start_reveal_back_p: bool,
    back_text_full_p: bool,
    back_line: Vec<u8>,
    back_pos: usize,
    back_y: i32,

    /// The phone number being traced, as negative digits until each is found.
    tracing: Vec<i8>,
    density: i32,

    /// The script being typed, and how far through it we are.
    typing: Option<&'static [u8]>,
    typing_scroll_p: bool,
    typing_cursor_p: bool,
    typing_bold_p: bool,
    typing_stutter_p: bool,
    typing_left_margin: i32,
    typing_char_delay: i64,
    typing_line_delay: i64,
    typing_delay: i64,

    /// Upstream blinks the cursor on an X toolkit timer, 666 on and 333 off.
    /// Here it is read off the clock, which is the same cycle without one.
    cursor_enabled: bool,
    cursor_on: bool,
    cursor_phase: f64,
    cursor_x: i32,
    cursor_y: i32,

    plain: Pixmap,
    glow: Pixmap,
    images_flipped_p: bool,

    glyph_map: &'static [usize],
    /// The five greens of the SYSTEM FAILURE box, outside in.
    colors: [Pixel; 5],
    delay: i64,
}

impl XMatrix {
    fn cell_index(&self, x: i32, y: i32) -> usize {
        (self.grid_width * y + x) as usize
    }

    // ---- The glyph sheets ------------------------------------------------

    /// Mirror every glyph in place. The Matrix characters are reversed
    /// katakana, and the sheet holds them the right way round.
    fn flip_images(&mut self, flipped_p: bool) {
        if flipped_p == self.images_flipped_p {
            return;
        }
        self.images_flipped_p = flipped_p;
        let ww = self.char_width;
        for image in [&mut self.plain, &mut self.glow] {
            for y in 0..image.height() {
                for x in 0..CHAR_COLS {
                    let row: Vec<Pixel> =
                        (0..ww).map(|xx| image.get_pixel(x * ww + xx, y)).collect();
                    for xx in 0..ww {
                        image.put_pixel(x * ww + xx, y, row[(ww - xx - 1) as usize]);
                    }
                }
            }
        }
    }

    // ---- Text ------------------------------------------------------------

    /// One character out of the text source into the ring.
    fn fill_input(&mut self, d: &mut Dpy) {
        let load_bytes = if self.buf_done > self.buf_pos {
            (self.buf_done - self.buf_pos) as i32 - 1
        } else {
            ((BUF_SIZE - self.buf_pos) + self.buf_done) as i32 - 1
        };

        let mut n = 0;
        if load_bytes > 0 {
            match d.text_getc() {
                Some(c) => {
                    self.buf[self.buf_pos] = c;
                    n = 1;
                }
                None => n = -1,
            }
        }

        if n > 0 {
            self.do_fill_buff = false;
            self.buf_pos += 1;
            if self.buf_pos > BUF_SIZE {
                self.buf.copy_within(BUF_SIZE..self.buf_pos, 0);
            }
            self.buf_pos %= BUF_SIZE;
        } else {
            // Nothing to be had: assume the end, and start again.
            self.do_fill_buff = true;
        }
    }

    // ---- Cursor ----------------------------------------------------------

    fn set_cursor(&mut self, on: bool, now: f64) {
        self.cursor_enabled = on;
        if on {
            self.cursor_phase = now;
        }
        self.set_cursor_1(on);
    }

    fn set_cursor_1(&mut self, on: bool) {
        let changed = self.cursor_on != on;
        self.cursor_on = on;
        if changed && self.cursor_x >= 0 && self.cursor_y >= 0 {
            let i = self.cell_index(self.cursor_x, self.cursor_y);
            if i < self.cells.len() {
                self.cells[i].glow = 0;
                self.cells[i].changed = true;
            }
        }
    }

    /// Tick the blink. On for two thirds of a second, off for one.
    fn blink_cursor(&mut self, now: f64) {
        if !self.cursor_enabled {
            self.set_cursor_1(false);
            return;
        }
        let phase = (now - self.cursor_phase).rem_euclid(0.999);
        self.set_cursor_1(phase < 0.666);
    }

    // ---- Modes -----------------------------------------------------------

    fn init_spinners(&mut self) {
        for c in self.cells.iter_mut() {
            c.spinner = false;
        }
        let mut i = self.nspinners;
        while i > 1 {
            i -= 1;
            let x = random_below(self.grid_width);
            let y = random_below(self.grid_height);
            let idx = self.cell_index(x, y);
            self.cells[idx].spinner = true;
        }
    }

    fn clear_spinners(&mut self) {
        for c in self.cells.iter_mut() {
            if c.spinner {
                c.spinner = false;
                c.changed = true;
            }
        }
    }

    /// Turn the phone number into digits waiting to be discovered. Each is
    /// stored negative until the trace finds it.
    fn init_trace(&mut self) {
        self.tracing = PHONE
            .bytes()
            .filter(|c| c.is_ascii_digit())
            .map(|c| -(c as i8))
            .collect();
        self.glyph_map = DECIMAL_ENCODING;
    }

    /// Empty the screen by feeding nothing in from the top.
    fn init_drain(&mut self, now: f64) {
        self.set_cursor(false, now);
        self.cursor_x = -1;
        self.cursor_y = -1;
        for f in self.feeders.iter_mut() {
            f.y = -1;
            f.remaining = 0;
            f.throttle = 0;
        }
        // Turn off all the spinners, else they never go away.
        self.clear_spinners();
    }

    fn screen_blank_p(&self) -> bool {
        self.cells.iter().all(|c| c.glyph == 0)
    }

    fn set_mode(&mut self, mode: Mode, now: f64) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.typing = None;

        match mode {
            Mode::Matrix => {
                self.glyph_map = MATRIX_ENCODING;
                self.flip_images(true);
                self.init_spinners();
            }
            Mode::Dna => {
                self.glyph_map = DNA_ENCODING;
                self.flip_images(false);
            }
            Mode::Binary => {
                self.glyph_map = BINARY_ENCODING;
                self.flip_images(false);
            }
            Mode::Hex => {
                self.glyph_map = HEX_ENCODING;
                self.flip_images(false);
            }
            Mode::Ascii => {
                self.glyph_map = &ASCII_ENCODING;
                self.flip_images(false);
            }
            Mode::Dec | Mode::TraceA | Mode::TraceB | Mode::Nmap | Mode::Knock => {
                self.glyph_map = DECIMAL_ENCODING;
                self.flip_images(false);
            }
            Mode::TraceTextA | Mode::TraceTextB => {
                self.flip_images(false);
                self.init_trace();
            }
            m if m.is_drain() => self.init_drain(now),
            _ => {}
        }
    }

    // ---- Feeding ---------------------------------------------------------

    fn insert_glyph(&mut self, glyph: u16, x: i32, y: i32) {
        let bottom_feeder_p = y >= 0;
        if y >= self.grid_height {
            return;
        }
        let to = if bottom_feeder_p {
            self.cell_index(x, y)
        } else {
            for y in (1..self.grid_height).rev() {
                let from = self.cell_index(x, y - 1);
                let to = self.cell_index(x, y);
                self.cells[to].glyph = self.cells[from].glyph;
                self.cells[to].glow = self.cells[from].glow;
                self.cells[to].changed = true;
            }
            x as usize
        };

        self.cells[to].glyph = glyph;
        self.cells[to].changed = true;
        if self.cells[to].glyph == 0 {
        } else if bottom_feeder_p {
            let n = if self.tracing.is_empty() { 2 } else { 4 };
            self.cells[to].glow = 1 + random_below(n);
        } else {
            self.cells[to].glow = 0;
        }
    }

    fn place_back_char(&mut self, textc: u8, x: i32, y: i32) {
        if x >= 0 && y >= 0 && x < self.grid_width && y < self.grid_height {
            let i = self.cell_index(x, y);
            let mut glyph = u16::from(CHAR_MAP[textc as usize]) + 1;
            if glyph == 0 || glyph == 3 {
                glyph = u16::from(CHAR_MAP[32]) + 1;
            }
            self.background[i].glyph = glyph;
            self.background[i].changed = true;
        }
    }

    fn place_back_text(&mut self, text: &[u8], x: i32, y: i32) {
        for (i, c) in text.iter().enumerate() {
            self.place_back_char(*c, x + i as i32, y);
        }
    }

    /// Build up a page of the piped text behind the rain, a line at a time.
    fn place_back_pipe(&mut self, textc: u8) {
        let mut new_line = false;
        self.back_line[self.back_pos] = textc;
        if textc == b'\n' {
            self.back_line[self.back_pos] = 0;
            new_line = true;
        } else if self.back_pos as i32 > self.grid_width - 4 || self.back_pos >= BUF_SIZE {
            self.back_pos += 1;
            self.back_line[self.back_pos] = 0;
            new_line = true;
        } else {
            self.back_pos += 1;
        }
        if new_line {
            let line: Vec<u8> = self.back_line[..self.back_pos].to_vec();
            let startx = (self.grid_width >> 1) - (line.len() as i32 >> 1);
            let y = self.back_y;
            self.place_back_text(&line, startx, y);
            self.back_pos = 0;
            self.back_y += 1;
            if self.back_y >= self.grid_height - 1 {
                self.back_y = 1;
                self.back_text_full_p = true;
                self.start_reveal_back_p = true;
            }
        }
    }

    fn feed_matrix(&mut self, d: &mut Dpy) {
        match self.mode {
            Mode::TraceA => {
                let l = self.tracing.len();
                let count = self.tracing.iter().filter(|c| **c > 0).count();
                if count >= l {
                    self.set_mode(Mode::TraceDone, d.time);
                    self.typing_delay = 1_000_000;
                    return;
                }
                // How fast the numbers get discovered.
                let i = 5 + (30 / (count as i32 + 1));
                if random_below(i) == 0 {
                    let i = random_below(l as i32) as usize;
                    if self.tracing[i] < 0 {
                        self.tracing[i] = -self.tracing[i];
                    }
                }
            }
            Mode::TraceB if random_below(40) == 0 => {
                self.set_mode(Mode::TraceFail, d.time);
                return;
            }
            _ => {}
        }

        if self.use_pipe_p && !self.back_text_full_p {
            let c = self.buf[self.buf_done];
            self.place_back_pipe(c);
            self.buf_done = (self.buf_done + 1) % BUF_SIZE;
            if self.buf_done + 1 == self.buf_pos {
                self.do_fill_buff = true;
            }
        }
        if self.buf_done == self.buf_pos + 1 {
            self.do_fill_buff = false;
        } else {
            self.do_fill_buff = true;
            self.fill_input(d);
        }

        for x in 0..self.grid_width {
            let f = self.feeders[x as usize];
            if f.throttle != 0 {
                /* this is a delay tick, synced to frame */
                self.feeders[x as usize].throttle -= 1;
            } else if f.remaining > 0 {
                /* how many items are in the pipe */
                let rval = if self.use_pipe_p && !self.back_text_full_p {
                    let v = self.buf[f.pipe_loc] as usize;
                    self.feeders[x as usize].pipe_loc += 1;
                    if self.feeders[x as usize].pipe_loc > BUF_SIZE - 1 {
                        self.feeders[x as usize].pipe_loc = 0;
                    }
                    v % self.glyph_map.len()
                } else {
                    random_below(self.glyph_map.len() as i32) as usize
                };
                let g = self.glyph_map[rval] as u16 + 1;
                self.insert_glyph(g, x, f.y);
                self.feeders[x as usize].remaining -= 1;
                if f.y >= 0 {
                    self.feeders[x as usize].y += 1;
                }
            } else {
                /* if pipe is empty, insert spaces */
                self.insert_glyph(0, x, f.y);
                if f.y >= 0 {
                    self.feeders[x as usize].y += 1;
                }
            }

            if random_below(10) == 0 {
                /* randomly change throttle speed */
                self.feeders[x as usize].throttle = random_below(5) + random_below(5);
            }
        }
    }

    /// Percentages of screen coverage to the parameter that actually controls
    /// it. Upstream: "Horrid kludge. I got this mapping empirically, on a
    /// 1024x768 screen. Sue me."
    fn densitizer(&self) -> i32 {
        match self.density {
            d if d < 10 => 85,
            d if d < 15 => 60,
            d if d < 20 => 45,
            d if d < 25 => 25,
            d if d < 30 => 20,
            d if d < 35 => 15,
            d if d < 45 => 10,
            d if d < 50 => 8,
            d if d < 55 => 7,
            d if d < 65 => 5,
            d if d < 80 => 3,
            d if d < 90 => 2,
            _ => 1,
        }
    }

    fn hack_matrix(&mut self, d: &mut Dpy) {
        match self.mode {
            Mode::TraceDone | Mode::TraceFail => return,
            _ => {}
        }

        /* Glow some characters. */
        if !self.insert_bottom_p {
            let mut i = random_below(self.grid_width / 2);
            while i > 1 {
                i -= 1;
                let yy = random_below(self.grid_height);
                let xx = random_below(self.grid_width);
                let idx = self.cell_index(xx, yy);
                if self.cells[idx].glyph != 0 && self.cells[idx].glow == 0 {
                    self.cells[idx].glow = random_below(10);
                    self.cells[idx].changed = true;
                }
            }
        }

        /* Change some of the feeders. */
        for x in 0..self.grid_width as usize {
            if self.feeders[x].remaining > 0 {
                /* never change if pipe isn't empty */
                continue;
            }
            if random_below(self.densitizer()) != 0 {
                /* then change N% of the time */
                continue;
            }

            self.feeders[x].remaining = 3 + random_below(self.grid_height);
            self.feeders[x].throttle = random_below(5) + random_below(5);
            if random_below(4) != 0 {
                self.feeders[x].remaining = 0;
            }

            let bottom_feeder_p = if self.mode == Mode::TraceA || self.mode == Mode::TraceB {
                true
            } else if self.insert_top_p && self.insert_bottom_p {
                random() & 1 != 0
            } else {
                self.insert_bottom_p
            };

            self.feeders[x].y = if bottom_feeder_p {
                random_below(self.grid_height / 2)
            } else {
                -1
            };
        }

        if self.mode == Mode::Matrix && random_below(500) == 0 {
            self.init_spinners();
        }
        let _ = d;
    }

    // ---- Drawing ---------------------------------------------------------

    fn redraw_cells(&mut self, d: &mut Dpy, active: bool) {
        // Upstream declares this outside both loops and never resets it, so
        // once one cell has come from the background every later cell in the
        // scan is treated as though it had. That is what makes the reveal
        // sweep down the screen rather than appear all at once.
        let mut use_back_p = false;

        for y in 0..self.grid_height {
            for x in 0..self.grid_width {
                let idx = self.cell_index(x, y);
                let cursor_p = self.cursor_on && x == self.cursor_x && y == self.cursor_y;

                let mut from_back = false;
                if self.cells[idx].glyph == 0
                    && self.start_reveal_back_p
                    && self.background[idx].glyph != 0
                    && !self.mode.is_trace()
                {
                    use_back_p = true;
                    from_back = true;
                }

                // In trace mode the state of each cell is random unless we
                // have a match for this digit.
                if active && self.mode.is_trace() && !self.tracing.is_empty() {
                    let xx = (x as usize % self.tracing.len()) as i32;
                    let dead_p = self.tracing[xx as usize] > 0;
                    if y == 0 && x == xx && !use_back_p {
                        self.cells[idx].glyph = if dead_p {
                            let digit = (self.tracing[xx as usize] as u8 - b'0') as usize;
                            self.glyph_map[digit.min(self.glyph_map.len() - 1)] as u16 + 1
                        } else {
                            0
                        };
                    } else if y == 0 && !use_back_p {
                        self.cells[idx].glyph = 0;
                    } else if !use_back_p {
                        self.cells[idx].glyph = if dead_p {
                            0
                        } else {
                            let g = random_below(self.glyph_map.len() as i32) as usize;
                            self.glyph_map[g] as u16 + 1
                        };
                    }
                    if !use_back_p {
                        self.cells[idx].changed = true;
                    }
                }

                let cell = if from_back {
                    self.background[idx]
                } else {
                    self.cells[idx]
                };
                if !cell.changed {
                    continue;
                }

                if cell.glyph == 0 && !cursor_p && !use_back_p {
                    let (cw, ch) = (self.char_width, self.char_height);
                    let gc = self.erase_gc.clone();
                    d.win().fill_rectangle(&gc, x * cw, y * ch, cw, ch);
                } else {
                    let g = if cursor_p { CURSOR_GLYPH } else { cell.glyph };
                    let cx = i32::from(g - 1) % CHAR_COLS;
                    let cy = i32::from(g - 1) / CHAR_COLS;
                    let sheet = if cell.glow != 0 || cell.spinner {
                        &self.glow
                    } else {
                        &self.plain
                    };
                    let (cw, ch) = (self.char_width, self.char_height);
                    d.win().copy_area(
                        &self.draw_gc,
                        sheet,
                        cx * cw,
                        cy * ch,
                        cw,
                        ch,
                        x * cw,
                        y * ch,
                    );
                }

                if !use_back_p {
                    self.cells[idx].changed = false;
                }
                if self.cells[idx].glow > 0 && self.mode != Mode::Nmap && !use_back_p {
                    self.cells[idx].glow -= 1;
                    self.cells[idx].changed = true;
                }
                if self.cells[idx].spinner && active && !use_back_p {
                    let g = random_below(self.glyph_map.len() as i32) as usize;
                    self.cells[idx].glyph = self.glyph_map[g] as u16 + 1;
                    self.cells[idx].changed = true;
                }
            }
        }
    }

    // ---- The set pieces --------------------------------------------------

    /// Start a script, or type the next character of the one running.
    fn hack_text(&mut self, d: &mut Dpy) {
        let now = d.time;
        let Some(typing) = self.typing else {
            self.begin_typing(d);
            return;
        };
        let mut typing = typing;

        let mut scrolled_p = false;
        let mut x = self.cursor_x;
        let mut y = self.cursor_y;

        loop {
            let c = typing.first().copied().unwrap_or(0);
            let c1 = if c != 0 {
                typing.get(1).copied().unwrap_or(0)
            } else {
                0
            };

            self.typing_delay = if c == 0 || c1 == b'\n' {
                self.typing_line_delay
            } else {
                self.typing_char_delay
            };
            if c == 0 {
                self.typing_delay = 0;
                self.typing = None;
                return;
            }

            if self.typing_scroll_p && (c == b'\n' || x >= self.grid_width - 1) {
                self.set_cursor(false, now);
                x = 0;
                y += 1;
                if y >= self.grid_height - 1 {
                    let (gw, gh) = (self.grid_width, self.grid_height);
                    for yy in 0..gh - 2 {
                        for xx in 0..gw {
                            let ii = (yy * gw + xx) as usize;
                            let jj = ((yy + 1) * gw + xx) as usize;
                            self.cells[ii] = self.cells[jj];
                            self.cells[ii].changed = true;
                        }
                    }
                    /* clear bottom row */
                    for xx in 0..gw {
                        let ii = ((gh - 2) * gw + xx) as usize;
                        self.cells[ii].glyph = 0;
                        self.cells[ii].changed = true;
                    }
                    y -= 1; /* move back up to bottom line */
                    scrolled_p = true;
                }
            }

            if c == b'\n' {
                if !self.typing_scroll_p {
                    self.set_cursor(false, now);
                    x = self.typing_left_margin;
                    /* clear the line */
                    let i = (self.grid_width * y) as usize;
                    let j = i + self.grid_width as usize;
                    for cell in &mut self.cells[i..j] {
                        cell.glyph = 0;
                        cell.changed = true;
                    }
                }
                self.typing_bold_p = false;
                self.typing_stutter_p = false;
                scrolled_p = true;
            } else if c == 0o10 {
                self.typing_delay += 500_000;
            } else if c == 0o1 {
                self.typing_stutter_p = true;
                self.typing_bold_p = false;
            } else if c == 0o2 {
                self.typing_bold_p = true;
            } else if x < self.grid_width - 1 {
                let idx = self.cell_index(x, y);
                self.cells[idx].glyph = u16::from(CHAR_MAP[c as usize]) + 1;
                if c == b' ' || c == b'\t' {
                    self.cells[idx].glyph = 0;
                }
                self.cells[idx].changed = true;
                self.cells[idx].glow = if self.typing_bold_p { 127 } else { 0 };
            }

            if c >= b' ' {
                x += 1;
            }
            if x >= self.grid_width - 1 {
                x = self.grid_width - 1;
            }

            typing = &typing[1..];
            self.typing = Some(typing);

            if self.typing_stutter_p {
                if self.typing_delay == 0 {
                    self.typing_delay = 20000;
                }
                if random_below(3) != 0 {
                    self.typing_delay += i64::from(random_below(200_000) + 1);
                }
            }

            /* If there's no delay after this character, just keep going. */
            if self.typing_delay != 0 {
                break;
            }
        }

        if scrolled_p || x != self.cursor_x || y != self.cursor_y {
            self.set_cursor(false, now);
            self.cursor_x = x;
            self.cursor_y = y;
            if self.typing_cursor_p {
                self.set_cursor(true, now);
            }
        }
    }

    fn begin_typing(&mut self, d: &mut Dpy) {
        let now = d.time;
        self.set_cursor(false, now);
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.typing_scroll_p = false;
        self.typing_bold_p = false;
        self.typing_cursor_p = true;
        self.typing_stutter_p = false;
        self.typing_char_delay = 10000;
        self.typing_line_delay = 1_500_000;

        match self.mode {
            Mode::TraceTextA | Mode::TraceTextB => {
                self.clear_spinners();
                let wide = self.grid_width >= 52;
                self.typing = Some(match (self.mode, wide) {
                    (Mode::TraceTextA, true) => TRACE_A_WIDE,
                    (Mode::TraceTextA, _) => TRACE_A_NARROW,
                    (_, true) => TRACE_B_WIDE,
                    _ => TRACE_B_NARROW,
                });
            }
            Mode::TraceFail => {
                let s = b"SYSTEM FAILURE\n";
                let len = s.len() as i32;
                let cx = ((self.grid_width - len - 1) as f32 / 2.0 - 0.5).max(0.0);
                let cy = ((self.grid_height / 2) as f32 - 1.3).max(0.0);
                let (cw, ch) = (self.char_width, self.char_height);

                let gc = self.erase_gc.clone();
                d.win().fill_rectangle(
                    &gc,
                    (cx * cw as f32) as i32,
                    (cy * ch as f32) as i32,
                    len * cw,
                    (ch as f32 * 1.6) as i32,
                );

                for i in -2..3 {
                    self.scratch_gc
                        .set_foreground(self.colors[(i + 2) as usize]);
                    d.win().draw_rectangle(
                        &self.scratch_gc,
                        (cx * cw as f32) as i32 - i,
                        (cy * ch as f32) as i32 - i,
                        len * cw + (2 * i),
                        (ch as f32 * 1.6) as i32 + (2 * i),
                    );
                }

                // Otherwise part of the box gets overwritten.
                for cell in self.cells.iter_mut() {
                    cell.changed = false;
                }

                self.cursor_x = ((self.grid_width - len - 1) / 2).max(0);
                self.cursor_y = (self.grid_height / 2 - 1).max(0);
                self.typing = Some(s);
                self.typing_char_delay = 0;
                self.typing_cursor_p = false;
            }
            Mode::Knock => {
                self.clear_spinners();
                self.typing = Some(KNOCK_TEXT);
                self.cursor_x = 4;
                self.cursor_y = 2;
                self.typing_char_delay = 0;
                self.typing_line_delay = 2_000_000;
            }
            _ => {
                self.clear_spinners();
                self.typing = Some(NMAP_TEXT);
                self.cursor_x = 0;
                self.cursor_y = self.grid_height - 3;
                self.typing_scroll_p = true;
                self.typing_char_delay = 0;
                self.typing_line_delay = 20000;
            }
        }

        self.typing_left_margin = self.cursor_x;
        self.typing_delay = self.typing_char_delay;
        if self.typing_cursor_p {
            self.set_cursor(true, now);
        }
    }
}

impl Screenhack for XMatrix {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.blink_cursor(d.time);

        if self.typing_delay > 0 {
            self.typing_delay -= self.delay;
            if self.typing_delay < 0 {
                self.typing_delay = 0;
            }
            self.redraw_cells(d, false);
            return self.delay as u32;
        }

        let now = d.time;
        match self.mode {
            Mode::Matrix
            | Mode::Dna
            | Mode::Binary
            | Mode::Dec
            | Mode::Hex
            | Mode::Ascii
            | Mode::TraceA
            | Mode::TraceB => {
                self.feed_matrix(d);
                self.hack_matrix(d);
            }

            m if m.is_drain() => {
                self.feed_matrix(d);
                if self.screen_blank_p() {
                    self.typing_delay = 500_000;
                    if self.start_reveal_back_p {
                        self.typing_delay = 5_000_000;
                        self.start_reveal_back_p = false;
                        self.back_text_full_p = false;
                        // Move the whole background page into the foreground.
                        for i in 0..self.cells.len() {
                            self.cells[i].glyph = self.background[i].glyph;
                            self.cells[i].changed = self.background[i].changed;
                            self.background[i].glyph = 0;
                            self.background[i].changed = false;
                        }
                    }
                    let next = match m {
                        Mode::DrainTraceA => Mode::TraceTextA,
                        Mode::DrainTraceB => Mode::TraceTextB,
                        Mode::DrainKnock => Mode::Knock,
                        Mode::DrainNmap => Mode::Nmap,
                        _ => self.def_mode,
                    };
                    self.set_mode(next, now);
                }
            }

            Mode::TraceDone => self.set_mode(self.def_mode, now),

            Mode::TraceTextA | Mode::TraceTextB | Mode::TraceFail | Mode::Knock | Mode::Nmap => {
                self.hack_text(d);
                if self.typing.is_none() {
                    /* done typing */
                    self.set_cursor(false, now);
                    let next = match self.mode {
                        Mode::TraceTextA => Mode::TraceA,
                        Mode::TraceTextB => Mode::TraceB,
                        _ => self.def_mode,
                    };
                    self.set_mode(next, now);
                }
            }

            _ => {}
        }

        if self.start_reveal_back_p {
            self.set_mode(Mode::DrainMatrix, now);
        }
        if self.mode == Mode::Matrix && self.knock_knock_p && random_below(10000) == 0 {
            let next = if random_below(5) == 0 {
                Mode::DrainNmap
            } else {
                Mode::DrainKnock
            };
            self.set_mode(next, now);
        }

        self.redraw_cells(d, true);
        self.delay as u32
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        let (ow, oh) = (self.grid_width, self.grid_height);
        self.grid_width = (width / self.char_width + 1).max(5);
        self.grid_height = (height / self.char_height + 1).max(5);

        if ow != self.grid_width || oh != self.grid_height {
            let n = (self.grid_width * self.grid_height) as usize;
            let mut ncells = vec![Cell::default(); n];
            let mut nbackground = vec![Cell::default(); n];
            let mut nfeeders = vec![Feeder::default(); self.grid_width as usize];
            for y in 0..oh.min(self.grid_height) {
                for x in 0..ow.min(self.grid_width) {
                    ncells[(y * self.grid_width + x) as usize] = self.cells[(y * ow + x) as usize];
                    nbackground[(y * self.grid_width + x) as usize] =
                        self.background[(y * ow + x) as usize];
                }
            }
            let keep = ow.min(self.grid_width) as usize;
            nfeeders[..keep].copy_from_slice(&self.feeders[..keep]);
            self.cells = ncells;
            self.background = nbackground;
            self.feeders = nfeeders;
            d.clear_window();
        }

        d.text_reshape(self.grid_width - 2, self.grid_height - 1);
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        let now = d.time;
        if let XEvent::KeyPress { key } = *event {
            match key {
                '0' => {
                    self.back_y = 1;
                    self.back_text_full_p = true;
                    self.start_reveal_back_p = true;
                    return true;
                }
                '+' | '=' | '>' | '.' => {
                    self.density = (self.density + 10).min(100);
                    return true;
                }
                '-' | '_' | '<' | ',' => {
                    self.density = (self.density - 10).max(0);
                    return true;
                }
                '[' | '(' | '{' => {
                    self.insert_top_p = true;
                    self.insert_bottom_p = false;
                    return true;
                }
                ']' | ')' | '}' => {
                    self.insert_top_p = false;
                    self.insert_bottom_p = true;
                    return true;
                }
                '\\' | '|' => {
                    self.insert_top_p = true;
                    self.insert_bottom_p = true;
                    return true;
                }
                't' => {
                    self.set_mode(Mode::DrainTraceA, now);
                    return true;
                }
                'T' => {
                    self.set_mode(Mode::DrainTraceB, now);
                    return true;
                }
                'k' => {
                    self.set_mode(Mode::DrainKnock, now);
                    return true;
                }
                'c' => {
                    self.set_mode(Mode::DrainNmap, now);
                    return true;
                }
                _ => {}
            }
        }

        if screenhack_event_helper(event) {
            self.set_mode(Mode::DrainMatrix, now);
            return true;
        }
        false
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let small_p = d.res.string("matrixFont").eq_ignore_ascii_case("small");

    let (plain_bytes, glow_bytes) = if small_p {
        (
            crate::images::MATRIX_PLAIN_SMALL,
            crate::images::MATRIX_GLOW_SMALL,
        )
    } else {
        (crate::images::MATRIX_PLAIN, crate::images::MATRIX_GLOW)
    };
    // The sheets are compiled in; an empty one would leave a black screen
    // rather than stopping the saver.
    let plain = png::decode(plain_bytes).map_or_else(|| Pixmap::new(16, 13), |(i, _)| i);
    let glow = png::decode(glow_bytes).map_or_else(|| Pixmap::new(16, 13), |(i, _)| i);
    let char_width = (plain.width() / CHAR_COLS).max(1);
    let char_height = (plain.height() / CHAR_ROWS).max(1);

    let grid_width = (d.width() / char_width + 1).max(5);
    let grid_height = (d.height() / char_height + 1).max(5);

    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");

    let insert = d.res.string("insert").to_string();
    let (insert_top_p, insert_bottom_p) = match insert.as_str() {
        "top" => (true, false),
        "both" => (true, true),
        _ => (false, true),
    };

    let n = (grid_width * grid_height) as usize;
    let mut st = XMatrix {
        draw_gc: Gc::new(fg, bg),
        erase_gc: Gc::new(bg, bg),
        scratch_gc: Gc::new(fg, bg),
        grid_width,
        grid_height,
        char_width,
        char_height,
        cells: vec![Cell::default(); n],
        background: vec![Cell::default(); n],
        feeders: vec![Feeder::default(); grid_width as usize],
        nspinners: d.res.int("spinners"),
        knock_knock_p: d.res.bool("knockKnock"),
        insert_top_p,
        insert_bottom_p,
        use_pipe_p: d.res.bool("usePipe"),
        // Deliberately not any real mode, so the first `set_mode` takes.
        mode: Mode::TraceDone,
        def_mode: Mode::Matrix,
        buf: vec![0; BUF_SIZE * 2 + 1],
        do_fill_buff: true,
        buf_done: 0,
        buf_pos: 1,
        start_reveal_back_p: false,
        back_text_full_p: false,
        back_line: vec![0; BUF_SIZE * 2 + 1],
        back_pos: 0,
        back_y: 0,
        tracing: Vec::new(),
        density: d.res.int("density"),
        typing: None,
        typing_scroll_p: false,
        typing_cursor_p: false,
        typing_bold_p: false,
        typing_stutter_p: false,
        typing_left_margin: 0,
        typing_char_delay: 0,
        typing_line_delay: 0,
        typing_delay: 0,
        cursor_enabled: false,
        cursor_on: false,
        cursor_phase: 0.0,
        cursor_x: -1,
        cursor_y: -1,
        plain,
        glow,
        images_flipped_p: false,
        glyph_map: MATRIX_ENCODING,
        colors: [
            rgb(0x08, 0x1E, 0x08),
            rgb(0x5A, 0xD2, 0x5A),
            rgb(0xE0, 0xF7, 0xE0),
            rgb(0x5A, 0xD2, 0x5A),
            rgb(0x08, 0x1E, 0x08),
        ],
        delay: i64::from(d.res.int("delay").max(0)),
    };
    st.buf[0] = b' '; /* spacer byte in buffer (space) */

    let now = d.time;
    let mode = d.res.string("mode").to_string();
    match mode.to_ascii_lowercase().as_str() {
        "trace" => st.set_mode(
            if random_below(3) != 0 {
                Mode::TraceTextA
            } else {
                Mode::TraceTextB
            },
            now,
        ),
        "crack" => st.set_mode(Mode::DrainNmap, now),
        "dna" => {
            st.def_mode = Mode::Dna;
            st.set_mode(Mode::Dna, now);
        }
        "bin" | "binary" => {
            st.def_mode = Mode::Binary;
            st.set_mode(Mode::Binary, now);
        }
        "hex" | "hexadecimal" => {
            st.def_mode = Mode::Hex;
            st.set_mode(Mode::Hex, now);
        }
        "dec" | "decimal" => {
            st.def_mode = Mode::Dec;
            st.set_mode(Mode::Dec, now);
        }
        "asc" | "ascii" => {
            st.def_mode = Mode::Ascii;
            st.set_mode(Mode::Ascii, now);
        }
        "pipe" => {
            st.def_mode = Mode::Ascii;
            st.use_pipe_p = true;
            st.set_mode(Mode::Ascii, now);
        }
        _ => st.set_mode(Mode::Matrix, now),
    }

    if st.mode == Mode::Matrix && d.res.bool("trace") {
        let m = if random_below(3) != 0 {
            Mode::TraceTextA
        } else {
            Mode::TraceTextB
        };
        st.set_mode(m, now);
    }

    d.text_reshape(st.grid_width - 2, st.grid_height - 1);
    Box::new(st)
}

/// The number the trace program is hunting for. Upstream reads this from a
/// resource; there is no text widget in the panel, so it is the one from the
/// film's screens.
const PHONE: &str = "(415) 626-1409";

const TRACE_A_WIDE: &[u8] = b"Call trans opt: received. 2-19-98 13:24:18 REC:Log>\n\
Trace program: running\n";
const TRACE_A_NARROW: &[u8] = b"Call trans opt: received.\n2-19-98 13:24:18 REC:Log>\n\
Trace program: running\n";
const TRACE_B_WIDE: &[u8] = b"Call trans opt: received. 9-18-99 14:32:21 REC:Log>\n\
WARNING: carrier anomaly\n\
Trace program: running\n";
const TRACE_B_NARROW: &[u8] = b"Call trans opt: received.\n9-18-99 14:32:21 REC:Log>\n\
WARNING: carrier anomaly\n\
Trace program: running\n";

const KNOCK_TEXT: &[u8] = b"\x01Wake up, Neo...\n\
\x01The Matrix has you...\n\
\x01Follow the white rabbit.\n\
\n\
Knock, knock, Neo.\n";

/// Trinity's session. Upstream's note: what she is using here is moderately
/// accurate. She runs nmap, then breaks in with a hypothetical program called
/// "sshnuke" that exploits the very real SSHv1 CRC32 compensation attack
/// detector bug. The command syntax of the power grid control software looks a
/// lot like Cisco IOS.
const NMAP_TEXT: &[u8] = b"# \x08\x08\x08\x08\
\x01nmap -v -sS -O 10.2.2.2\n\
Starting nmap V. 2.54BETA25\n\
\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\
Insufficient responses for TCP sequencing (3), OS detection may be less accurate\n\
Interesting ports on 10.2.2.2:\n\
(The 1539 ports scanned but not shown below are in state: closed)\n\
Port       state       service\n\
22/tcp     open        ssh\n\
\n\
No exact OS matches for host\n\
\n\
Nmap run completed -- 1 IP address (1 host up) scanned\n\
# \x08\x08\x08\x08\
\x01sshnuke 10.2.2.2 -rootpw=\"Z1ON0101\"\n\
Connecting to 10.2.2.2:ssh ... \x08\x08successful.\n\
Attempting to exploit SSHv1 CRC32 ... \x08\x08\x08\x08successful.\n\
Resetting root password to \"Z1ON0101\".\n\
\x08\x08System open: Access Level <9>\n\
# \x08\x08\
\x01ssh 10.2.2.2 -l root\n\
\x08\x08root@10.2.2.2's password: \x08\x08\n\x08\x08\n\
RRF-CONTROL> \x08\x08\
\x01disable grid nodes 21 - 48\n\
\n\
\x02Warning: Disabling nodes 21-48 will disconnect sector 11 (28 nodes)\n\
\n\
\x02         ARE YOU SURE? (y/n) \x08\x08\
\x01y\n\n\n\
\x08\x02Grid Node 21 offline...\n\
\x08\x02Grid Node 22 offline...\n\
\x08\x02Grid Node 23 offline...\n\
\x08\x02Grid Node 24 offline...\n\
\x08\x02Grid Node 25 offline...\n\
\x08\x02Grid Node 26 offline...\n\
\x08\x02Grid Node 27 offline...\n\
\x08\x02Grid Node 28 offline...\n\
\x08\x02Grid Node 29 offline...\n\
\x08\x02Grid Node 30 offline...\n\
\x08\x02Grid Node 31 offline...\n\
\x08\x02Grid Node 32 offline...\n\
\x08\x02Grid Node 33 offline...\n\
\x08\x02Grid Node 34 offline...\n\
\x08\x02Grid Node 35 offline...\n\
\x08\x02Grid Node 36 offline...\n\
\x08\x02Grid Node 37 offline...\n\
\x08\x02Grid Node 38 offline...\n\
\x08\x02Grid Node 39 offline...\n\
\x08\x02Grid Node 40 offline...\n\
\x08\x02Grid Node 41 offline...\n\
\x08\x02Grid Node 42 offline...\n\
\x08\x02Grid Node 43 offline...\n\
\x08\x02Grid Node 44 offline...\n\
\x08\x02Grid Node 45 offline...\n\
\x08\x02Grid Node 46 offline...\n\
\x08\x02Grid Node 47 offline...\n\
\x08\x02Grid Node 48 offline...\n\
\x08\x08\
\nRRF-CONTROL> \x08\x08\x08\x08\x08\x08\x08\x08";

const DEFAULTS: &[&str] = &[
    ".background:		   black",
    ".foreground:		   #00AA00",
    "*fpsSolid:		   true",
    "*matrixFont:		   large",
    "*delay:		   10000",
    "*insert:		   both",
    "*mode:		   Matrix",
    "*tracePhone:            (415) 626-1409",
    "*spinners:		   5",
    "*density:		   75",
    "*trace:		   True",
    "*knockKnock:		   True",
    "*usePipe:		   False",
    "*usePty:                False",
    "*program:		   xscreensaver-text --latin1",
];

const FONTS: &[SelectItem] = &[
    SelectItem {
        value: "large",
        label: "Large font",
    },
    SelectItem {
        value: "small",
        label: "Small font",
    },
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "matrix",
        label: "Matrix encoding",
    },
    SelectItem {
        value: "binary",
        label: "Binary encoding",
    },
    SelectItem {
        value: "hex",
        label: "Hexadecimal encoding",
    },
    SelectItem {
        value: "dna",
        label: "Genetic encoding",
    },
    SelectItem {
        value: "pipe",
        label: "Piped ASCII text",
    },
];

const FILLS: &[SelectItem] = &[
    SelectItem {
        value: "both",
        label: "Synergistic algorithm",
    },
    SelectItem {
        value: "top",
        label: "Slider algorithm",
    },
    SelectItem {
        value: "bottom",
        label: "Expansion algorithm",
    },
];

const OPTS: &[Opt] = &[
    Opt::select("matrixFont", "Font size", FONTS, "large"),
    Opt::select("mode", "Encoding", MODES, "matrix"),
    Opt::select("insert", "Fill", FILLS, "both"),
    Opt::boolean("trace", "Run trace program", "True"),
    Opt::boolean("knockKnock", "Knock knock", "True"),
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("density", "Density", 1.0, 100.0, 1.0, 0, "75"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "xmatrix",
    label: "XMatrix",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=dSJQHm-YoWc"),
        blurb: "The digital rain, as it was on the screens in The Matrix.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
