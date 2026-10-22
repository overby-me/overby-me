//! Port of `hacks/xjack.c`.
//!
//! ```text
//! xscreensaver, Copyright © 1997-2021 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Wendy, let me explain something to you.  Whenever you come in here and
//! interrupt me, you're BREAKING my CONCENTRATION.  You're DISTRACTING me!
//! And it will then take me time to get back to where I was. You understand?
//! Now, we're going to make a new rule.  When you come in here and you hear
//! me typing, or whether you DON'T hear me typing, or whatever the FUCK you
//! hear me doing; when I'm in here, it means that I am working, THAT means
//! don't come in!  Now, do you think you can handle that?
//! ```
//!
//! A novel by Jack Torrance, typed one character at a time onto a page that
//! scrolls when it fills. It is not the sentence that makes it, it is the
//! typing: the keys mis-strike and land a fraction off, a finger catches the
//! next key along, a letter comes out capitalised, the carriage backs up to
//! correct a mistake or fails to advance at all, and the pauses between
//! characters are as uneven as a real hand. The margins wander down the page,
//! and every so often the whole thing is interrupted by an NFS server going
//! away and coming back.
//!
//! This is the first port to draw text. See [`crate::runtime::font`] for what
//! that means here: one size of one bitmap font, so the twenty-four point face
//! this asks for comes out as twenty-two pixels and the page has the number of
//! columns that implies.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::font::Font;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, random, random_below,
};

/// If you're here because you're thinking about making this string be
/// customizable, then you don't get the joke. You loser.
const SOURCE: &str = "All work and no play makes Jack a dull boy.  ";

struct XJack {
    font: Font,
    gc: Gc,
    width: i32,
    height: i32,

    /// How far through [`SOURCE`] the typist is.
    s: usize,
    /* characters */
    columns: i32,
    rows: i32,
    left: i32,
    right: i32,
    /* pixels */
    char_width: i32,
    line_height: i32,
    /* characters */
    x: i32,
    y: i32,
    /// Which way the margins are wandering: 0 is straight, 1 and 2 walk the
    /// left margin, 3 and 4 the right.
    mode: i32,
    hspace: i32,
    vspace: i32,
    break_para: bool,
    caps: bool,
    sentences: i32,
    paras: i32,
    scrolling: i32,
    subscrolling: i32,
    pining: i32,

    delay: u32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.width(), d.height());
    // Upstream picks a different size for a small window and for a retina one.
    let fontname = if width > 1200 || height > 1200 {
        d.res.string("font3").to_string()
    } else if width > 480 {
        d.res.string("font").to_string()
    } else {
        d.res.string("font2").to_string()
    };
    let font = Font::load(&fontname);

    let mut st = XJack {
        char_width: font.char_width(),
        line_height: font.ascent() + font.descent() + 1,
        font,
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        width,
        height,
        s: 0,
        columns: 0,
        rows: 0,
        left: 0,
        right: 0,
        x: 0,
        y: 0,
        mode: 0,
        hspace: 0,
        vspace: 0,
        break_para: false,
        caps: false,
        sentences: 0,
        paras: 0,
        scrolling: 0,
        subscrolling: 0,
        pining: 0,
        delay: d.res.int("delay").max(0) as u32,
    };
    st.reshape(d, width, height);

    st.left = 0xFF & random_below(st.columns / 2 + 1);
    st.right = st.left + (0xFF & (random_below(st.columns - st.left) + 10));
    if st.right < st.left + 10 {
        st.right = st.left + 10;
    }
    if st.right > st.columns {
        st.right = st.columns;
    }
    st.x = st.left;
    st.y = 0;

    if width > 200 && height > 200 {
        st.hspace = 40;
        st.vspace = 40;
    }
    Box::new(st)
}

impl XJack {
    /// Where on the page a character cell lands, in pixels, with the origin on
    /// the text baseline as X wants it.
    fn cell(&self, x: i32, y: i32) -> (i32, i32) {
        (
            x * self.char_width + self.hspace,
            y * self.line_height + self.vspace + self.font.ascent(),
        )
    }

    /// Roll the page up. The scroll is done a seventh of a line at a time, so
    /// the paper visibly moves rather than jumping.
    fn scroll(&mut self, d: &mut Dpy) -> u32 {
        self.break_para = false;
        if self.subscrolling > 0 {
            let inc = self.line_height / 7;
            let (w, h) = (self.width, self.height);
            d.win().copy_area_self(&self.gc, 0, inc, w, h - inc, 0, 0);

            /* See? It's OK. He saw it on the television. */
            let bg = self.gc.background;
            d.win().clear_area(bg, 0, h - inc, w, inc);

            self.subscrolling -= inc;
            if self.subscrolling <= 0 {
                self.subscrolling = 0;
                if self.scrolling > 0 {
                    self.scrolling -= 1;
                }
                self.y -= 1;
            }
            return self.delay / 1000;
        } else if self.scrolling != 0 {
            self.subscrolling = self.line_height;
        }

        if self.y < 0 {
            self.y = 0;
        } else if self.y >= self.rows - 1 {
            self.y = self.rows - 1;
        }
        self.delay
    }

    /// See also <http://catalog.com/hopkins/unix-haters/login.html>
    fn pine(&mut self, d: &mut Dpy) -> u32 {
        let n1 = "NFS server overlook not responding, still trying...";
        let n2 = "NFS server overlook ok.";
        let prev = self.pining;

        if self.pining == 0 {
            self.pining = 1 + random_below(3);
        }

        if prev != 0 {
            self.type_out(d, n2);
        }
        self.y += 1;
        self.x = 0;
        self.pining -= 1;

        if self.pining != 0 {
            self.type_out(d, n1);
        }
        5_000_000
    }

    /// One character per cell, wrapping at the right edge of the page.
    fn type_out(&mut self, d: &mut Dpy, s: &str) {
        for ch in s.chars() {
            let (px, py) = self.cell(self.x, self.y);
            let (font, gc) = (self.font, self.gc.clone());
            d.win().draw_string(&gc, &font, px, py, &ch.to_string());
            self.x += 1;
            if self.x >= self.columns {
                self.x = 0;
                self.y += 1;
            }
        }
    }
}

/// Keys next to each other on a typewriter, so a typo lands on a plausible
/// neighbour rather than any letter at all.
const TYPO: &[&str] = &[
    "asqw", "ASQW", "bgvhn", "cxdfv", "dserfcx", "ewsdrf", "Jhuikmn", "kjiol,m", "lkop;.,",
    "mnjk,", "nbhjm", "oiklp09", "pol;(-0", "redft54", "sawedxz", "uyhji87", "wqase32", "yuhgt67",
    ".,l;/",
];

impl Screenhack for XJack {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut this_delay = self.delay;

        if self.scrolling != 0 {
            return self.scroll(d);
        }
        if self.pining != 0 {
            return self.pine(d);
        }

        let bytes = SOURCE.as_bytes();
        let mut word_length = 0;
        let mut s2 = self.s;
        while s2 < bytes.len() && bytes[s2] != b' ' {
            word_length += 1;
            s2 += 1;
        }
        let here = if self.s < bytes.len() {
            bytes[self.s]
        } else {
            0
        };

        if self.break_para || (here != b' ' && self.x + word_length >= self.right) {
            self.x = self.left;
            self.y += 1;
            if self.break_para {
                self.y += 1;
            }
            self.break_para = false;

            if self.mode == 1 || self.mode == 2 {
                /* 1 = left margin goes southwest; 2 = southeast */
                self.left += if self.mode == 1 { 1 } else { -1 };
                if self.left >= self.right - 10 {
                    if self.right < self.columns - 10 && random() & 1 != 0 {
                        self.right += 0xFF & random_below(self.columns - self.right);
                    } else {
                        self.mode = 2;
                    }
                } else if self.left <= 0 {
                    self.left = 0;
                    self.mode = 1;
                }
            } else if self.mode == 3 || self.mode == 4 {
                /* 3 = right margin goes southeast; 4 = southwest */
                self.right += if self.mode == 3 { 1 } else { -1 };
                if self.right >= self.columns {
                    self.right = self.columns;
                    self.mode = 4;
                } else if self.right <= self.left + 10 {
                    self.mode = 3;
                }
            }

            if self.y >= self.rows - 1 {
                /* bottom of page */
                self.scrolling = 1;
                return self.scroll(d);
            }
        }

        if here != b' ' && here != 0 {
            let mut c = here as char;
            let mut xshift = 0;
            let mut yshift = 0;
            if random_below(50) == 0 {
                /* mis-strike */
                xshift = random_below(self.char_width / 3 + 1);
                yshift = random_below(self.line_height / 6 + 1);
                if random_below(3) == 0 {
                    yshift *= 2;
                }
                if random() & 1 != 0 {
                    xshift = -xshift;
                }
                if random() & 1 != 0 {
                    yshift = -yshift;
                }
            }

            if random_below(250) == 0 {
                /* introduce adjacent-key typo */
                if let Some(row) = TYPO.iter().find(|t| t.starts_with(c)) {
                    let rest = &row[1..];
                    let i = (0xFF & (random_below(rest.len() as i32) + 1)) as usize;
                    if let Some(n) = row.chars().nth(i) {
                        c = n;
                    }
                }
            }

            /* caps typo */
            if c.is_ascii_lowercase() && (self.caps || random_below(350) == 0) {
                c = c.to_ascii_uppercase();
                if c == 'O' && random() & 1 != 0 {
                    c = '0';
                }
            }

            // The overstrike: having struck the key once, occasionally strike
            // it again a whisker away, which is what a worn typewriter does.
            loop {
                let (px, py) = self.cell(self.x, self.y);
                let (font, gc) = (self.font, self.gc.clone());
                d.win()
                    .draw_string(&gc, &font, px + xshift, py + yshift, &c.to_string());
                if !(xshift == 0 && yshift == 0 && (random() & 3000) == 0) {
                    break;
                }
                let mut off = self.font.ascent() / 10;
                if off <= 0 {
                    off = 1;
                }
                if random() & 1 != 0 {
                    off = (f64::from(off) * 1.5) as i32;
                }
                if random() & 1 != 0 {
                    xshift -= off;
                } else {
                    yshift -= off;
                }
            }

            let mistyped = !c.eq_ignore_ascii_case(&(here as char));
            let redo = if mistyped {
                random_below(10) == 0 /* backup to correct */
            } else {
                random_below(400) == 0 /* fail to advance */
            };
            if redo {
                self.x -= 1;
                self.s = self.s.saturating_sub(1);
                if self.delay != 0 {
                    this_delay +=
                        0xFFFF & (self.delay + (random() % (self.delay.saturating_mul(10)).max(1)));
                }
            }
        }

        self.x += 1;
        self.s += 1;

        if random_below(200) == 0 {
            if random() & 1 != 0 && self.s != 0 {
                self.s -= 1; /* duplicate character */
            } else if self.s < bytes.len() {
                self.s += 1; /* skip character */
            }
        }

        if self.s >= bytes.len() {
            self.sentences += 1;
            self.caps = random_below(40) == 0; /* capitalize sentence */

            if random_below(10) == 0
                || (self.mode == 0 && (random_below(10) == 0 || self.sentences > 20))
            {
                self.break_para = true;
                self.sentences = 0;
                self.paras += 1;

                if random() & 1 != 0 {
                    self.mode = 0; /* mode=0 50% of the time */
                } else {
                    self.mode = 0xFF & random_below(5);
                }

                if random_below(2) == 0 {
                    /* re-pick margins */
                    self.left = 0xFF & random_below(self.columns / 3);
                    self.right = self.columns - (0xFF & random_below(self.columns / 3));

                    if random_below(3) == 0 {
                        /* sometimes be wide */
                        self.right = self.left + (self.right - self.left) / 2;
                    }
                }

                if self.right - self.left <= 10 {
                    /* introduce sanity */
                    self.left = 0;
                    self.right = self.columns;
                }

                if self.right - self.left > 50 {
                    /* if wide, shrink and move */
                    self.left += 0xFF & random_below(self.columns - 50 + 1);
                    self.right = self.left + (0xFF & (random_below(40) + 10));
                }

                /* oh, gag. */
                if self.mode == 0 && self.right - self.left < 25 && self.columns > 40 {
                    self.right += 20;
                    if self.right > self.columns {
                        self.left -= self.right - self.columns;
                    }
                }

                if self.right - self.left < 5 {
                    self.left = self.right - 5;
                }
                if self.left < 0 {
                    self.left = 0;
                }
                if self.right - self.left < 5 {
                    self.right = self.left + 5;
                }
            }
            self.s = 0;
        }

        if self.delay != 0 {
            if random_below(3) == 0 {
                this_delay += 0xFFFFFF & (random() % (self.delay.saturating_mul(5)).max(1) + 1);
            }
            if self.break_para {
                this_delay += 0xFFFFFF & (random() % (self.delay.saturating_mul(15)).max(1) + 1);
            }
        }

        if self.paras > 5 && random_below(1000) == 0 && self.y < self.rows - 2 {
            return self.pine(d);
        }
        this_delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.columns = (width - self.hspace - self.hspace) / self.char_width;
        self.rows = (height - self.vspace - self.vspace) / self.line_height;
        self.rows -= 1;
        self.columns -= 1;

        /* If the window is stupidly small, just truncate. */
        if self.rows < 4 {
            self.rows = 4;
        }
        if self.columns < 12 {
            self.columns = 12;
        }

        if self.y > self.rows {
            self.y = self.rows - 1;
        }
        if self.x > self.columns {
            self.x = self.columns - 2;
        }
        if self.right > self.columns {
            self.right = self.columns;
        }
        if self.left > self.columns - 20 {
            self.left = self.columns - 20;
        }
        if self.left < 0 {
            self.left = 0;
        }
        d.clear_window();
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if let XEvent::ButtonPress { .. } = event {
            self.scrolling += 1;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background:		#FFF0B4",
    ".foreground:		#000000",
    "*fpsSolid:		true",
    "*fpsTop:		true",
    ".font:  Special Elite 24, American Typewriter 24, Courier 24, monospace 24",
    ".font2: Special Elite 12, American Typewriter 12, Courier 12, monospace 12",
    ".font3: Special Elite 48, American Typewriter 48, Courier 48, monospace 48",
    "*delay:		50000",
];

const OPTS: &[Opt] =
    &[Opt::slider("delay", "Speed", 0.0, 200_000.0, 1000.0, 0, "50000").inverted()];

pub static DEF: SaverDef = SaverDef {
    slug: "xjack",
    label: "XJack",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=wSOiSrEbxu4"),
        blurb: "A novel by Jack Torrance.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
