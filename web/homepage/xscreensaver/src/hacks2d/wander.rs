//! Port of `hacks/wander.c`.
//!
//! ```text
//! wander, by Rick Campbell <rick@campbellcentral.org>, 19 December 1998.
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! A random walk that mostly stands still. Each step the walker either stays
//! put or moves one cell in any of the eight directions, wrapping at the edges,
//! and two thousand steps are taken per frame. It changes colour rarely enough
//! that a whole region comes out in one hue before the next takes over, and
//! rarer still it wipes the screen and starts somewhere else.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{BLACK, XColor, make_color_loop};
use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent,
    random_below, screenhack_event_helper,
};

const MAXIMUM_COLOR_COUNT: usize = 256;

/// Steps taken per frame.
const STEPS_PER_FRAME: usize = 2000;

struct Wander {
    gc: Gc,
    delay: u32,
    /// How far the colour index jumps when it changes. Zero means jump at
    /// random.
    advance: usize,
    circles: bool,
    colors: Vec<XColor>,
    color_index: usize,
    /// One step in `density` actually moves; the rest stand still.
    density: i32,
    /// The walk's own grid, in cells rather than pixels.
    width: i32,
    height: i32,
    width_1: i32,
    height_1: i32,
    length_limit: i32,
    reset_limit: i32,
    size: i32,
    x: i32,
    y: i32,
    last_x: i32,
    last_y: i32,
    color: Pixel,
    /// One cell's worth of spot, stamped down when drawing circles.
    pixmap: Pixmap,
    eraser: Option<Eraser>,
    reset_p: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    d.clear_window();

    let mut colors = make_color_loop(
        0,
        1.0,
        1.0,
        120,
        1.0,
        1.0,
        240,
        1.0,
        1.0,
        MAXIMUM_COLOR_COUNT,
    );
    if colors.is_empty() {
        colors = vec![
            XColor::from_rgb16(0, 0, 0),
            XColor::from_rgb16(0xFFFF, 0xFFFF, 0xFFFF),
        ];
    }

    let mut size = d.res.int("size").max(1);
    if d.width() > 2560 || d.height() > 2560 {
        size *= 3; // Retina displays
    }
    // A window smaller than one cell would leave the walk no room at all, and
    // the wrap loop below would never terminate.
    let width = (d.width() / size).max(1);
    let height = (d.height() / size).max(1);

    let n = colors.len() as i32;
    let mut gc = Gc::new(d.res.pixel("foreground"), d.res.pixel("background"));
    gc.set_foreground(colors[random_below(n) as usize].pixel);

    let x = random_below(width);
    let y = random_below(height);
    let color_index = random_below(n) as usize;
    let color = colors[random_below(n) as usize].pixel;

    // The spot to stamp, one cell across, on a black field.
    let mut pixmap = d.new_pixmap(size, size);
    gc.set_foreground(BLACK);
    pixmap.fill_rectangle(&gc, 0, 0, width * size, height * size);
    gc.set_foreground(color);
    pixmap.fill_arc(&gc, 0, 0, size, size, 0, FULL_CIRCLE);

    Box::new(Wander {
        gc,
        delay: d.res.int("delay").max(0) as u32,
        advance: d.res.int("advance").max(0) as usize,
        circles: d.res.bool("circles"),
        color_index,
        density: d.res.int("density").max(1),
        width,
        height,
        width_1: width - 1,
        height_1: height - 1,
        length_limit: d.res.int("length").max(1),
        reset_limit: d.res.int("reset").max(100),
        size,
        x,
        y,
        last_x: x,
        last_y: y,
        color,
        pixmap,
        colors,
        eraser: None,
        reset_p: false,
    })
}

impl Wander {
    /// Restamp the spot in the current colour, for the circle style.
    fn restamp(&mut self) {
        if self.circles {
            let (size, color) = (self.size, self.color);
            self.gc.set_foreground(color);
            self.pixmap
                .fill_arc(&self.gc, 0, 0, size, size, 0, FULL_CIRCLE);
        }
    }
}

impl Screenhack for Wander {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.eraser.is_some() {
            self.eraser = erase_window(d, self.eraser.take());
            return self.delay;
        }

        for _ in 0..STEPS_PER_FRAME {
            if random_below(self.density) != 0 {
                self.x = self.last_x;
                self.y = self.last_y;
            } else {
                self.last_x = self.x;
                self.last_y = self.y;
                // Plus width minus one, so the wrap turns this into a step of
                // minus one, zero or plus one.
                self.x += self.width_1 + random_below(3);
                while self.x >= self.width {
                    self.x -= self.width;
                }
                self.y += self.height_1 + random_below(3);
                while self.y >= self.height {
                    self.y -= self.height;
                }
            }

            if random_below(self.length_limit) == 0 {
                self.color_index = if self.advance == 0 {
                    random_below(self.colors.len() as i32) as usize
                } else {
                    (self.color_index + self.advance) % self.colors.len()
                };
                self.color = self.colors[self.color_index].pixel;
                let color = self.color;
                self.gc.set_foreground(color);
                self.restamp();
            }

            if self.reset_p || random_below(self.reset_limit) == 0 {
                self.reset_p = false;
                self.eraser = erase_window(d, self.eraser.take());
                self.color = self.colors[random_below(self.colors.len() as i32) as usize].pixel;
                self.x = random_below(self.width);
                self.y = random_below(self.height);
                self.last_x = self.x;
                self.last_y = self.y;
                self.restamp();
            }

            let (x, y, size) = (self.x, self.y, self.size);
            if size == 1 {
                d.win().draw_point(&self.gc, x, y);
            } else if self.circles {
                d.win()
                    .copy_area(&self.gc, &self.pixmap, 0, 0, size, size, x * size, y * size);
            } else {
                d.win()
                    .fill_rectangle(&self.gc, x * size, y * size, size, size);
            }
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = (width / self.size).max(1);
        self.height = (height / self.size).max(1);
        self.width_1 = self.width - 1;
        self.height_1 = self.height - 1;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.reset_p = true;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    ".fpsSolid: true",
    ".advance: 1",
    ".density: 2",
    ".length: 25000",
    ".delay: 20000",
    ".reset: 2500000",
    ".circles: False",
    ".size: 1",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("density", "Density", 1.0, 30.0, 1.0, 0, "2").inverted(),
    Opt::slider(
        "reset",
        "Duration",
        10000.0,
        3_000_000.0,
        10000.0,
        0,
        "2500000",
    ),
    Opt::slider("length", "Length", 100.0, 100_000.0, 100.0, 0, "25000"),
    Opt::slider("advance", "Color contrast", 1.0, 100.0, 1.0, 0, "1"),
    Opt::boolean("circles", "Draw spots", "False"),
    Opt::spin("size", "Size", 0.0, 100.0, "1"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "wander",
    label: "Wander",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Rick Campbell",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=2ZZC46Z9wJE"),
        blurb: "A colorful random walk.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
