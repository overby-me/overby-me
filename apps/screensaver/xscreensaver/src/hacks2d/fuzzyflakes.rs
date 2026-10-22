//! Port of `hacks/fuzzyflakes.c`.
//!
//! ```text
//! fuzzyflakes, Copyright (c) 2004
//!  Barry Dmytro <badcherry@mailc.net>
//!
//! ! 2004.06.10 21:05
//! ! - Added support for resizing
//! ! - Added a color scheme generation algorithm
//! !    Thanks to <ZoeB> from #vegans@irc.blitzed.org
//! ! - Added random color generation
//! ! - Fixed errors in the xml config file
//! ! - Cleaned up a few inconsistencies in the code
//! ! - Changed the default color to #EFBEA5
//!
//! ! 2004.05.?? ??:??
//! ! -original creation
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
//! Snowflakes drift down in layers, each one a star of thick arms on a thicker
//! border, swaying sideways on a sine as it falls. The nearer layers fall
//! faster and are drawn larger, which is the whole depth cue.
//!
//! The three colours are worked out from the background alone: convert it to
//! hue, saturation and lightness, then take the two hues a third and two
//! thirds of the way round the wheel. That is why one knob picks the whole
//! palette.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{parse_color, rgb, unrgb};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, SelectItem, StartArgs,
    random, random_below,
};

/// I have need of 1/3 and 2/3 constants later on.
const N1_3: f64 = 0.3333333333;
const N2_3: f64 = 0.6666666666;

#[derive(Clone, Copy, Default)]
struct Flake {
    ticks: f64,
    x_pos: f64,
    y_pos: f64,
    true_x: f64,
    x_offset: f64,
    angle: f64,
}

struct State {
    gc: Gc,
    arms: i32,
    thickness: i32,
    border_thickness: i32,
    radius: i32,
    border_color: Pixel,
    fore_color: Pixel,
    back_color: Pixel,
    layers: usize,
    density: usize,
    delay: u32,
    falling_speed: i32,
    /// One list per layer, nearest layer first.
    flakes: Vec<Vec<Flake>>,
    back: Pixmap,
    width: i32,
    height: i32,
}

/// The colour-matching algorithm the author got from a friend, written in PHP
/// and ported to C: take the background's hue, and pick the two hues a third
/// and two thirds of the way round from it at the same lightness.
///
/// Returns `None` when the background is too grey to see anything against,
/// which is upstream's cue to roll a random colour and try again.
fn color_helper(back: Pixel) -> Option<(Pixel, Pixel)> {
    let (r, g, b) = unrgb(back);
    let f_r = r as f64 / 255.0;
    let f_g = g as f64 / 255.0;
    let f_b = b as f64 / 255.0;

    let max = f_r.max(f_g).max(f_b);
    let min = f_r.min(f_g).min(f_b);
    let lig = (max + min) / 2.0;

    let sat = if max == min {
        0.0
    } else if lig < 0.5 {
        (max - min) / (max + min)
    } else {
        (max - min) / (2.0 - max - min)
    };

    // If our saturation is too low we will not be able to see any objects.
    if sat < 0.03 {
        return None;
    }

    let mut hue = if f_r == max {
        (f_g - f_b) / (max - min)
    } else if f_g == max {
        2.0 + (f_b - f_r) / (max - min)
    } else {
        4.0 + (f_r - f_g) / (max - min)
    };
    hue /= 6.0;

    // Find two equidistant hues.
    let mut hue0 = hue + N1_3;
    if hue0 > 1.0 {
        hue0 -= 1.0;
    }
    let mut hue1 = hue0 + N1_3;
    if hue1 > 1.0 {
        hue1 -= 1.0;
    }

    let f2 = if lig < 0.5 {
        lig * (1.0 + sat)
    } else {
        (lig + sat) - (lig * sat)
    };
    let f1 = (2.0 * lig) - f2;

    let wrap = |v: f64| {
        let mut v = v;
        if v < 0.0 {
            v += 1.0;
        }
        if v > 1.0 {
            v -= 1.0;
        }
        v
    };
    let chan = |v: f64| {
        let c = if 6.0 * v < 1.0 {
            f1 + (f2 - f1) * 6.0 * v
        } else if 2.0 * v < 1.0 {
            f2
        } else if 3.0 * v < 2.0 {
            f1 + (f2 - f1) * (N2_3 - v) * 6.0
        } else {
            f1
        };
        (c * 255.0) as u8
    };

    let fore = rgb(
        chan(wrap((hue0 + 1.0) / 3.0)),
        chan(wrap(hue0)),
        chan(wrap((hue0 - 1.0) / 3.0)),
    );
    let border = rgb(
        chan(wrap((hue1 + 1.0) / 3.0)),
        chan(wrap(hue1)),
        chan(wrap((hue1 - 1.0) / 3.0)),
    );
    Some((fore, border))
}

/// `#%X%X%X%X%X%X` of six random nybbles, which is what upstream reaches for
/// when it cannot use the colour it was given.
fn random_color() -> Pixel {
    let nyb = || (random() % 16) as u8;
    rgb(nyb() * 16 + nyb(), nyb() * 16 + nyb(), nyb() * 16 + nyb())
}

impl State {
    fn build(&mut self, d: &mut Dpy) {
        self.width = d.width();
        self.height = d.height();
        self.back = Pixmap::new(self.width, self.height);

        self.density = (self.width as usize / 200) * d.res.int("density").max(0) as usize;
        self.flakes = (0..self.layers)
            .map(|_| {
                (0..self.density)
                    .map(|_| Flake {
                        x_pos: random_below(self.width) as f64,
                        y_pos: random_below(self.height) as f64,
                        angle: random_below(360) as f64 * (std::f64::consts::PI / 180.0),
                        ticks: random_below(360) as f64,
                        x_offset: random_below(self.height) as f64,
                        true_x: 0.0,
                    })
                    .collect()
            })
            .collect();
    }

    fn move_flakes(&mut self) {
        for (i, layer) in self.flakes.iter_mut().enumerate() {
            let depth = (i + 1) as f64;
            for f in layer.iter_mut() {
                f.ticks += 1.0;
                f.y_pos += self.falling_speed as f64 / 10.0 / depth;
                f.true_x = (f.x_offset
                    + f.ticks
                        * (std::f64::consts::PI / 180.0)
                        * (self.falling_speed as f64 / 10.0))
                    .sin()
                    * 10.0
                    + f.x_pos;
                f.angle += 0.005 * (self.falling_speed as f64 / 10.0);
                if f.y_pos - self.radius as f64 > self.height as f64 {
                    f.ticks = 0.0;
                    f.y_pos = -(self.radius as f64);
                }
            }
        }
    }

    /// One flake: the border arms first, then the thinner coloured arms over
    /// the top of them, so each arm gets an outline for free.
    fn draw_flake(&mut self, x_pos: i32, y_pos: i32, angle_offset: f64, layer: i32) {
        let radius = (self.radius - layer * 5) as f64;
        let diameter = (self.border_thickness * 2 + self.thickness) / layer;

        for pass in 0..2 {
            if pass == 0 {
                self.gc.set_line_width(diameter);
                self.gc.set_foreground(self.border_color);
            } else {
                self.gc.set_line_width(self.thickness / layer);
                self.gc.set_foreground(self.fore_color);
            }
            for i in 1..=self.arms {
                let angle =
                    ((2.0 * std::f64::consts::PI) / self.arms as f64) * i as f64 + angle_offset;
                let y = (angle.sin() * radius) as i32;
                let x = (angle.cos() * radius) as i32;
                self.back
                    .draw_line(&self.gc, x_pos, y_pos, x_pos + x, y_pos + y);
            }
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut back_color = parse_color(d.res.string("color")).unwrap_or_else(random_color);
    if d.res.bool("randomColors") {
        back_color = random_color();
    }
    // A background with no colour in it leaves nothing to derive, so upstream
    // keeps rolling until one has some.
    let (fore_color, border_color) = match color_helper(back_color) {
        Some(pair) => pair,
        None => {
            back_color = random_color();
            color_helper(back_color).unwrap_or((rgb(0xef, 0xbe, 0xa5), rgb(0xa5, 0xef, 0xbe)))
        }
    };

    let mut thickness = d.res.int("thickness").max(1);
    let mut border_thickness = d.res.int("bthickness").max(0);
    let mut radius = d.res.int("radius").max(1);
    let mut falling_speed = d.res.int("fallingspeed").max(1);
    if d.width() > 2560 || d.height() > 2560 {
        // Retina displays.
        thickness *= 2;
        border_thickness *= 2;
        radius *= 2;
        falling_speed *= 2;
    }

    let mut st = State {
        gc: Gc::new(fore_color, back_color),
        arms: d.res.int("arms").max(1),
        thickness,
        border_thickness,
        radius,
        border_color,
        fore_color,
        back_color,
        layers: d.res.int("layers").clamp(1, 32) as usize,
        density: 0,
        delay: d.res.int("delay").max(0) as u32,
        falling_speed,
        flakes: Vec::new(),
        back: Pixmap::new(1, 1),
        width: 0,
        height: 0,
    };
    st.gc.set_line_width(st.thickness);
    st.build(d);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.move_flakes();
        self.gc.set_foreground(self.back_color);
        let (w, h) = (self.width, self.height);
        self.back.fill_rectangle(&self.gc, 0, 0, w, h);

        // Furthest layer first, so the near ones land on top.
        for layer in (1..=self.layers).rev() {
            for j in 0..self.flakes[layer - 1].len() {
                let f = self.flakes[layer - 1][j];
                self.draw_flake(f.true_x as i32, f.y_pos as i32, f.angle, layer as i32);
            }
        }

        d.win().copy_area(&self.gc, &self.back, 0, 0, w, h, 0, 0);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        if self.width != width || self.height != height {
            self.build(d);
        }
    }
}

const DEFAULTS: &[&str] = &[
    "*color: #efbea5",
    "*arms: 5",
    "*thickness: 10",
    "*bthickness: 3",
    "*radius: 20",
    "*layers: 3",
    "*density: 5",
    "*fallingspeed: 10",
    "*delay: 10000",
    "*randomColors: False",
];

const COLORS: &[SelectItem] = &[
    SelectItem {
        value: "#efbea5",
        label: "Pink",
    },
    SelectItem {
        value: "#FF0000",
        label: "Red",
    },
    SelectItem {
        value: "#FFFF00",
        label: "Yellow",
    },
    SelectItem {
        value: "#00FF00",
        label: "Green",
    },
    SelectItem {
        value: "#00FFFF",
        label: "Cyan",
    },
    SelectItem {
        value: "#0000FF",
        label: "Blue",
    },
    SelectItem {
        value: "#FF00FF",
        label: "Magenta",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("fallingspeed", "Speed", 1.0, 50.0, 1.0, 0, "10"),
    Opt::slider("layers", "Layers", 1.0, 10.0, 1.0, 0, "3"),
    Opt::select("color", "Color", COLORS, "#efbea5"),
    Opt::boolean("randomColors", "Random colors", "False"),
    Opt::slider("arms", "Arms", 1.0, 10.0, 1.0, 0, "5"),
    Opt::slider("thickness", "Thickness", 1.0, 50.0, 1.0, 0, "10"),
    Opt::slider("bthickness", "Border thickness", 0.0, 50.0, 1.0, 0, "3"),
    Opt::slider("radius", "Radius", 1.0, 100.0, 1.0, 0, "20"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "fuzzyflakes",
    label: "Fuzzy Flakes",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Barry Dmytro",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=NrGe3xcqAns"),
        blurb: "Falling colored snowflake/flower shapes.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
