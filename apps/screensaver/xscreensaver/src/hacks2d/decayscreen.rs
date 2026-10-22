//! Port of `hacks/decayscreen.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Modified 1998-05-27 by David Wald <wald@wald.org>
//! Modified 2000-03-24 by Vince Levey <vincel@vincel.org>
//! ```
//!
//! Melts whatever picture it is given by copying random sub-rectangles one
//! pixel off in a biased direction, over and over. The whole hack is
//! `XCopyArea` on the window, which is why it needs an image to start from: see
//! [`crate::runtime::image`] for where that comes from.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, GXFunc, Gc, ImageLoad, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs,
    XEvent, random, random_below, screenhack_event_helper,
};

/// The melt styles, in upstream's order: `mode` indexes this list and "random"
/// picks one from it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Shuffle,
    Up,
    Left,
    Right,
    Down,
    UpLeft,
    DownLeft,
    UpRight,
    DownRight,
    In,
    Out,
    Melt,
    Stretch,
    Fuzz,
}

const MODES: [Mode; 14] = [
    Mode::Shuffle,
    Mode::Up,
    Mode::Left,
    Mode::Right,
    Mode::Down,
    Mode::UpLeft,
    Mode::DownLeft,
    Mode::UpRight,
    Mode::DownRight,
    Mode::In,
    Mode::Out,
    Mode::Melt,
    Mode::Stretch,
    Mode::Fuzz,
];

impl Mode {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "shuffle" => Mode::Shuffle,
            "up" => Mode::Up,
            "left" => Mode::Left,
            "right" => Mode::Right,
            "down" => Mode::Down,
            "upleft" => Mode::UpLeft,
            "downleft" => Mode::DownLeft,
            "upright" => Mode::UpRight,
            "downright" => Mode::DownRight,
            "in" => Mode::In,
            "out" => Mode::Out,
            "melt" => Mode::Melt,
            "stretch" => Mode::Stretch,
            "fuzz" => Mode::Fuzz,
            _ => return None,
        })
    }
}

/// Which way a copied rectangle drifts. Upstream spells these as sixteen-entry
/// tables so the bias is a weighting, not a direction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drift {
    L,
    R,
    U,
    D,
}
use Drift::{D, L, R, U};

const NO_BIAS: [Drift; 16] = [L, L, L, L, R, R, R, R, U, U, U, U, D, D, D, D];
const UP_BIAS: [Drift; 16] = [L, L, L, L, R, R, R, R, U, U, U, U, U, U, D, D];
const DOWN_BIAS: [Drift; 16] = [L, L, L, L, R, R, R, R, U, U, D, D, D, D, D, D];
const LEFT_BIAS: [Drift; 16] = [L, L, L, L, L, L, R, R, U, U, U, U, D, D, D, D];
const RIGHT_BIAS: [Drift; 16] = [L, L, R, R, R, R, R, R, U, U, U, U, D, D, D, D];
const UPLEFT_BIAS: [Drift; 16] = [L, L, L, L, L, R, R, R, U, U, U, U, U, D, D, D];
const DOWNLEFT_BIAS: [Drift; 16] = [L, L, L, L, L, R, R, R, U, U, U, D, D, D, D, D];
const UPRIGHT_BIAS: [Drift; 16] = [L, L, L, R, R, R, R, R, U, U, U, U, U, D, D, D];
const DOWNRIGHT_BIAS: [Drift; 16] = [L, L, L, R, R, R, R, R, U, U, U, D, D, D, D, D];

struct DecayScreen {
    gc: Gc,
    delay: u32,
    /// Seconds to melt one image before asking for the next.
    duration: i64,
    mode: Mode,
    random_p: bool,
    sizex: i32,
    sizey: i32,
    /// The image as it arrived, so a resize can re-centre it.
    saved: Option<crate::runtime::Pixmap>,
    saved_w: i32,
    saved_h: i32,
    start_time: f64,
    fuzz_toggle: bool,
    img_loader: Option<ImageLoad>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let name = d.res.string("mode").to_string();
    let (mode, random_p) = match Mode::from_name(&name) {
        Some(m) => (m, false),
        // Upstream warns on an unknown mode and falls back to random.
        None => (MODES[random_below(MODES.len() as i32) as usize], true),
    };

    let mut gc = Gc::new(d.res.pixel("background"), d.res.pixel("background"));
    gc.set_function(GXFunc::Copy);

    let mut st = DecayScreen {
        gc,
        delay: d.res.int("delay").max(0) as u32,
        duration: d.res.int("duration").max(1) as i64,
        mode,
        random_p,
        sizex: d.width(),
        sizey: d.height(),
        saved: None,
        saved_w: 0,
        saved_h: 0,
        start_time: d.time,
        fuzz_toggle: false,
        img_loader: None,
    };
    st.load_image(d);
    Box::new(st)
}

impl DecayScreen {
    fn load_image(&mut self, d: &mut Dpy) {
        self.sizex = d.width();
        self.sizey = d.height();
        self.img_loader = d.load_image_async_simple(None);
        if self.img_loader.is_none() {
            // Answered on the spot (no host, so colour bars).
            self.image_arrived(d);
        }
    }

    /// Everything upstream does the frame a load finishes.
    fn image_arrived(&mut self, d: &mut Dpy) {
        self.start_time = d.time;
        if self.random_p {
            self.mode = MODES[random_below(MODES.len() as i32) as usize];
        }
        if matches!(self.mode, Mode::Melt | Mode::Stretch) {
            // Make sure the screen eventually turns the background colour.
            let sizex = self.sizex;
            d.win().draw_line(&self.gc, 0, 0, sizex, 0);
        }
        let mut saved = d.new_pixmap(self.sizex, self.sizey);
        saved.copy_area(&self.gc, d.win_ref(), 0, 0, self.sizex, self.sizey, 0, 0);
        self.saved_w = self.sizex;
        self.saved_h = self.sizey;
        self.saved = Some(saved);
    }

    fn bias(&self) -> &'static [Drift; 16] {
        match self.mode {
            Mode::Up => &UP_BIAS,
            Mode::Left => &LEFT_BIAS,
            Mode::Right => &RIGHT_BIAS,
            Mode::Down => &DOWN_BIAS,
            Mode::UpLeft => &UPLEFT_BIAS,
            Mode::DownLeft => &DOWNLEFT_BIAS,
            Mode::UpRight => &UPRIGHT_BIAS,
            Mode::DownRight => &DOWNRIGHT_BIAS,
            _ => &NO_BIAS,
        }
    }
}

/// `nrnd`: upstream's guard against `random() % 0`.
fn nrnd(x: i32) -> i32 {
    if x > 0 { random_below(x) } else { x }
}

impl Screenhack for DecayScreen {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.img_loader.is_some() {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return self.delay;
        }

        if d.time >= self.start_time + self.duration as f64 {
            self.load_image(d);
            return self.delay;
        }

        // Retina displays melt in bigger steps or it takes all day.
        let off = if self.sizex > 2560 || self.sizey > 2560 {
            2
        } else {
            1
        };

        let mut current_bias = self.bias();
        let (left, top, width, height, toleft, totop);

        match self.mode {
            Mode::Melt | Mode::Stretch => {
                left = nrnd(self.sizex / 2);
                top = nrnd(self.sizey);
                width = nrnd(self.sizex / 2) + self.sizex / 2 - left;
                height = nrnd(self.sizey - top);
                toleft = left;
                totop = top + off;
            }

            // By Vince Levey, inspired by the "melt" mode of Paul Haeberli's
            // IrisGL "scrhack" from about 1991.
            Mode::Fuzz => {
                let mut l = nrnd(self.sizex - 1);
                let mut t = nrnd(self.sizey - 1);
                self.fuzz_toggle = !self.fuzz_toggle;
                if self.fuzz_toggle {
                    totop = t;
                    height = off;
                    let mut tl = nrnd(self.sizex - 1);
                    if tl > l {
                        width = tl - l;
                        tl = l;
                        l += 1;
                    } else {
                        width = l - tl;
                        l = tl;
                        tl += 1;
                    }
                    toleft = tl;
                } else {
                    toleft = l;
                    width = off;
                    let mut tt = nrnd(self.sizey - 1);
                    if tt > t {
                        height = tt - t;
                        tt = t;
                        t += 1;
                    } else {
                        height = t - tt;
                        t = tt;
                        tt += 1;
                    }
                    totop = tt;
                }
                left = l;
                top = t;
            }

            _ => {
                left = nrnd(self.sizex - 1);
                top = nrnd(self.sizey);
                width = nrnd(self.sizex - left);
                height = nrnd(self.sizey - top);

                if matches!(self.mode, Mode::In | Mode::Out) {
                    let x = left + (width / 2);
                    let y = top + (height / 2);
                    let cx = self.sizex / 2;
                    let cy = self.sizey / 2;
                    current_bias = match (self.mode, x > cx, y > cy) {
                        (Mode::In, true, true) => &UPLEFT_BIAS,
                        (Mode::In, false, true) => &UPRIGHT_BIAS,
                        (Mode::In, false, false) => &DOWNRIGHT_BIAS,
                        (Mode::In, true, false) => &DOWNLEFT_BIAS,
                        (_, true, true) => &DOWNRIGHT_BIAS,
                        (_, false, true) => &DOWNLEFT_BIAS,
                        (_, false, false) => &UPLEFT_BIAS,
                        (_, true, false) => &UPRIGHT_BIAS,
                    };
                }

                match current_bias[(random() % current_bias.len() as u32) as usize] {
                    L => {
                        toleft = left - off;
                        totop = top;
                    }
                    R => {
                        toleft = left + off;
                        totop = top;
                    }
                    U => {
                        toleft = left;
                        totop = top - off;
                    }
                    D => {
                        toleft = left;
                        totop = top + off;
                    }
                }
            }
        }

        if self.mode == Mode::Stretch {
            let (sizex, sizey) = (self.sizex, self.sizey);
            d.win().copy_area_self(
                &self.gc,
                0,
                sizey - top - off * 2,
                sizex,
                top + off,
                0,
                sizey - top - off,
            );
        } else {
            d.win()
                .copy_area_self(&self.gc, left, top, width, height, toleft, totop);
        }

        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        let Some(saved) = self.saved.as_ref() else {
            return; // Image might not be loaded yet.
        };
        let (sw, sh) = (self.saved_w, self.saved_h);
        d.clear_window();
        d.win().copy_area(
            &self.gc,
            saved,
            0,
            0,
            sw,
            sh,
            (width - sw) / 2,
            (height - sh) / 2,
        );
        self.sizex = width;
        self.sizey = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            // Next frame decides the duration has elapsed, so a poke means
            // "give me the next picture".
            self.start_time = f64::NEG_INFINITY;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background:			Black",
    ".foreground:			Yellow",
    "*dontClearRoot:		True",
    "*fpsSolid:			True",
    "*delay:			10000",
    "*mode:			random",
    "*duration:			120",
];

const MELT_MODES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random melt style",
    },
    SelectItem {
        value: "shuffle",
        label: "Shuffle melt",
    },
    SelectItem {
        value: "up",
        label: "Melt up",
    },
    SelectItem {
        value: "down",
        label: "Melt down",
    },
    SelectItem {
        value: "left",
        label: "Melt left",
    },
    SelectItem {
        value: "right",
        label: "Melt right",
    },
    SelectItem {
        value: "upleft",
        label: "Melt up, left",
    },
    SelectItem {
        value: "upright",
        label: "Melt up, right",
    },
    SelectItem {
        value: "downleft",
        label: "Melt down, left",
    },
    SelectItem {
        value: "downright",
        label: "Melt down, right",
    },
    SelectItem {
        value: "in",
        label: "Melt towards center",
    },
    SelectItem {
        value: "out",
        label: "Melt away from center",
    },
    SelectItem {
        value: "melt",
        label: "Melty melt",
    },
    SelectItem {
        value: "stretch",
        label: "Stretchy melt",
    },
    SelectItem {
        value: "fuzz",
        label: "Fuzzy melt",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::select("mode", "Melt style", MELT_MODES, "random"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "decayscreen",
    label: "Decay Screen",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "David Wald, Vivek Khera, Jamie Zawinski, and Vince Levey",
        year: "1993",
        video: Some("https://www.youtube.com/watch?v=dFlyRTObuDo"),
        blurb: "Melts the picture into a puddle.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
