//! Port of `hacks/fontglide.c`.
//!
//! ```text
//! xscreensaver, Copyright © 2003-2026 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! fontglide -- reads text from a subprocess and puts it on the screen using
//! large characters that glide in from the edges, assemble, then disperse.
//! Requires a system with scalable fonts.  (X's font handing sucks.  A lot.)
//! ```
//!
//! Words fly in from off screen, settle into a laid-out page, sit there a
//! moment, and fly off again, or else a whole sentence scrolls past from right
//! to left. Several sentences are in the air at once, on top of each other, and
//! each picks its own size, its own colours and its own alignment.
//!
//! Three details are what make it read as typesetting rather than as sliding
//! boxes. Each word decelerates into place on a sine curve rather than
//! travelling at a constant rate. A sentence ends where its text does: after a
//! few words it will stop at a full stop, and after a dozen it will settle for
//! a comma. And when the layout runs past the right margin the line is aligned
//! *retrospectively*, by shifting every word placed since the last break, which
//! is how the same code centres, ranges left and ranges right.
//!
//! Two things this needs that the runtime did not have before: text to read,
//! which now comes from [`crate::runtime::text`], and a font, which comes from
//! [`crate::runtime::font`]. The second is the visible compromise: upstream
//! trawls the system for scalable faces and picks a different one for every
//! sentence, and there is only one here, at whatever whole magnification comes
//! nearest the size it asked for.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::rgb;
use crate::runtime::font::Font;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, SelectItem, StartArgs, frand,
    random, random_below,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Anim {
    In,
    Pause,
    Out,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Page,
    Scroll,
    Chars,
}

struct Word {
    text: String,
    /// Position of the origin of the first character.
    x: i32,
    y: i32,
    /* These have the same meanings as in XCharStruct: */
    /// Origin to leftmost pixel.
    lbearing: i32,
    /// Origin to rightmost pixel.
    rbearing: i32,
    /// Origin to topmost pixel.
    ascent: i32,
    /// Origin to bottommost pixel.
    descent: i32,
    /// Origin to the next word's origin.
    width: i32,

    nticks: i32,
    tick: i32,
    start_x: i32,
    start_y: i32,
    target_x: i32,
    target_y: i32,
    /// The word rendered once, to be flown about as a picture.
    ///
    /// Upstream pairs this with a one-bit mask of everything that is not the
    /// black it was cleared to, and clips through it. There are no bitmap clip
    /// masks here, but the mask is *derived* from the pixmap, so skipping the
    /// black pixels at blit time is the same operation.
    pixmap: Option<Pixmap>,
}

struct Sentence {
    id: i32,
    dark_p: bool,
    move_chars_p: bool,
    width: i32,
    font: Option<Font>,
    fg: Pixel,
    bg: Pixel,
    words: Vec<Word>,
    anim_state: Anim,
    alignment: Align,
    pause_tick: i32,
}

struct FontGlide {
    gc: Gc,
    bg_pixel: Pixel,
    width: i32,
    height: i32,

    /// Size of the font outline.
    border_width: i32,
    /// Frame rate multiplier.
    speed: f64,
    /// Multiplier for how long to leave words on screen.
    linger: f64,
    trails_p: bool,
    mode: Mode,

    /// This only needs to be as big as one "word".
    buf: Vec<u8>,
    /// A word taken back off the layout, to be read again next time.
    unread_word_text: Option<String>,
    early_p: bool,

    sentences: Vec<Option<Sentence>>,
    /// Whether it is time to create a new sentence.
    spawn_p: bool,
    latest_sentence: i32,
    frame_delay: u32,
    id_tick: i32,
}

/// How big `buf` gets, which upstream sizes to hold one word.
const BUF_MAX: usize = 40;

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.width(), d.height());

    let mut border_width = d.res.int("fontBorderWidth");
    if !(0..=20).contains(&border_width) {
        border_width = 1;
    }
    let mut speed = d.res.float("speed");
    if speed <= 0.0 || speed > 200.0 {
        speed = 1.0;
    }
    let mut linger = d.res.float("linger");
    if linger <= 0.0 || linger > 200.0 {
        linger = 1.0;
    }

    let mode = match d.res.string("mode").to_ascii_lowercase().as_str() {
        "scroll" => Mode::Scroll,
        "page" => Mode::Page,
        "chars" | "char" => Mode::Chars,
        // "random", and anything unrecognised, which upstream warns about.
        _ => {
            if random() & 1 != 0 {
                Mode::Scroll
            } else {
                Mode::Page
            }
        }
    };

    let bg_pixel = d.res.pixel("background");
    Box::new(FontGlide {
        gc: Gc::new(d.res.pixel("foreground"), bg_pixel),
        bg_pixel,
        width,
        height,
        border_width,
        speed,
        linger,
        trails_p: d.res.bool("trails"),
        mode,
        buf: Vec::new(),
        unread_word_text: None,
        early_p: true,
        sentences: (0..5).map(|_| None).collect(), /* #### */
        spawn_p: true,
        latest_sentence: 0,
        frame_delay: d.res.int("delay").max(0) as u32,
        id_tick: 0,
    })
}

impl FontGlide {
    fn pick_font_size(&self) -> i32 {
        let scale = f64::from(self.height) / 1024.0; /* shrink for small windows */
        let mut min = (scale * 24.0) as i32;
        let mut max = (scale * 260.0) as i32;
        if min < 10 {
            min = 10;
        }
        if max < 30 {
            max = 30;
        }
        let r = (max - min) / 3 + 1;
        let mut pixel = min + random_below(r) + random_below(r) + random_below(r);
        if self.mode == Mode::Scroll {
            /* scroll mode likes bigger fonts */
            pixel = (f64::from(pixel) * 1.5) as i32;
        }
        pixel
    }

    /// When the subprocess has generated some output, this reads as much as it
    /// can into `buf`.
    fn drain_input(&mut self, d: &mut Dpy) {
        while self.buf.len() < BUF_MAX - 2 {
            match d.text_getc() {
                Some(c) => self.buf.push(c),
                None => break,
            }
        }
    }

    /// One word, or `None` if there is no complete word available. A blank line
    /// also gives `None`, which is what breaks one sentence from the next.
    fn get_word_text(&mut self, d: &mut Dpy) -> Option<String> {
        self.drain_input(d);

        // If we just launched and have had no text yet, and it has been a
        // while, push out "Loading..." so there is something to look at.
        if self.early_p && self.buf.is_empty() && self.unread_word_text.is_none() && d.time > 2.0 {
            self.unread_word_text = Some("Loading...".to_string());
            self.early_p = false;
        }

        if let Some(t) = self.unread_word_text.take() {
            return Some(t);
        }

        // Skip whitespace, counting the linebreaks in it.
        let mut start = 0;
        let mut lfs = 0;
        while start < self.buf.len() && (self.buf[start] as char).is_whitespace() {
            let c = self.buf[start];
            if c == b'\n' || (c == b'\r' && self.buf.get(start + 1) != Some(&b'\n')) {
                lfs += 1;
            }
            start += 1;
        }

        let mut end = start;
        let mut result = None;
        if lfs < 2 {
            // Skip forward to the end of this word.
            while end < self.buf.len() && !(self.buf[end] as char).is_whitespace() {
                end += 1;
            }
            if end > start {
                result = Some(String::from_utf8_lossy(&self.buf[start..end]).into_owned());
            }
        }

        // Make room in the buffer by dropping what has been processed.
        if end > 0 {
            self.buf.drain(..end);
        }
        result
    }

    /// Gets some random text, and creates a "word" object from it.
    fn new_word(&self, se: &Sentence, txt: &str, alloc_p: bool) -> Word {
        let bw = self.border_width;
        let font = se.font.unwrap_or_else(|| Font::at_size(22));

        // With one fixed-width face there are no side bearings to speak of:
        // the ink is the advance, which is why these come out symmetrical.
        let mut w = Word {
            text: txt.to_string(),
            x: 0,
            y: 0,
            lbearing: -bw,
            rbearing: font.text_width(txt) + bw,
            ascent: font.ascent() + bw,
            descent: font.descent() + bw,
            width: font.text_width(txt),
            nticks: 0,
            tick: 0,
            start_x: 0,
            start_y: 0,
            target_x: 0,
            target_y: 0,
            pixmap: None,
        };

        if alloc_p {
            let width = (w.rbearing - w.lbearing).max(1);
            let height = (w.ascent + w.descent).max(1);
            let mut pm = Pixmap::new(width, height);
            // Cleared to black, which is also what says which pixels are not
            // part of the word when it comes to be drawn.
            pm.clear(rgb(0, 0, 0));

            let mut gc = Gc::new(se.fg, se.bg);

            /* Draw background text for border */
            gc.set_foreground(se.bg);
            for i in -bw..=bw {
                for j in -bw..=bw {
                    pm.draw_string(&gc, &font, -w.lbearing + i, w.ascent + j, txt);
                }
            }

            /* Draw foreground text */
            gc.set_foreground(se.fg);
            pm.draw_string(&gc, &font, -w.lbearing, w.ascent, txt);

            w.pixmap = Some(pm);
        }
        w
    }

    /// Divide each of the words in the sentence into one character words,
    /// without changing the positions of those characters.
    fn split_words(&self, se: &mut Sentence) {
        let mut words2: Vec<Word> = Vec::new();
        for parent in std::mem::take(&mut se.words) {
            let (mut x, mut sx, mut tx) = (parent.x, parent.start_x, parent.target_x);
            let (y, sy, ty) = (parent.y, parent.start_y, parent.target_y);
            for ch in parent.text.chars() {
                let mut w2 = self.new_word(se, &ch.to_string(), true);
                w2.x = x;
                w2.y = y;
                w2.start_x = sx;
                w2.start_y = sy;
                w2.target_x = tx;
                w2.target_y = ty;
                x += w2.width;
                sx += w2.width;
                tx += w2.width;
                words2.push(w2);
            }
        }
        se.words = words2;
    }

    /// Set the source or destination position of the words to be somewhere off
    /// screen. A quarter of the time the whole sentence goes the same way,
    /// which reads as a flock rather than a scattering.
    fn scatter_sentence(&self, se: &mut Sentence) {
        let off = self.border_width * 4 + 2;
        let flock_p = random_below(4) == 0;
        let mode = if flock_p { random_below(12) } else { 0 };

        for w in se.words.iter_mut() {
            let r = if flock_p { mode } else { random_below(4) };
            let left = -(off + w.rbearing);
            let top = -(off + w.descent);
            let right = off - w.lbearing + self.width;
            let bottom = off + w.ascent + self.height;

            let (x, y) = match r {
                /* random positions on the edges */
                0 => (left, random_below(self.height)),
                1 => (right, random_below(self.height)),
                2 => (random_below(self.width), top),
                3 => (random_below(self.width), bottom),
                /* straight towards the edges */
                4 => (left, w.target_y),
                5 => (right, w.target_y),
                6 => (w.target_x, top),
                7 => (w.target_x, bottom),
                /* corners */
                8 => (left, top),
                9 => (left, bottom),
                10 => (right, top),
                _ => (right, bottom),
            };

            if se.anim_state == Anim::In {
                w.start_x = x;
                w.start_y = y;
            } else {
                w.start_x = w.x;
                w.start_y = w.y;
                w.target_x = x;
                w.target_y = y;
            }

            w.nticks = ((100 + random_below(140) + random_below(140) + random_below(140)) as f64
                / self.speed) as i32;
            if w.nticks < 2 {
                w.nticks = 2;
            }
            w.tick = 0;
        }
    }

    /// Set the source position of the words to be off the right side, and the
    /// destination to be off the left side.
    fn aim_sentence(&self, se: &mut Sentence) {
        if se.words.is_empty() {
            return;
        }
        // Shift the sentence up or down a little, but never so far that it
        // falls off before its last character has reached the left edge.
        let mut yoff = 0;
        for _ in 0..10 {
            let ty = random_below(self.height - se.words[0].ascent);
            yoff = ty - se.words[0].target_y;
            if yoff < self.height / 3 {
                break; /* this one is ok */
            }
        }

        let se_width = se.width;
        for w in se.words.iter_mut() {
            w.start_x = w.target_x + self.width;
            w.target_x -= se_width;
            w.start_y = w.target_y;
            w.target_y += yoff;
        }

        let mut nticks =
            ((se.words[0].start_x - se.words[0].target_x) as f64 / (self.speed * 7.0)) as i32;
        nticks = (f64::from(nticks) * (frand(0.9) + frand(0.9) + frand(0.9))) as i32;
        if nticks < 2 {
            nticks = 2;
        }
        for w in se.words.iter_mut() {
            w.nticks = nticks;
            w.tick = 0;
        }
    }

    /// Randomize the order of the words, since that changes which are on top.
    fn shuffle_words(se: &mut Sentence) {
        let n = se.words.len();
        for i in 0..n.saturating_sub(1) {
            let j = i + random_below((n - i) as i32) as usize;
            se.words.swap(i, j);
        }
    }

    /// Re-pick the colours of the text and its border. One of the two is always
    /// bright and the other dark; which way round is a coin flip.
    fn recolor(se: &mut Sentence) {
        let mut fg = (
            random_below(0x5555) + 0xAAAA,
            random_below(0x5555) + 0xAAAA,
            random_below(0x5555) + 0xAAAA,
        );
        let mut bg = (
            random_below(0x5555),
            random_below(0x5555),
            random_below(0x5555),
        );
        se.dark_p = false;
        if random() & 1 != 0 {
            std::mem::swap(&mut fg, &mut bg);
            se.dark_p = true;
        }
        let pack = |c: (i32, i32, i32)| rgb((c.0 >> 8) as u8, (c.1 >> 8) as u8, (c.2 >> 8) as u8);
        se.fg = pack(fg);
        se.bg = pack(bg);
    }

    /// Shift everything placed since the last line break, which is how the one
    /// layout pass ends up left, centre or right aligned.
    fn align_line(se: &mut Sentence, line_start: usize, x: i32, right: i32) {
        let off = match se.alignment {
            Align::Left => 0,
            Align::Center => (right - x) / 2,
            Align::Right => right - x,
        };
        if off != 0 {
            for w in se.words[line_start..].iter_mut() {
                w.target_x += off;
            }
        }
    }

    /// Fill the sentence with new words: in "page" mode, fill the page with
    /// text; in "scroll" mode, make one long horizontal sentence. The sentence
    /// might have *no* words in it, if no text is currently available.
    fn populate_sentence(&mut self, d: &mut Dpy, se: &mut Sentence) {
        se.move_chars_p = match self.mode {
            Mode::Chars => true,
            Mode::Scroll => false,
            Mode::Page => random_below(3) == 0,
        };
        se.alignment = match random_below(3) {
            0 => Align::Left,
            1 => Align::Center,
            _ => Align::Right,
        };

        Self::recolor(se);
        se.words.clear();

        let (left, right, top) = match self.mode {
            Mode::Page | Mode::Chars => (
                random_below(self.width / 3),
                self.width - random_below(self.width / 3),
                random_below(self.height * 2 / 3),
            ),
            Mode::Scroll => (0, self.width, random_below(self.height)),
        };

        let mut x = left;
        let mut y = top;
        let mut space = 0;
        let mut line_start = 0;
        let mut done = false;

        while !done {
            let Some(txt) = self.get_word_text(d) else {
                // If the stream is empty, bail. If it ran dry after some
                // words, that is the end of this sentence.
                if se.words.is_empty() {
                    return;
                }
                break;
            };

            if se.font.is_none() {
                /* Got a word: need a font now */
                let font = Font::at_size(self.pick_font_size());
                se.font = Some(font);
                if y < font.ascent() {
                    y += font.ascent();
                }
                // Measure the space character to figure out how much room to
                // leave between words, since we don't actually render it.
                space = font.char_width();
            }
            let font = se.font.unwrap_or_else(|| Font::at_size(22));

            let mut w = self.new_word(se, &txt, !se.move_chars_p);

            // If we have a few words, let punctuation terminate the sentence.
            if se.words.len() >= 4 && txt.ends_with(['.', '?', '!']) {
                done = true;
            }
            // If the sentence is kind of long already, settle for a comma.
            if se.words.len() >= 12 && txt.ends_with([',', ';', ':', '-', ')', ']', '}']) {
                done = true;
            }
            if se.words.len() >= 25 {
                /* ok that's just about enough out of you */
                done = true;
            }

            if (self.mode == Mode::Page || self.mode == Mode::Chars) && x + w.rbearing > right {
                /* wrap line */
                Self::align_line(se, line_start, x, right);
                line_start = se.words.len();
                x = left;
                y += font.ascent() + font.descent();

                // If we're close to the bottom of the screen, stop and unread
                // the current word. But not if it is the first, or we might
                // get stuck on it.
                if !se.words.is_empty() && y + font.ascent() + font.descent() > self.height {
                    self.unread_word_text = Some(w.text.clone());
                    break;
                }
            }

            w.target_x = x;
            w.target_y = y;
            x += w.width + space;
            se.width = x;
            se.words.push(w);
        }

        se.width -= space;

        match self.mode {
            Mode::Page | Mode::Chars => {
                Self::align_line(se, line_start, x, right);
                if se.move_chars_p {
                    self.split_words(se);
                }
                self.scatter_sentence(se);
                Self::shuffle_words(se);
            }
            Mode::Scroll => self.aim_sentence(se),
        }
    }

    /// If there is room for more sentences, add one.
    fn more_sentences(&mut self, d: &mut Dpy) {
        let Some(i) = self.sentences.iter().position(Option::is_none) else {
            return;
        };
        self.id_tick += 1;
        let mut se = Sentence {
            id: self.id_tick,
            dark_p: false,
            move_chars_p: false,
            width: 0,
            font: None,
            fg: rgb(0xFF, 0xFF, 0xFF),
            bg: rgb(0, 0, 0),
            words: Vec::new(),
            anim_state: Anim::In,
            alignment: Align::Left,
            pause_tick: 0,
        };
        self.populate_sentence(d, &mut se);
        if se.words.is_empty() {
            return;
        }
        self.spawn_p = false;
        self.latest_sentence = se.id;
        self.sentences[i] = Some(se);
        // Sort by id, so that sentences added later are on top.
        self.sentences
            .sort_by_key(|s| s.as_ref().map_or(999_999, |s| s.id));
    }

    /// Render all the words of one sentence, and run its animation one step.
    fn draw_sentence(&mut self, d: &mut Dpy, i: usize) {
        let Some(se) = self.sentences[i].take() else {
            return;
        };
        let mut se = se;
        let mut moved = false;

        for wi in 0..se.words.len() {
            match self.mode {
                Mode::Page | Mode::Chars => {
                    let w = &mut se.words[wi];
                    if se.anim_state != Anim::Pause && w.tick <= w.nticks {
                        let dx = w.target_x - w.start_x;
                        let dy = w.target_y - w.start_y;
                        // Decelerating into place, which is what makes it look
                        // like typesetting rather than sliding.
                        let r = (f64::from(w.tick) * std::f64::consts::PI
                            / f64::from(2 * w.nticks))
                        .sin();
                        w.x = w.start_x + (f64::from(dx) * r) as i32;
                        w.y = w.start_y + (f64::from(dy) * r) as i32;
                        w.tick += 1;
                        if se.anim_state == Anim::Out {
                            w.tick += 1; /* go out faster */
                        }
                        moved = true;
                    }
                }
                Mode::Scroll => {
                    let (x, width) = {
                        let w = &mut se.words[wi];
                        let dx = w.target_x - w.start_x;
                        let dy = w.target_y - w.start_y;
                        let r = f64::from(w.tick) / f64::from(w.nticks);
                        w.x = w.start_x + (f64::from(dx) * r) as i32;
                        w.y = w.start_y + (f64::from(dy) * r) as i32;
                        w.tick += 1;
                        moved = w.tick <= w.nticks;
                        (w.x, se.width)
                    };

                    // Launch a new sentence when the front of this one is
                    // almost off the left edge and its end is almost on
                    // screen, or now and then at random.
                    if se.anim_state != Anim::Out && wi == 0 && se.id == self.latest_sentence {
                        let new_p = x < (self.width as f64 * 0.4) as i32
                            && x + width < (self.width as f64 * 2.1) as i32;
                        let rand_p = !new_p && random_below(2000) == 0;
                        if new_p || rand_p {
                            se.anim_state = Anim::Out;
                            self.spawn_p = true;
                        }
                    }
                }
            }

            let w = &se.words[wi];
            let (x, y) = (w.x, w.y);
            let _ = (x, y);
            self.draw_word_at(d, &se, wi);
        }

        if !moved {
            match se.anim_state {
                Anim::In => {
                    se.anim_state = Anim::Pause;
                    se.pause_tick = (se.words.len() as f64 * 7.0 * self.linger) as i32;
                    if se.move_chars_p {
                        se.pause_tick /= 5;
                    }
                    self.scatter_sentence(&mut se);
                    Self::shuffle_words(&mut se);
                }
                Anim::Pause => {
                    se.pause_tick -= 1;
                    if se.pause_tick <= 0 {
                        se.anim_state = Anim::Out;
                        self.spawn_p = true;
                    }
                }
                Anim::Out => return, /* dead: leave the slot empty */
            }
        }
        self.sentences[i] = Some(se);
    }

    /// `draw_word`, reached with the sentence moved out of `self`.
    fn draw_word_at(&mut self, d: &mut Dpy, se: &Sentence, wi: usize) {
        let w = &se.words[wi];
        let Some(pm) = &w.pixmap else {
            return;
        };
        let x = w.x + w.lbearing;
        let y = w.y - w.ascent;
        let width = w.rbearing - w.lbearing;
        let height = w.ascent + w.descent;
        if x + width < 0 || y + height < 0 || x > self.width || y > self.height {
            return;
        }
        let black = rgb(0, 0, 0);
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + width).min(self.width), (y + height).min(self.height));
        for py in y0..y1 {
            for px in x0..x1 {
                let c = pm.get_pixel(px - x, py - y);
                if c != black {
                    d.win().put_pixel(px, py, c);
                }
            }
        }
    }
}

impl Screenhack for FontGlide {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.spawn_p {
            self.more_sentences(d);
        }

        if !self.trails_p {
            let bg = self.bg_pixel;
            self.gc.set_foreground(bg);
            let (w, h) = (self.width, self.height);
            d.win().fill_rectangle(&self.gc, 0, 0, w, h);
        }

        for i in 0..self.sentences.len() {
            self.draw_sentence(d, i);
        }

        self.frame_delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        d.clear_window();
    }
}

const DEFAULTS: &[&str] = &[
    ".background:		#000000",
    ".foreground:		#DDDDDD",
    ".borderColor:	#555555",
    "*delay:	        10000",
    "*program:	        xscreensaver-text",
    "*usePty:             false",
    "*mode:               random",
    ".font:               (default)",
    "*fontBorderWidth:    2",
    "*speed:              1.0",
    "*linger:             1.0",
    "*trails:             False",
    "*doubleBuffer:	True",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random display style",
    },
    SelectItem {
        value: "page",
        label: "Pages of text",
    },
    SelectItem {
        value: "scroll",
        label: "Horizontally scrolling text",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("linger", "Page linger", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::select("mode", "Display style", MODES, "random"),
    Opt::spin("fontBorderWidth", "Font border thickness", 0.0, 8.0, "2"),
    Opt::boolean("trails", "Vapor trails", "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "fontglide",
    label: "Font Glide",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=2KCXD19FHk0"),
        blurb: "Text glides in from the edges, assembles, then disperses.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
