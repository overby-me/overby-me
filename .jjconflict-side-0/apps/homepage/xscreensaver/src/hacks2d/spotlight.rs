//! Port of `hacks/spotlight.c`.
//!
//! ```text
//! spotlight - an xscreensaver module
//! Copyright (c) 1999, 2001 Rick Schultz <rick.schultz@gmail.com>
//!
//! loosely based on the BackSpace module "StefView" by Darcy Brockbank
//!
//! modified from a module from the xscreensaver distribution
//!
//! xscreensaver, Copyright (c) 1992-2006 Jamie Zawinski <jwz@jwz.org>
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
//! A picture is loaded off screen and a circular window onto it is swept
//! around, so all you ever see is the part the spotlight is over. The path is
//! two sines of different periods, fifteen seconds across and twelve down, so
//! it never quite repeats.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Gc, ImageLoad, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent,
    random, screenhack_event_helper,
};

const X_PERIOD: f64 = 15000.0;
const Y_PERIOD: f64 = 12000.0;

struct State {
    sizex: i32,
    sizey: i32,
    delay: u32,
    duration: f64,
    start_time: f64,
    window_gc: Gc,
    /// Radius of the spotlight in pixels.
    radius: i32,

    /// The picture, kept off screen.
    pm: Pixmap,

    /// Upper left corner of the buffer square, and its width.
    x: i32,
    y: i32,
    s: i32,

    /// A random offset from the clock, so two copies of spotlight running at
    /// once do not move in step.
    off: f64,

    img_loader: Option<ImageLoad>,
    loading: bool,
}

impl State {
    /// Copy the picture onto the screen through a circular hole.
    ///
    /// Upstream builds a one-bit pixmap with a filled circle in it and hands
    /// that to `XSetClipMask`, then copies the picture through it.
    fn copy_through_circle(&self, d: &mut Dpy) {
        let r = self.radius;
        let (cx, cy) = (self.x + 2 * r, self.y + 2 * r);
        let r2 = r * r;
        for dy in -r..=r {
            let py = cy + dy;
            if py < 0 || py >= self.sizey {
                continue;
            }
            let span = ((r2 - dy * dy) as f64).sqrt() as i32;
            for dx in -span..=span {
                let px = cx + dx;
                if px < 0 || px >= self.sizex {
                    continue;
                }
                let p = self.pm.get_pixel(px, py);
                d.win().put_pixel(px, py, p);
            }
        }
    }

    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    /// The channel draws into the window, so lift the picture off it and put
    /// the window back to black: upstream loads straight into an off-screen
    /// pixmap and the screen never shows the whole thing.
    fn image_arrived(&mut self, d: &mut Dpy) {
        self.pm = d.win_ref().sub_image(0, 0, self.sizex, self.sizey);
        self.start_time = d.time;
        self.loading = false;
        let (w, h) = (self.sizex, self.sizey);
        d.win().fill_rectangle(&self.window_gc, 0, 0, w, h);
    }

    fn onestep(&mut self, d: &mut Dpy) {
        if self.loading {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return;
        }

        if self.start_time + self.duration < d.time {
            self.start_load(d);
            return;
        }

        // s = width of buffer.
        self.s = self.radius * 4;
        let now = d.time * 1000.0 + self.off;

        // Find new x, y. Upstream saves the old position into oldx one line
        // after computing the new one, so the speed limiting that follows can
        // never fire; the path is whatever the sines say.
        self.x = (((1.0 + (now / X_PERIOD * 2.0 * std::f64::consts::PI).sin()) / 2.0)
            * (self.sizex - self.s / 2) as f64) as i32
            - self.s / 4;
        self.y = (((1.0 + (now / Y_PERIOD * 2.0 * std::f64::consts::PI).sin()) / 2.0)
            * (self.sizey - self.s / 2) as f64) as i32
            - self.s / 4;

        // Upstream keeps an off-screen buffer to avoid flicker, and skips it
        // on platforms that already double-buffer. This runtime hands the
        // whole framebuffer to the canvas once a frame, so it is one of those:
        // clear the window, then paint the circle straight onto it. That is
        // the branch upstream compiles on macOS, and it is the one that leaves
        // no trail behind the spotlight.
        d.clear_window();
        self.copy_through_circle(d);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let bg = d.res.pixel("background");
    let (sizex, sizey) = (d.width(), d.height());

    // Read parameters, keep them sane.
    let mut radius = d.res.int("radius");
    if radius < 0 {
        radius = 125;
    }
    if sizex > 2560 || sizey > 2560 {
        radius *= 2; // Retina displays.
    }
    // Do not let the spotlight be bigger than the window.
    while radius as f64 > sizex as f64 * 0.45 {
        radius /= 2;
    }
    while radius as f64 > sizey as f64 * 0.45 {
        radius /= 2;
    }
    radius = radius.max(4);

    let mut st = State {
        sizex,
        sizey,
        delay: d.res.int("delay").max(1) as u32,
        duration: d.res.int("duration").max(1) as f64,
        start_time: 0.0,
        window_gc: Gc::new(bg, bg),
        radius,
        pm: Pixmap::new(sizex, sizey),
        x: 0,
        y: 0,
        s: radius * 4,
        off: (random() % 100_000) as f64,
        img_loader: None,
        loading: false,
    };
    d.clear_window();
    st.start_load(d);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.onestep(d);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream only clears, and notes it should resize the playfield.
        // Here it can: the picture is reloaded at the new size.
        self.sizex = width;
        self.sizey = height;
        self.pm = Pixmap::new(width, height);
        d.clear_window();
        self.start_load(d);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            // Ask for a fresh picture on the next step.
            self.start_time = f64::NEG_INFINITY;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*dontClearRoot: True",
    "*fpsSolid: true",
    "*delay: 10000",
    "*duration: 120",
    "*radius: 125",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::slider("radius", "Spotlight size", 5.0, 350.0, 5.0, 0, "125"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "spotlight",
    label: "Spotlight",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Rick Schultz and Jamie Zawinski",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=av29CVh2UeM"),
        blurb: "A spotlight scanning across a black screen, illuminating a loaded image when it passes.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
