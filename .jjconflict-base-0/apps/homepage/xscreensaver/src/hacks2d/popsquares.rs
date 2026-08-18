//! Port of `hacks/popsquares.c`.
//!
//! ```text
//! Copyright (c) 2003 Levi Burton <donburton@sbcglobal.net>
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
//! A grid of squares, each stepping through a closed colour ramp between the
//! foreground and the background at its own offset. A square that reaches the
//! end of the ramp jumps to a random point in it, so the grid keeps churning
//! rather than pulsing in step. With twitch on, one such wrap in four
//! rerolls every square at once.
//!
//! Upstream's double buffering is left out: it exists to stop X flickering, and
//! this port composes a whole frame in memory before it reaches the canvas.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{make_color_ramp, rgb_to_hsv};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XColor, random,
};

#[derive(Clone, Copy, Default)]
struct Square {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: usize,
}

struct PopSquares {
    gc: Gc,
    delay: u32,
    /// The `subdivision` resource, before the aspect-ratio correction.
    subdivision: i32,
    border: i32,
    twitch: bool,
    colors: Vec<XColor>,
    /// Square size and grid size.
    sw: i32,
    sh: i32,
    gw: i32,
    gh: i32,
    squares: Vec<Square>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let fg = XColor::from_pixel(d.res.pixel("foreground"));
    let bg = XColor::from_pixel(d.res.pixel("background"));
    let ncolors = d.res.int("ncolors").max(1) as usize;

    let (h1, s1, v1) = rgb_to_hsv(fg.red, fg.green, fg.blue);
    let (h2, s2, v2) = rgb_to_hsv(bg.red, bg.green, bg.blue);

    let mut st = PopSquares {
        gc: Gc::new(fg.pixel, bg.pixel),
        delay: d.res.int("delay").max(0) as u32,
        subdivision: d.res.int("subdivision").max(1),
        border: d.res.int("border"),
        twitch: d.res.bool("twitch"),
        colors: make_color_ramp(h1, s1, v1, h2, s2, v2, ncolors, true),
        sw: 0,
        sh: 0,
        gw: 0,
        gh: 0,
        squares: Vec::new(),
    };
    let (w, h) = (d.width(), d.height());
    st.rebuild(w, h);
    Box::new(st)
}

impl PopSquares {
    fn randomize_colors(&mut self) {
        let n = self.colors.len();
        for s in self.squares.iter_mut() {
            s.color = random() as usize % n;
        }
    }

    /// `popsquares_reshape`: work out the grid for this window and lay it out.
    fn rebuild(&mut self, width: i32, height: i32) {
        let mut s = self.subdivision;

        if width < 100 || height < 100 {
            // Tiny window.
            s = (width.min(height) / 15).max(1);
        }

        if width > height * 5 || height > width * 5 {
            // Weird aspect ratio.
            let r = width as f64 / height as f64;
            if r > 1.0 {
                self.sh = height / s;
                self.sw = width / (s as f64 * r) as i32;
            } else {
                self.sw = width / s;
                self.sh = height / (s as f64 / r) as i32;
            }
        } else {
            self.sw = width / s;
            self.sh = height / s;
        }

        self.gw = if self.sw != 0 { width / self.sw } else { 0 };
        self.gh = if self.sh != 0 { height / self.sh } else { 0 };
        let nsquares = (self.gw * self.gh).max(1) as usize;
        self.squares = vec![Square::default(); nsquares];

        for y in 0..self.gh {
            for x in 0..self.gw {
                self.squares[(self.gw * y + x) as usize] = Square {
                    x: x * self.sw,
                    y: y * self.sh,
                    w: self.sw,
                    h: self.sh,
                    color: 0,
                };
            }
        }

        self.randomize_colors();
    }
}

impl Screenhack for PopSquares {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let n = self.colors.len();
        for i in 0..(self.gw * self.gh) as usize {
            let s = self.squares[i];
            self.gc.set_foreground(self.colors[s.color].pixel);
            let (w, h) = if self.border != 0 {
                (s.w - self.border, s.h - self.border)
            } else {
                (s.w, s.h)
            };
            d.win().fill_rectangle(&self.gc, s.x, s.y, w, h);

            self.squares[i].color += 1;
            if self.squares[i].color == n {
                if self.twitch && random().is_multiple_of(4) {
                    self.randomize_colors();
                } else {
                    self.squares[i].color = random() as usize % n;
                }
            }
        }
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.rebuild(width, height);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: #0000FF",
    ".foreground: #00008B",
    "*delay: 25000",
    "*subdivision: 5",
    "*border: 1",
    "*ncolors: 128",
    "*twitch: False",
    "*fpsSolid: true",
];

const BACKGROUNDS: &[SelectItem] = &[
    SelectItem {
        value: "#FF0000",
        label: "Light red",
    },
    SelectItem {
        value: "#FFFF00",
        label: "Light yellow",
    },
    SelectItem {
        value: "#00FF00",
        label: "Light green",
    },
    SelectItem {
        value: "#00FFFF",
        label: "Light cyan",
    },
    SelectItem {
        value: "#0000FF",
        label: "Light blue",
    },
    SelectItem {
        value: "#FF00FF",
        label: "Light magenta",
    },
];

const FOREGROUNDS: &[SelectItem] = &[
    SelectItem {
        value: "#8C0000",
        label: "Dark red",
    },
    SelectItem {
        value: "#8C8C00",
        label: "Dark yellow",
    },
    SelectItem {
        value: "#008C00",
        label: "Dark green",
    },
    SelectItem {
        value: "#008C8C",
        label: "Dark cyan",
    },
    SelectItem {
        value: "#00008B",
        label: "Dark blue",
    },
    SelectItem {
        value: "#8C008C",
        label: "Dark magenta",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "25000").inverted(),
    Opt::spin("subdivision", "Subdivision", 1.0, 64.0, "5"),
    Opt::spin("border", "Border", 0.0, 5.0, "1"),
    Opt::spin("ncolors", "Number of colors", 1.0, 512.0, "128"),
    Opt::select("background", "Background", BACKGROUNDS, "#0000FF"),
    Opt::select("foreground", "Foreground", FOREGROUNDS, "#00008B"),
    Opt::boolean("twitch", "Twitch", "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "popsquares",
    label: "Pop Squares",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Levi Burton",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=99Aweq7Nypc"),
        blurb: "A pop-art-ish grid of pulsing colors.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
