//! Port of `hacks/noseguy.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2018 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Make a little guy with a big nose and a hat wanter around the screen,
//! spewing out messages.  Derived from xnlock by
//! Dan Heller <argv@danheller.com>.
//! ```
//!
//! Eight drawings of the same man and a state machine that decides, every
//! interval, whether to keep walking, stop and look around, or say something.
//! The walk is two frames alternating with the whole figure bouncing two pixels
//! up and down, and there is a comment upstream about a bug this works around,
//! which is preserved here along with the workaround.
//!
//! What he says comes from [`crate::runtime::text`], the same source the other
//! text hacks read, and the speech box is sized from however many lines have
//! arrived.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::font::Font;
use crate::runtime::png;
use crate::runtime::{
    About, Dpy, Fb, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent, XRectangle,
    random,
};
use std::rc::Rc;

/// A frame: the drawing, and a bitmap of the man's outline. X has no alpha, so
/// this is the pair every sprite is made of.
struct Frame {
    p: Pixmap,
    m: Option<Rc<Fb>>,
}

const LEFT: u32 = 0o01;
const RIGHT: u32 = 0o02;
const DOWN: u32 = 0o04;
const UP: u32 = 0o10;
const FRONT: u32 = 0o20;
const X_INCR: i32 = 3;
const Y_INCR: i32 = 2;

/// Which of the eight drawings, in the order upstream loads them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Left1,
    Left2,
    Right1,
    Right2,
    LeftFront,
    RightFront,
    Front,
    Down,
}

/// What to do at the next interval. Upstream keeps a function pointer; there
/// are only two, and one of them is "carry on talking".
#[derive(Clone, Copy, PartialEq)]
enum Next {
    Move,
    Talk,
}

const MAX_LINES: usize = 10;
const LINE_LEN: usize = 256;

struct NoseGuy {
    width: i32,
    height: i32,
    fg_gc: Gc,
    bg_gc: Gc,
    text_fg_gc: Gc,
    text_bg_gc: Gc,
    font: Font,
    x: i32,
    y: i32,
    /// How long until the next step, in milliseconds. Upstream's `interval`.
    interval: u64,
    frames: Vec<Frame>,
    pix_w: i32,
    pix_h: i32,
    next_fn: Next,
    move_length: i32,
    move_dir: u32,
    walk_lastdir: u32,
    walk_up: i32,
    walk_frame: Which,
    talking: bool,
    s_rect: XRectangle,
    words: String,
}

impl NoseGuy {
    /// Upstream's `COPY` macro: paint the background out, then blit the drawing
    /// through its own outline so nothing square shows.
    fn copy(&mut self, d: &mut Dpy, which: Which, x2: i32, y2: i32) {
        let (w, h) = (self.pix_w, self.pix_h);
        d.win().fill_rectangle(&self.bg_gc, x2, y2, w, h);
        let f = &self.frames[which as usize];
        let mut gc = self.fg_gc.clone();
        if let Some(m) = &f.m {
            gc.set_clip_mask(Rc::clone(m)).set_clip_origin(x2, y2);
        }
        d.win().copy_area(&gc, &f.p, 0, 0, w, h, x2, y2);
    }

    /// Decide where to go, then take one step of it.
    fn do_move(&mut self, d: &mut Dpy) {
        if self.move_length == 0 {
            let mut tries = 0;
            self.move_dir = 0;
            if random() & 1 != 0 && self.think(d) {
                self.talk(d, false); /* sets the timeout to itself */
                return;
            }
            if random().is_multiple_of(3) {
                self.interval = self.look(d);
                if self.interval != 0 {
                    self.next_fn = Next::Move;
                    return;
                }
            }
            self.interval = 20 + u64::from(random() % 100);
            loop {
                if tries == 0 {
                    self.move_length = self.width / 100 + (random() % 90) as i32;
                    tries = 8;
                } else {
                    tries -= 1;
                }
                // Upstream's guard against a window too small for any direction
                // to be legal, which would otherwise spin here forever.
                if tries == 0 && self.move_length <= 1 {
                    self.move_length = 1;
                    break;
                }
                let far_x = X_INCR * self.move_length;
                let far_y = Y_INCR * self.move_length;
                let can_left = self.x - far_x >= 5;
                let can_right = self.x + far_x <= self.width - 70;
                let can_up = self.y - far_y >= 5;
                let can_down = self.y + far_y <= self.height - 70;
                match random() % 8 {
                    0 if can_left => self.move_dir = LEFT,
                    1 if can_right => self.move_dir = RIGHT,
                    2 if can_up => {
                        self.move_dir = UP;
                        self.interval = 40;
                    }
                    3 if can_down => {
                        self.move_dir = DOWN;
                        self.interval = 20;
                    }
                    4 if can_left && can_up => self.move_dir = LEFT | UP,
                    5 if can_right && can_up => self.move_dir = RIGHT | UP,
                    6 if can_left && can_down => self.move_dir = LEFT | DOWN,
                    7 if can_right && can_down => self.move_dir = RIGHT | DOWN,
                    _ => {}
                }
                if self.move_dir != 0 {
                    break;
                }
            }
        }
        if self.move_dir != 0 {
            self.walk(d, self.move_dir);
        }
        self.move_length -= 1;
        self.next_fn = Next::Move;
    }

    fn walk(&mut self, d: &mut Dpy, dir: u32) {
        let mut incr = 0;

        if dir & (LEFT | RIGHT) != 0 {
            /* left/right movement (maybe up/down too) */
            self.walk_up = -self.walk_up; /* bouncing effect, even into a wall */
            if dir & LEFT != 0 {
                incr = X_INCR;
                self.walk_frame = if self.walk_up < 0 {
                    Which::Left1
                } else {
                    Which::Left2
                };
            } else {
                incr = -X_INCR;
                self.walk_frame = if self.walk_up < 0 {
                    Which::Right1
                } else {
                    Which::Right2
                };
            }
            if (self.walk_lastdir == FRONT || self.walk_lastdir == DOWN) && dir & UP != 0 {
                // Upstream: "workaround silly bug that leaves screen dust when
                // guy is facing forward or down and moves up-left/right."
                let (x, y) = (self.x, self.y);
                self.copy(d, self.walk_frame, x, y);
            }
            /* note that maybe neither UP nor DOWN is set */
            if dir & UP != 0 && self.y > Y_INCR {
                self.y -= Y_INCR;
            } else if dir & DOWN != 0 && self.y < self.height - self.pix_h {
                self.y += Y_INCR;
            }
        } else if dir == UP {
            self.y -= Y_INCR;
            let (x, y) = (self.x, self.y);
            self.copy(d, Which::Front, x, y);
        } else if dir == DOWN {
            self.y += Y_INCR;
            let (x, y) = (self.x, self.y);
            self.copy(d, Which::Down, x, y);
        } else if dir == FRONT && self.walk_frame != Which::Front {
            if self.walk_up > 0 {
                self.walk_up = -self.walk_up;
            }
            self.walk_frame = if self.walk_lastdir & LEFT != 0 {
                Which::LeftFront
            } else if self.walk_lastdir & RIGHT != 0 {
                Which::RightFront
            } else {
                Which::Front
            };
            let (x, y) = (self.x, self.y);
            self.copy(d, self.walk_frame, x, y);
        }

        if dir & LEFT != 0 {
            while incr > 0 {
                incr -= 1;
                self.x -= 1;
                let (x, y, up) = (self.x, self.y, self.walk_up);
                self.copy(d, self.walk_frame, x, y + up);
            }
        } else if dir & RIGHT != 0 {
            while incr < 0 {
                incr += 1;
                self.x += 1;
                let (x, y, up) = (self.x, self.y, self.walk_up);
                self.copy(d, self.walk_frame, x, y + up);
            }
        }
        self.walk_lastdir = dir;
    }

    /// Turn to face the viewer, and say whether there is anything to say.
    fn think(&mut self, d: &mut Dpy) -> bool {
        if random() & 1 != 0 {
            self.walk(d, FRONT);
        }
        random() & 1 != 0
    }

    /// Either put up the speech box, or take it down again.
    fn talk(&mut self, d: &mut Dpy, force_erase: bool) {
        if self.talking || force_erase {
            if !self.talking {
                return;
            }
            let r = self.s_rect;
            d.win()
                .fill_rectangle(&self.bg_gc, r.x - 5, r.y - 5, r.width + 10, r.height + 10);
            self.talking = false;
            if !force_erase {
                self.next_fn = Next::Move;
            }
            self.interval = 0;
            // Might as well check the window for size changes now.
            self.width = d.width() + 2;
            self.height = d.height() + 2;
            return;
        }

        if self.words.is_empty() {
            self.talking = false;
            return;
        }
        self.talking = true;
        self.walk(d, FRONT);

        let words = self.words.replace('\t', " ");
        let mut lines: Vec<String> = Vec::new();
        let mut total = 0;
        let mut width = 0;
        // Upstream treats "no newline at all, or nothing after it" as one line
        // measured whole, and anything else as lines up to a limit.
        if !words.contains('\n') || words.split_once('\n').is_some_and(|(_, r)| r.is_empty()) {
            total = words.len();
            width = self.font.text_width(&words);
            lines.push(words.chars().take(LINE_LEN - 1).collect());
        } else {
            for line in words.split('\n') {
                width = width.max(self.font.text_width(line));
                total += line.len();
                lines.push(line.chars().take(LINE_LEN - 1).collect());
                if lines.len() == MAX_LINES {
                    /* Message too long */
                    break;
                }
            }
        }
        let height = lines.len() as i32;

        // Fifteen pixels of margin each way, and the box goes above him unless
        // there is no room, in which case it goes below.
        let font_height = self.font.ascent() + self.font.descent();
        self.s_rect.width = width + 30;
        self.s_rect.height = height * font_height + 30;
        if self.x - self.s_rect.width - 10 < 5 {
            self.s_rect.x = 5;
        } else {
            self.s_rect.x = self.x + 32 - (self.s_rect.width + 15) / 2;
            if self.s_rect.x + self.s_rect.width + 15 > self.width - 5 {
                self.s_rect.x = self.width - 15 - self.s_rect.width;
            }
        }
        if self.y - self.s_rect.height - 10 < 5 {
            self.s_rect.y = self.y + self.pix_h + 5;
        } else {
            self.s_rect.y = self.y - 5 - self.s_rect.height;
        }

        let r = self.s_rect;
        d.win()
            .fill_rectangle(&self.text_bg_gc, r.x, r.y, r.width, r.height);

        // A box five pixels thick, and a thin one inside it.
        self.text_fg_gc.set_line_width(5);
        d.win()
            .draw_rectangle(&self.text_fg_gc, r.x, r.y, r.width - 1, r.height - 1);
        self.text_fg_gc.set_line_width(0);
        d.win().draw_rectangle(
            &self.text_fg_gc,
            r.x + 7,
            r.y + 7,
            r.width - 15,
            r.height - 15,
        );

        let mut ty = 15 + font_height;
        for line in &lines {
            let line = line.trim_end_matches(['\r', '\n']);
            d.win()
                .draw_string(&self.text_fg_gc, &self.font, r.x + 15, r.y + ty, line);
            ty += font_height;
        }

        // Long enough to read it: fifteen characters a second, and never less
        // than two seconds.
        self.interval = ((total / 15) * 1000).max(2000) as u64;
        self.next_fn = Next::Talk;
        self.words.clear();
    }

    /// Stop and look about. Returns how long to hold the pose, or zero for
    /// "carry on walking instead".
    fn look(&mut self, d: &mut Dpy) -> u64 {
        if !random().is_multiple_of(3) {
            let which = if random() & 1 != 0 {
                Which::Down
            } else {
                Which::Front
            };
            let (x, y) = (self.x, self.y);
            self.copy(d, which, x, y);
            return 1000;
        }
        if random().is_multiple_of(5) {
            return 0;
        }
        if !random().is_multiple_of(3) {
            let which = if random() & 1 != 0 {
                Which::LeftFront
            } else {
                Which::RightFront
            };
            let (x, y) = (self.x, self.y);
            self.copy(d, which, x, y);
            return 1000;
        }
        if random().is_multiple_of(5) {
            return 0;
        }
        let which = if random() & 1 != 0 {
            Which::Left1
        } else {
            Which::Right1
        };
        let (x, y) = (self.x, self.y);
        self.copy(d, which, x, y);
        1000
    }

    /// Take whatever text has arrived, up to ten lines of it.
    fn fill_words(&mut self, d: &mut Dpy) {
        let mut lines = self.words.matches('\n').count();
        while self.words.len() < 10240 - 1 && lines < MAX_LINES {
            match d.text_getc() {
                Some(c) => {
                    if c == b'\n' {
                        lines += 1;
                    }
                    self.words.push(c as char);
                }
                None => break,
            }
        }
    }
}

impl Screenhack for NoseGuy {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.fill_words(d);
        match self.next_fn {
            Next::Move => self.do_move(d),
            Next::Talk => self.talk(d, false),
        }
        // Upstream's interval is in milliseconds and the driver wants
        // microseconds, so this is its `interval * 1000`.
        (self.interval * 1000).min(u64::from(u32::MAX)) as u32
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width + 2;
        self.height = height + 2;
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // Forty columns by fifteen lines is what upstream asks the text source for.
    d.text_reshape(40, 15);

    let mut frames = Vec::with_capacity(crate::images::NOSE.len());
    let (mut pix_w, mut pix_h) = (1, 1);
    for bytes in crate::images::NOSE {
        // The sprites are compiled in, so a failure here is a broken build; an
        // empty frame keeps the man invisible rather than stopping the saver.
        let (p, m) = png::decode(bytes).unwrap_or_else(|| (Pixmap::new(1, 1), None));
        pix_w = p.width();
        pix_h = p.height();
        frames.push(Frame {
            p,
            m: m.map(Rc::new),
        });
    }

    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");
    // Upstream falls back to the reverse of the main pair when the text colours
    // were not given, which is why the box is light on dark like the man.
    let text_fg = match d.res.get("textForeground") {
        Some(_) => d.res.pixel("textForeground"),
        None => bg,
    };
    let text_bg = match d.res.get("textBackground") {
        Some(_) => d.res.pixel("textBackground"),
        None => fg,
    };

    Box::new(NoseGuy {
        width: d.width() + 2,
        height: d.height() + 2,
        fg_gc: Gc::new(fg, bg),
        bg_gc: Gc::new(bg, fg),
        text_fg_gc: Gc::new(text_fg, text_bg),
        text_bg_gc: Gc::new(text_bg, text_fg),
        font: Font::load(d.res.string("font")),
        x: (d.width() + 2) / 2,
        y: (d.height() + 2) / 2,
        interval: 0,
        frames,
        pix_w,
        pix_h,
        next_fn: Next::Move,
        move_length: 0,
        move_dir: 0,
        walk_lastdir: 0,
        walk_up: 1,
        walk_frame: Which::Front,
        talking: false,
        s_rect: XRectangle::default(),
        words: String::new(),
    })
}

const DEFAULTS: &[&str] = &[
    ".background:	    black",
    ".foreground:	    #CCCCCC",
    "*textForeground: black",
    "*textBackground: #CCCCCC",
    "*fpsSolid:	 true",
    "*program:	 xscreensaver-text",
    "*usePty:      False",
    ".font:	 sans-serif 14",
];

const OPTS: &[Opt] = &[];

pub static DEF: SaverDef = SaverDef {
    slug: "noseguy",
    label: "Nose Guy",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Dan Heller and Jamie Zawinski",
        year: "1992",
        video: Some("https://www.youtube.com/watch?v=ONJlg9Y_TLI"),
        blurb: "A little man with a big nose wanders around saying things.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
