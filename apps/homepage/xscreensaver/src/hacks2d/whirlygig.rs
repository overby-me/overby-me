//! Port of `hacks/whirlygig.c`.
//!
//! ```text
//! Whirlygig -- an experiment in X programming
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//!  When I was in trigonometry class as a kid, I remember being fascinated
//!  by the beauty of the shapes one receives when playing with sine waves
//!  Here is a little experiment to show that beauty is simple
//! ```
//!
//! A clock ticks, and two of eight little formulas turn that tick into an x
//! and a y. A spot is drawn there, then another for each of a handful of
//! whirlies whose clocks run a bit ahead of each other, and each of those is
//! repeated down a few lines pushed apart by a sine of their own. Erase,
//! advance the clock, draw again. That is the whole hack, and the shapes come
//! out of nothing but which formula was picked for which axis.
//!
//! The formulas are the author playing, and the comments in the original say
//! as much: one is called fun, one is test, one is "me goofing off". They are
//! kept exactly as written, including the parts that are almost certainly
//! accidents. `funky` recomputes its own argument in the second branch and
//! divides an angle by 180 twice over. `innie` beats one cosine against
//! another at a period of two million. Straightening any of it out would
//! change the picture, which is the only thing here that is not an accident.
//!
//! The clock is where the strangest of it lives. It starts at a random
//! thirty-two bit number, the per-whirly offsets are added in a signed int, and
//! the result is handed to formulas that take an unsigned long: a clock that
//! has gone negative arrives as a number near two to the sixty-fourth, and the
//! cosine of that is a perfectly repeatable arbitrary value. That is done
//! deliberately here, because it is what the hack looks like.
//!
//! The explain option is not offered. It draws a sentence naming the current
//! mode, and there is no font path in this runtime yet.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{BLACK, XColor, make_uniform_colormap};
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent, frand, random,
};

const NCOLORS: usize = 100;
const FULL_CYCLE: u64 = 429_496_729;

/// The eight little formulas, in the order the mode resource names them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Spin,
    Funky,
    Circle,
    Linear,
    Test,
    Fun,
    Innie,
    Lissajous,
}

impl Mode {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "spin" => Mode::Spin,
            "funky" => Mode::Funky,
            "circle" => Mode::Circle,
            "linear" => Mode::Linear,
            "test" => Mode::Test,
            "fun" => Mode::Fun,
            "innie" => Mode::Innie,
            "lissajous" => Mode::Lissajous,
            _ => return None,
        })
    }

    /// `random() % (int) lissajous_mode`, which stops one short of the last
    /// mode: an unnamed mode string never picks lissajous.
    fn any() -> Self {
        Mode::of(random() % 7)
    }

    fn of(n: u32) -> Self {
        match n {
            1 => Mode::Funky,
            2 => Mode::Circle,
            3 => Mode::Linear,
            4 => Mode::Test,
            5 => Mode::Fun,
            6 => Mode::Innie,
            7 => Mode::Lissajous,
            _ => Mode::Spin,
        }
    }
}

/// All the random pieces of information the formulas may want.
struct Info {
    /// A modifier of the argument to cos, so it changes the frequency.
    xspeed: f64,
    yspeed: f64,
    xamplitude: f64,
    yamplitude: f64,
    whirlies: i32,
    nlines: i32,
    half_width: i32,
    half_height: i32,
    speed: u64,
    trail: bool,
    color_modifier: i32,
    xoffset: f64,
    yoffset: f64,
    offset_period: f64,
}

struct State {
    gc: Gc,
    info: Info,
    /// The global clock.
    current_time: u64,
    start_time: u64,
    /// Whether each axis re-randomises its formula every so often.
    xchange: bool,
    ychange: bool,
    xmode: Mode,
    ymode: Mode,
    /// For innie.
    modifier: f64,
    wrap: bool,
    pos: [i32; 2],
    /// One cell per whirly per line, so the last spot can be painted out.
    last_x: Vec<i32>,
    last_y: Vec<i32>,
    last_size: Vec<i32>,
    current_color: usize,
    colors: Vec<XColor>,
}

/// The formulas all take an `unsigned long` clock. A clock that has gone
/// negative in the signed arithmetic above arrives here enormous, and stays
/// that way, because that is what the C does.
fn as_clock(t: i32) -> u64 {
    (t as i64) as u64
}

/// `%` against a count that a one-pixel window makes zero. Upstream divides by
/// it unguarded.
fn nonzero(n: i32) -> u64 {
    n.max(1) as u64
}

impl State {
    fn spin(&mut self, the_time: u64, index: usize) {
        let i = &self.info;
        let funky = ((the_time % 360) as f64 / 180.0) * std::f64::consts::PI;
        self.pos[index] = if index == 0 {
            let the_cos = (the_time as f64 / (180.0 * i.xspeed)).cos();
            let dist_mod_x = funky.cos() * (i.half_width - 50) as f64;
            (i.xamplitude * (the_cos * dist_mod_x)) as i32 + i.half_width
        } else {
            let the_sin = (the_time as f64 / (180.0 * i.yspeed)).sin();
            let dist_mod_y = funky.sin() * (i.half_height - 50) as f64;
            (i.yamplitude * (the_sin * dist_mod_y)) as i32 + i.half_height
        };
    }

    /// Me goofing off. The second branch recomputes `new_time` for itself, and
    /// both divide an angle that is already in radians by 180 again.
    fn funky(&mut self, the_time: u64, index: usize) {
        let i = &self.info;
        let new_time = ((the_time % 360) as f64 / 180.0) * std::f64::consts::PI;
        self.pos[index] = if index == 0 {
            let time_modifier = (new_time / 180.0).cos();
            let the_cos = ((new_time * i.xspeed) + (time_modifier * 80.0)).cos();
            let dist_mod_x = new_time.cos() * (i.half_width - 50) as f64;
            (i.xamplitude * (the_cos * dist_mod_x)) as i32 + i.half_width
        } else {
            let time_modifier = (new_time / 180.0).sin();
            let the_sin = ((new_time * i.yspeed) + (time_modifier * 80.0)).sin();
            let dist_mod_y = new_time.sin() * (i.half_height - 50) as f64;
            (i.yamplitude * (the_sin * dist_mod_y)) as i32 + i.half_height
        };
    }

    /// Does something or other. Looks cool, though.
    fn innie(&mut self, the_time: u64, index: usize) {
        let i = &self.info;
        let t = the_time as f64;
        let frequency = 2_000_000.0 + (self.modifier * (t / 100.0).cos());
        let amplitude = 200.0 * (t / frequency).cos();
        let fun = 150.0 * (t / 2000.0).cos();
        self.pos[index] = if index == 0 {
            let horiz_mod = (fun * (t / 100.0).cos()) as i32 + i.half_width;
            (amplitude * (t / 100.0 * i.xspeed).cos()) as i32 + horiz_mod
        } else {
            let vert_mod = (fun * (t / 100.0).sin()) as i32 + i.half_height;
            (amplitude * (t / 100.0 * i.yspeed).sin()) as i32 + vert_mod
        };
    }

    /// A pretty standard lissajous curve, `x = a sin(nt + c)`, `y = b sin(t)`,
    /// except that the n and c modifiers are cyclic as well.
    fn lissajous(&mut self, the_time: u64, index: usize) {
        let i = &self.info;
        let t = the_time as f64;
        let time = t / 100.0;
        let fun = 15.0 * (t / 800.0).cos();
        let weird = ((time / 1_100_000.0) / 1000.0).cos();
        self.pos[index] = if index == 0 {
            (i.xamplitude * 200.0 * ((weird * time) + fun).sin()) as i32 + i.half_width
        } else {
            (i.yamplitude * 200.0 * time.sin()) as i32 + i.half_height
        };
    }

    /// Graphs the x and y positions as you trace the edge of a circle over
    /// time. `test` is the same formula; upstream keeps it separate as a place
    /// to play with ideas.
    fn circle(&mut self, the_time: u64, index: usize) {
        let i = &self.info;
        let t = the_time as f64;
        self.pos[index] = if index == 0 {
            (i.xamplitude * ((t / 100.0 * i.xspeed).cos() * (i.half_width / 2) as f64)) as i32
                + i.half_width
        } else {
            (i.yamplitude * ((t / 100.0 * i.yspeed).sin() * (i.half_height / 2) as f64)) as i32
                + i.half_height
        };
    }

    /// The coolest. A triangle wave off the clock, used as the amplitude of a
    /// circle.
    fn fun(&mut self, the_time: u64, index: usize) {
        let i = &self.info;
        let max = i.half_width;
        let span = nonzero(max * 2);
        let m = (the_time % span) as i64;
        let amplitude = if m < max as i64 {
            max as i64 - (m - max as i64)
        } else {
            m
        } - max as i64;
        let t = the_time as f64;
        self.pos[index] = if index == 0 {
            (amplitude as f64 * (t / 100.0 * i.xspeed).cos()) as i32 + i.half_width
        } else {
            (amplitude as f64 * (t / 100.0 * i.yspeed).sin()) as i32 + i.half_height
        };
    }

    /// Draws a straight line.
    fn linear(&mut self, the_time: u64, index: usize) {
        let i = &self.info;
        self.pos[index] = if index == 0 {
            ((the_time / 2) % nonzero(i.half_width * 2)) as i32
        } else {
            ((the_time / 2) % nonzero(i.half_height * 2)) as i32
        };
    }

    fn apply(&mut self, mode: Mode, the_time: u64, index: usize) {
        match mode {
            Mode::Spin => self.spin(the_time, index),
            Mode::Funky => self.funky(the_time, index),
            Mode::Circle | Mode::Test => self.circle(the_time, index),
            Mode::Linear => self.linear(the_time, index),
            Mode::Fun => self.fun(the_time, index),
            Mode::Innie => self.innie(the_time, index),
            Mode::Lissajous => self.lissajous(the_time, index),
        }
    }
}

/// Wrap a coordinate back onto the screen, once.
fn preen(current: i32, max: i32) -> i32 {
    let mut current = current;
    if current > max {
        current -= max;
    }
    if current < 0 {
        current += max;
    }
    current
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (w, h) = (d.width(), d.height());
    let bg = d.res.pixel("background");
    let fg = d.res.pixel("foreground");

    let xmode_str = d.res.string("xmode").to_string();
    let ymode_str = d.res.string("ymode").to_string();
    let xmode = Mode::parse(&xmode_str).unwrap_or_else(Mode::any);
    let ymode = Mode::parse(&ymode_str).unwrap_or_else(Mode::any);

    // Upstream copies `current_time` into `start_time` before it has been set,
    // so the clock starts a random distance ahead of the last mode change and
    // the first frame almost always changes mode.
    let start_time = 0;
    let st_res = d.res.int("start_time");
    let current_time = if st_res == -1 {
        random() as u64
    } else {
        st_res as u64
    };

    let mut whirlies = d.res.int("whirlies");
    if whirlies == -1 {
        whirlies = 1 + (random() % 15) as i32;
    }
    let mut nlines = d.res.int("nlines");
    if nlines == -1 {
        nlines = 1 + (random() % 5) as i32;
    }
    let mut color_modifier = d.res.int("color_modifier");
    if color_modifier == -1 {
        color_modifier = 1 + (random() % 25) as i32;
    }
    // Upstream's cells are a fixed hundred by a hundred, so that is the most
    // of either it can actually keep track of.
    whirlies = whirlies.min(100);
    nlines = nlines.min(100);
    let cells = (whirlies.max(0) * nlines.max(0)) as usize;

    let info = Info {
        xspeed: d.res.float("xspeed"),
        yspeed: d.res.float("yspeed"),
        xamplitude: d.res.float("xamplitude"),
        yamplitude: d.res.float("yamplitude"),
        whirlies,
        nlines,
        half_width: w / 2,
        half_height: h / 2,
        speed: d.res.int("speed").max(0) as u64,
        trail: d.res.bool("trail"),
        color_modifier,
        xoffset: d.res.float("xoffset"),
        yoffset: d.res.float("yoffset"),
        offset_period: d.res.float("offset_period"),
    };

    Box::new(State {
        gc: Gc::new(fg, bg),
        info,
        current_time,
        start_time,
        xchange: xmode_str == "change",
        ychange: ymode_str == "change",
        xmode,
        ymode,
        modifier: 3000.0 + frand(1500.0),
        wrap: d.res.bool("wrap"),
        pos: [0, 0],
        last_x: vec![0; cells],
        last_y: vec![0; cells],
        last_size: vec![0; cells],
        current_color: 1 + (random() % NCOLORS as u32) as usize,
        colors: make_uniform_colormap(NCOLORS),
    })
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        const CHANGE_TIME: u64 = 4000;
        let due = self.current_time.wrapping_sub(self.start_time) > CHANGE_TIME;

        // A changing axis picks from the four in the middle of the list only,
        // and when both change they take separate rolls.
        if self.xchange && self.ychange {
            if due {
                self.start_time = self.current_time;
                self.xmode = Mode::of(1 + random() % 4);
                self.ymode = Mode::of(1 + random() % 4);
            }
        } else if self.xchange {
            if due {
                self.start_time = self.current_time;
                self.xmode = Mode::of(1 + random() % 4);
            }
        } else if self.ychange && due {
            self.start_time = self.current_time;
            self.ymode = Mode::of(1 + random() % 3);
        }

        self.current_color += 1;
        if self.current_color >= NCOLORS {
            self.current_color = 0;
        }

        let (whirlies, nlines) = (self.info.whirlies, self.info.nlines);
        for wcount in 0..whirlies {
            let color_offset = (self.current_color as i64
                + (self.info.color_modifier as i64 * wcount as i64))
                .rem_euclid(NCOLORS as i64) as usize;

            // The distance between whirlies grows with each whirly.
            let mut internal_time = 0i32;
            if self.current_time != 0 {
                internal_time = (self.current_time as i32)
                    .wrapping_add(10 * wcount)
                    .wrapping_add(wcount * wcount);
            }
            let the_time = as_clock(internal_time);

            let (xmode, ymode) = (self.xmode, self.ymode);
            self.apply(xmode, the_time, 0);
            self.apply(ymode, the_time, 1);

            for lcount in 0..nlines {
                let arg = (internal_time as f64 * self.info.offset_period) / 90.0;
                let line_offset = 20.0 * lcount as f64 * arg.sin();
                let size = (15.0 + 5.0 * (internal_time as f64 / 180.0).sin()) as i32;
                let cell = (wcount * nlines + lcount) as usize;

                // First delete the old circle. Upstream paints it out in the
                // screen's black rather than in the background resource.
                if !self.info.trail {
                    self.gc.set_foreground(BLACK);
                    let (lx, ly, ls) = (self.last_x[cell], self.last_y[cell], self.last_size[cell]);
                    d.win().fill_arc(&self.gc, lx, ly, ls, ls, 0, FULL_CIRCLE);
                }

                // Now draw in the new circle.
                let mut xpos = (self.info.xoffset * line_offset) as i32 + self.pos[0];
                let mut ypos = (self.info.yoffset * line_offset) as i32 + self.pos[1];
                if self.wrap {
                    xpos = preen(xpos, self.info.half_width * 2);
                    ypos = preen(ypos, self.info.half_height * 2);
                }
                self.last_x[cell] = xpos;
                self.last_y[cell] = ypos;
                self.last_size[cell] = size;
                self.gc.set_foreground(self.colors[color_offset].pixel);
                d.win()
                    .fill_arc(&self.gc, xpos, ypos, size, size, 0, FULL_CIRCLE);
            }
        }

        if self.current_time == FULL_CYCLE {
            self.current_time = 1;
        } else {
            self.current_time = self.current_time.wrapping_add(self.info.speed);
        }

        10000
    }

    fn reshape(&mut self, _d: &mut Dpy, _width: i32, _height: i32) {}

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*xspeed: 1.0",
    "*yspeed: 1.0",
    "*xamplitude: 1.0",
    "*yamplitude: 1.0",
    "*whirlies: -1",
    "*nlines: -1",
    "*xmode: change",
    "*ymode: change",
    "*speed: 1",
    "*trail: false",
    "*color_modifier: -1",
    "*start_time: -1",
    "*xoffset: 1.0",
    "*yoffset: 1.0",
    "*offset_period: 1",
    "*wrap: False",
];

const X_MODES: &[SelectItem] = &[
    SelectItem {
        value: "change",
        label: "X random",
    },
    SelectItem {
        value: "spin",
        label: "X spin",
    },
    SelectItem {
        value: "funky",
        label: "X funky",
    },
    SelectItem {
        value: "circle",
        label: "X circle",
    },
    SelectItem {
        value: "linear",
        label: "X linear",
    },
    SelectItem {
        value: "test",
        label: "X test",
    },
    SelectItem {
        value: "fun",
        label: "X fun",
    },
    SelectItem {
        value: "innie",
        label: "X innie",
    },
    SelectItem {
        value: "lissajous",
        label: "X lissajous",
    },
];

const Y_MODES: &[SelectItem] = &[
    SelectItem {
        value: "change",
        label: "Y random",
    },
    SelectItem {
        value: "spin",
        label: "Y spin",
    },
    SelectItem {
        value: "funky",
        label: "Y funky",
    },
    SelectItem {
        value: "circle",
        label: "Y circle",
    },
    SelectItem {
        value: "linear",
        label: "Y linear",
    },
    SelectItem {
        value: "test",
        label: "Y test",
    },
    SelectItem {
        value: "fun",
        label: "Y fun",
    },
    SelectItem {
        value: "innie",
        label: "Y innie",
    },
    SelectItem {
        value: "lissajous",
        label: "Y lissajous",
    },
];

const OPTS: &[Opt] = &[
    Opt::spin("whirlies", "Whirlies", -1.0, 50.0, "-1"),
    Opt::spin("nlines", "Lines", -1.0, 50.0, "-1"),
    Opt::slider("xspeed", "X speed", 0.0, 10.0, 0.1, 1, "1.0"),
    Opt::slider("yspeed", "Y speed", 0.0, 10.0, 0.1, 1, "1.0"),
    Opt::slider("xamplitude", "X amplitude", 0.0, 10.0, 0.1, 1, "1.0"),
    Opt::slider("yamplitude", "Y amplitude", 0.0, 10.0, 0.1, 1, "1.0"),
    Opt::select("xmode", "X mode", X_MODES, "change"),
    Opt::select("ymode", "Y mode", Y_MODES, "change"),
    Opt::boolean("trail", "Leave a trail", "false"),
    Opt::boolean("wrap", "Wrap the screen", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "whirlygig",
    label: "Whirlygig",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ashton Trey Belew",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=Y2JTY7bssPM"),
        blurb: "Zooming chains of sinusoidal spots.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
