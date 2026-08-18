//! Port of `hacks/boxfit.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 2005-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Boxfit -- fills space with a gradient of growing boxes or circles.
//!
//! Written by jwz, 21-Feb-2005.
//!
//! Inspired by http://www.levitated.net/daily/levBoxFitting.html
//! ```
//!
//! Boxes are dropped at random into whatever space is left and grow outwards
//! until they touch something, then stop. When nowhere is left to drop one,
//! the whole packing shrinks away again and starts over. Colours come off a
//! gradient across the screen, or out of a picture if one is on offer.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, ImageLoad, Opt, Pixel, Runner, SaverDef, Screenhack, SelectItem, StartArgs,
    XColor, XEvent, XImage, random, screenhack_event_helper,
};

const ALIVE: u8 = 1;
const CHANGED: u8 = 2;
const UNDEAD: u8 = 4;

const FULL_CIRCLE: i32 = 360 * 64;

#[derive(Clone, Copy, Default)]
struct Shape {
    fill_color: Pixel,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    flags: u8,
}

struct State {
    gc: Gc,
    fg_color: Pixel,
    bg_color: Pixel,
    border_size: i32,
    spacing: i32,
    inc: i32,

    /// `-1` for random, 0 for squares, 1 for circles.
    mode: i32,
    circles_p: bool,
    growing_p: bool,
    color_horiz_p: bool,

    box_count: usize,
    boxes: Vec<Shape>,

    image: Option<XImage>,
    ncolors: usize,
    colors: Vec<XColor>,
    delay: u32,
    countdown: i32,

    grab: bool,
    peek: bool,
    done_once: bool,
    img_loader: Option<ImageLoad>,

    width: i32,
    height: i32,
}

fn boxes_overlap_p(a: &Shape, b: &Shape, pad: i32) -> bool {
    // Two rectangles overlap if the max of the tops is less than the min of
    // the bottoms and the max of the lefts is less than the min of the rights.
    let maxleft = (a.x - pad).max(b.x);
    let maxtop = (a.y - pad).max(b.y);
    let minright = (a.x + a.w + pad + pad - 1).min(b.x + b.w);
    let minbot = (a.y + a.h + pad + pad - 1).min(b.y + b.h);
    maxtop < minbot && maxleft < minright
}

fn circles_overlap_p(a: &Shape, b: &Shape, pad: i32) -> bool {
    let ar = a.w / 2; // radius
    let br = b.w / 2;
    let ax = a.x + ar; // centre
    let ay = a.y + ar;
    let bx = b.x + br;
    let by = b.y + br;
    let d2 = (bx - ax) * (bx - ax) + (by - ay) * (by - ay);
    let r2 = (ar + br + pad) * (ar + br + pad);
    d2 < r2
}

impl State {
    /// Would this box touch a wall, or any box already placed? `skip` is the
    /// candidate's own index, which upstream excludes by pointer identity.
    fn box_collides_p(&self, a: &Shape, pad: i32, skip: usize) -> bool {
        if a.x - pad < 0
            || a.y - pad < 0
            || a.x + a.w + pad + pad >= self.width
            || a.y + a.h + pad + pad >= self.height
        {
            return true;
        }
        for (i, b) in self.boxes.iter().enumerate() {
            if i == skip {
                continue;
            }
            let hit = if self.circles_p {
                circles_overlap_p(a, b, pad)
            } else {
                boxes_overlap_p(a, b, pad)
            };
            if hit {
                return true;
            }
        }
        false
    }

    fn reset_boxes(&mut self, d: &mut Dpy) {
        self.boxes.clear();
        self.growing_p = true;
        self.color_horiz_p = random() & 1 == 1;

        if !self.done_once {
            self.mode = match d.res.string("mode").to_ascii_lowercase().as_str() {
                "squares" | "square" => 0,
                "circles" | "circle" => 1,
                // "random", and anything else. Upstream exits on a bad value;
                // a query string is not a command line, so take it as random.
                _ => -1,
            };
        }
        self.circles_p = if self.mode == -1 {
            random() & 1 == 1
        } else {
            self.mode == 1
        };
        self.done_once = true;

        if self.image.is_some() || self.grab {
            self.image = None;
            d.clear_window();
            self.img_loader = d.load_image_async_simple(None);
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
        } else {
            self.ncolors = d.res.int("colors").max(1) as usize; // re-get
            self.colors = make_smooth_colormap(self.ncolors);
            self.ncolors = self.colors.len().max(1);
            d.clear_window();
        }
    }

    /// Upstream loads into an offscreen pixmap unless `peek` is set, so the
    /// picture it is sampling never shows. The runtime's image channel draws
    /// into the window, so instead the window is cleared straight away when
    /// peeking is off, and left up for two seconds when it is on.
    fn image_arrived(&mut self, d: &mut Dpy) {
        self.image = Some(d.win_ref().sub_image(0, 0, self.width, self.height));
        if self.peek {
            self.countdown = 2_000_000;
        } else {
            d.clear_window();
        }
    }

    fn mark_all_changed(&mut self) {
        for b in &mut self.boxes {
            b.flags |= CHANGED;
        }
    }

    fn grow_boxes(&mut self) -> u32 {
        let inc2 = self.inc + self.spacing + self.border_size;
        let mut live_count = 0;

        // Check collisions, and grow whatever is still clear.
        for i in 0..self.boxes.len() {
            if self.boxes[i].flags & ALIVE == 0 {
                continue;
            }
            let a = self.boxes[i];
            if self.box_collides_p(&a, inc2, i) {
                self.boxes[i].flags &= !ALIVE;
                continue;
            }
            live_count += 1;
            let b = &mut self.boxes[i];
            b.x -= self.inc;
            b.y -= self.inc;
            b.w += self.inc + self.inc;
            b.h += self.inc + self.inc;
            b.flags |= CHANGED;
        }

        // Drop more in.
        while live_count < self.box_count {
            self.boxes.push(Shape {
                flags: CHANGED,
                ..Shape::default()
            });
            let idx = self.boxes.len() - 1;

            for _ in 0..100 {
                let a = Shape {
                    x: inc2 + (random() % (self.width - inc2).max(1) as u32) as i32,
                    y: inc2 + (random() % (self.height - inc2).max(1) as u32) as i32,
                    w: 0,
                    h: 0,
                    ..self.boxes[idx]
                };
                self.boxes[idx] = a;
                if !self.box_collides_p(&a, inc2, idx) {
                    self.boxes[idx].flags |= ALIVE;
                    live_count += 1;
                    break;
                }
            }

            if self.boxes[idx].flags & ALIVE == 0 || self.boxes.len() > 65535 {
                // Too many retries; go into fade-out mode now.
                self.boxes.pop();
                self.growing_p = false;
                return 2_000_000;
            }

            // Pick a colour for this box.
            let (x, y) = (self.boxes[idx].x, self.boxes[idx].y);
            self.boxes[idx].fill_color = match &self.image {
                Some(img) => img.get_pixel(x % img.width(), y % img.height()),
                None => {
                    let n = if self.color_horiz_p {
                        x as usize * self.ncolors / self.width.max(1) as usize
                    } else {
                        y as usize * self.ncolors / self.height.max(1) as usize
                    };
                    self.colors[n % self.ncolors].pixel
                }
            };
        }

        self.delay
    }

    /// `None` means everything has shrunk away and the packing should restart.
    fn shrink_boxes(&mut self) -> Option<u32> {
        let mut remaining = 0;
        for b in &mut self.boxes {
            if b.w <= 0 || b.h <= 0 {
                continue;
            }
            b.x += self.inc;
            b.y += self.inc;
            b.w -= self.inc + self.inc;
            b.h -= self.inc + self.inc;
            b.flags |= CHANGED;
            b.w = b.w.max(0);
            b.h = b.h.max(0);
            if b.w > 0 && b.h > 0 {
                remaining += 1;
            }
        }
        if remaining == 0 {
            None
        } else {
            Some(self.delay)
        }
    }

    fn draw_boxes(&mut self, d: &mut Dpy) {
        for i in 0..self.boxes.len() {
            let b = self.boxes[i];
            if b.flags & UNDEAD != 0 || b.flags & CHANGED == 0 {
                continue;
            }
            self.boxes[i].flags &= !CHANGED;

            if !self.growing_p {
                // When shrinking, black out an area outside of the border
                // before re-drawing the box.
                let margin = self.inc + self.border_size;
                self.gc.set_foreground(self.bg_color);
                if self.circles_p {
                    d.win().fill_arc(
                        &self.gc,
                        b.x - margin,
                        b.y - margin,
                        b.w + margin * 2,
                        b.h + margin * 2,
                        0,
                        FULL_CIRCLE,
                    );
                } else {
                    d.win().fill_rectangle(
                        &self.gc,
                        b.x - margin,
                        b.y - margin,
                        b.w + margin * 2,
                        b.h + margin * 2,
                    );
                }
                if b.w <= 0 || b.h <= 0 {
                    self.boxes[i].flags |= UNDEAD; // really very dead now
                }
            }

            if b.w <= 0 || b.h <= 0 {
                continue;
            }

            self.gc.set_foreground(b.fill_color);
            if self.circles_p {
                d.win()
                    .fill_arc(&self.gc, b.x, b.y, b.w, b.h, 0, FULL_CIRCLE);
            } else {
                d.win().fill_rectangle(&self.gc, b.x, b.y, b.w, b.h);
            }

            if self.border_size > 0 {
                // Upstream indexes the colormap with the fill colour itself,
                // which on a TrueColor visual is a packed pixel rather than an
                // index. Kept as it is: the arbitrary-looking borders are what
                // the hack looks like.
                let bd = if self.image.is_some() {
                    self.fg_color
                } else {
                    self.colors[(b.fill_color as usize + self.ncolors / 2) % self.ncolors].pixel
                };
                self.gc.set_foreground(bd);
                if self.circles_p {
                    d.win()
                        .draw_arc(&self.gc, b.x, b.y, b.w, b.h, 0, FULL_CIRCLE);
                } else {
                    d.win().draw_rectangle(&self.gc, b.x, b.y, b.w, b.h);
                }
            }
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg_color = d.res.pixel("foreground");
    let bg_color = d.res.pixel("background");
    let grab = d.res.bool("grab");

    let mut border_size = d.res.int("borderSize").max(0);
    let mut spacing = d.res.int("spacing");
    if d.width() > 2560 || d.height() > 2560 {
        // Retina displays.
        border_size *= 3;
        spacing *= 3;
    }

    let mut gc = Gc::new(fg_color, bg_color);
    gc.set_line_width(border_size);

    let mut st = State {
        gc,
        fg_color,
        bg_color,
        border_size,
        spacing,
        inc: d.res.int("growBy").max(1),
        mode: -1,
        circles_p: false,
        growing_p: true,
        color_horiz_p: false,
        box_count: d.res.int("boxCount").max(1) as usize,
        boxes: Vec::new(),
        image: None,
        ncolors: d.res.int("colors").max(1) as usize,
        colors: Vec::new(),
        delay: d.res.int("delay").max(0) as u32,
        countdown: 0,
        grab,
        peek: d.res.bool("peek"),
        done_once: false,
        img_loader: None,
        width: d.width(),
        height: d.height(),
    };

    st.reset_boxes(d);
    st.mark_all_changed();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.img_loader.is_some() {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return self.delay;
        }

        if self.countdown > 0 {
            self.countdown -= self.delay as i32;
            if self.countdown <= 0 {
                self.countdown = 0;
                d.clear_window();
            }
            return self.delay;
        }

        if self.growing_p {
            self.draw_boxes(d);
            self.grow_boxes()
        } else {
            let delay = self.shrink_boxes();
            self.draw_boxes(d);
            match delay {
                Some(delay) => delay,
                None => {
                    self.reset_boxes(d);
                    1_000_000
                }
            }
        }
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.mark_all_changed();
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.growing_p = !self.growing_p;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: #444444",
    "*fpsSolid: true",
    "*delay: 20000",
    "*mode: random",
    "*colors: 64",
    "*boxCount: 50",
    "*growBy: 1",
    "*spacing: 1",
    "*borderSize: 1",
    "*grab: False",
    "*peek: False",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Boxes or circles",
    },
    SelectItem {
        value: "squares",
        label: "Boxes only",
    },
    SelectItem {
        value: "circles",
        label: "Circles only",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::spin("boxCount", "Boxes", 1.0, 1000.0, "50"),
    Opt::spin("growBy", "Grow by", 1.0, 10.0, "1"),
    Opt::spin("spacing", "Spacing", 1.0, 10.0, "1"),
    Opt::spin("borderSize", "Border", 1.0, 10.0, "1"),
    Opt::select("mode", "Shape", MODES, "random"),
    Opt::boolean("grab", "Use pictures", "False"),
    Opt::boolean("peek", "Peek at underlying images", "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "boxfit",
    label: "Box Fit",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=8GkcbBbcwBk"),
        blurb: "Packs the screen with growing squares or circles which grow until they touch, then stop.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
