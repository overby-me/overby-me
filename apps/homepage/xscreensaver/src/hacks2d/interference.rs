//! Port of `hacks/interference.c`.
//!
//! ```text
//! interference.c --- colored fields via decaying sinusoidal waves.
//! An entry for the RHAD Labs Screensaver Contest.
//!
//! Author: Hannu Mallat <hmallat@cs.hut.fi>
//!
//! Copyright (C) 1998 Hannu Mallat.
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! decaying sinusoidal waves, which extend spherically from their
//! respective origins, move around the plane. a sort of interference
//! between them is calculated and the resulting twodimensional wave
//! height map is plotted in a grid, using softly changing colours.
//!
//! not physically (or in any sense) accurate, but fun to look at for
//! a while. you may tune the speed/resolution/interestingness tradeoff
//! with X resources, see below.
//!
//! Created      : Wed Apr 22 09:30:30 1998, hmallat
//! Last modified: Sun Aug 31 23:40:14 2003,
//!              david slimp <rock808@DavidSlimp.com>
//!              added -hue option to specify base color hue
//! Last modified: Wed May 15 00:04:43 2013,
//!              Dave Odell <dmo2118@gmail.com>
//!              Tuned performance; double-buffering is now off by default.
//!              Made animation speed independent of FPS.
//! Last modified: Fri Feb 21 02:14:29 2014, <dmo2118@gmail.com>
//!              Added support for SMP rendering.
//! Last modified: Tue Dec 30 16:43:33 2014, <dmo2118@gmail.com>
//!              Killed the black margin on the right and bottom.
//!              Reduced the default grid size to 2.
//! ```
//!
//! A handful of point sources each throw out a ring pattern that dies away with
//! distance, the rings are added together wherever they overlap, and the total
//! height at each point picks a colour out of a palette that loops. Where two
//! sources are close the sum beats into moire; where they separate it settles
//! back into rings. Nothing is drawn but a colour per cell.
//!
//! The interesting part is what is not computed. A ring's height at a distance
//! is a table lookup, not a cosine, and the distance itself is never square
//! rooted: squared distances index the table through a two-slope shift that is
//! fine near a source and coarse far from one, so a table of a couple of
//! thousand entries covers a radius of eight hundred pixels. Squared distance
//! along a row is not recomputed either, only stepped, since the second
//! difference of a square is constant.
//!
//! Upstream splits the rows across a thread pool and hands the result over as
//! one shared-memory image. Neither exists in a browser, so the rows are
//! computed in order and written into the framebuffer, which is what its
//! single-threaded path does.
//!
//! Two knobs here are not in the upstream XML: `gray` and `mono`, which are
//! upstream's own options and each of which is a whole palette that is otherwise
//! unreachable.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{BLACK, Pixel, WHITE, make_color_loop};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, frand};

/// The squared-distance table's two slopes, and where it changes from one to
/// the other. Eyeballed upstream, and the comments there record what each
/// setting costs: a coarser near slope makes the dot at a source visible, a
/// coarser far slope bands the rings.
const DISCARD_BITS1: u32 = 4;
const DISCARD_BITS2: u32 = 9;
const CUTOFF: i32 = 128 * 128;

/// Squared distance to a table index.
fn fast_table(x: i32) -> i32 {
    if x < CUTOFF {
        x >> DISCARD_BITS1
    } else {
        (x + ((CUTOFF << (DISCARD_BITS2 - DISCARD_BITS1)) - CUTOFF)) >> DISCARD_BITS2
    }
}

/// A table index back to the squared distance at the bottom of its bucket.
fn fast_inv_table(x: i32) -> f64 {
    if x < (CUTOFF >> DISCARD_BITS1) {
        (x << DISCARD_BITS1) as f64
    } else {
        (((x - (CUTOFF >> DISCARD_BITS1)) << DISCARD_BITS2) + CUTOFF) as f64
    }
}

#[derive(Clone, Copy, Default)]
struct Source {
    x: i32,
    y: i32,
    x_theta: f64,
    y_theta: f64,
}

struct Interference {
    delay: u32,
    count: usize,
    grid_size: i32,
    colors: usize,
    speed: i32,

    w: i32,
    h: i32,
    w_div_g: i32,
    h_div_g: i32,

    /// How many entries `wave_height` has, which is the squared wave extent run
    /// through the table mapping.
    radius: i32,
    last_frame: f64,
    wave_height: Vec<u32>,
    pal: Vec<Pixel>,
    sources: Vec<Source>,
    /// One row of summed wave heights, reused every row.
    result_row: Vec<u32>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let delay = d.res.int("delay").max(0) as u32;
    let count = d.res.int("count").max(1) as usize;
    let grid_size = d.res.int("gridsize").max(1);
    let mut mono = d.res.bool("mono");
    let mut colors = if mono {
        0
    } else {
        d.res.int("ncolors").max(2) as usize
    };

    let mut hue = d.res.int("hue") as f64;
    while !(0.0..360.0).contains(&hue) {
        hue = frand(360.0);
    }
    let speed = d.res.int("speed");
    let mut shift = d.res.float("color-shift") as i32 as f64;
    while shift >= 360.0 {
        shift -= 360.0;
    }
    while shift <= -360.0 {
        shift += 360.0;
    }
    let mut radius = d.res.int("radius").max(1);

    let (w, h) = (d.width(), d.height());
    // Retina displays.
    let scale = if w > 2560 || h > 2560 { 3.5 } else { 1.0 };
    radius = (radius as f64 * scale) as i32;

    let mut pal: Vec<Pixel> = Vec::new();
    if !mono {
        let (hh, ss, vv) = if d.res.bool("gray") {
            ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 0.5, 0.0])
        } else {
            let h1 = if hue + shift < 360.0 {
                hue + shift
            } else {
                hue + shift - 360.0
            };
            let h2 = if h1 + shift < 360.0 {
                h1 + shift
            } else {
                h1 + shift - 360.0
            };
            ([hue, h1, h2], [1.0, 1.0, 1.0], [1.0, 1.0, 1.0])
        };
        pal = make_color_loop(
            hh[0] as i32,
            ss[0],
            vv[0],
            hh[1] as i32,
            ss[1],
            vv[1],
            hh[2] as i32,
            ss[2],
            vv[2],
            colors,
        )
        .iter()
        .map(|c| c.pixel)
        .collect();
        if pal.len() < 2 {
            mono = true;
        }
    }
    // Deliberately not an `else`, as upstream has it: a palette that came back
    // too small falls through to here.
    if mono {
        colors = 2;
        pal = vec![BLACK, WHITE];
    }

    // The wave: full height at the source, dying away linearly to nothing at
    // the wave's extent, with a cosine ripple riding on it.
    let mut table_radius = fast_table(radius * radius).max(1);
    let mut wave_height = vec![0u32; table_radius as usize];
    for (i, wh) in wave_height.iter_mut().enumerate() {
        let fi = fast_inv_table(i as i32).sqrt();
        let max = colors as f64 * (radius as f64 - fi) / radius as f64;
        *wh = ((max + max * (fi / (50.0 * scale)).cos()) / 2.0) as u32;
    }
    table_radius = wave_height.len() as i32;

    let sources = (0..count)
        .map(|_| Source {
            x: 0,
            y: 0,
            x_theta: frand(2.0) * std::f64::consts::PI,
            y_theta: frand(2.0) * std::f64::consts::PI,
        })
        .collect();

    let w_div_g = (w + grid_size - 1) / grid_size;
    let h_div_g = (h + grid_size - 1) / grid_size;

    Box::new(Interference {
        delay,
        count,
        grid_size,
        colors,
        speed,
        w,
        h,
        w_div_g,
        h_div_g,
        radius: table_radius,
        last_frame: 0.0,
        wave_height,
        pal,
        sources,
        result_row: vec![0; w_div_g.max(1) as usize],
    })
}

impl Interference {
    /// Sum every source's wave over one row of cells, then paint the row.
    fn render_row(&mut self, d: &mut Dpy, j: i32) {
        let g = self.grid_size;
        let g2 = 2 * g * g;
        let px = g / 2;
        let py = j * g + px;

        self.result_row.fill(0);
        for k in 0..self.count {
            let dx = px - self.sources[k].x;
            let dy = py - self.sources[k].y;

            // The squared distance is stepped rather than recomputed: the
            // second difference of a square is constant.
            let mut dist0 = dx * dx + dy * dy;
            let ddist = -2 * g * self.sources[k].x;
            let mut px2g = g2;

            for i in 0..self.w_div_g as usize {
                let dist1 = fast_table(dist0);
                if dist1 < self.radius {
                    self.result_row[i] += self.wave_height[dist1 as usize];
                }
                dist0 += px2g + ddist;
                px2g += g2;
            }
        }

        // A subtraction or two before the modulus is slightly faster, as
        // upstream notes.
        let colors = self.colors as u32;
        let width = self.w;
        let pixels = d.win().pixels_mut();
        for i in 0..self.w_div_g {
            let mut result = self.result_row[i as usize];
            if result >= colors {
                result -= colors;
                if result >= colors {
                    result %= colors;
                }
            }
            let p = self.pal[result as usize];

            for row in 0..g {
                let y = j * g + row;
                if y >= self.h {
                    break;
                }
                let base = (y * width) as usize;
                for col in 0..g {
                    let x = i * g + col;
                    if x >= width {
                        break;
                    }
                    pixels[base + x as usize] = p;
                }
            }
        }
    }
}

impl Screenhack for Interference {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let now = d.time;
        let elapsed = (now - self.last_frame) * 10.0;
        self.last_frame = now;

        let tau = std::f64::consts::TAU;
        for k in 0..self.count {
            let s = &mut self.sources[k];
            s.x_theta += elapsed * self.speed as f64 / 1000.0;
            if s.x_theta > tau {
                s.x_theta -= tau;
            }
            s.y_theta += elapsed * self.speed as f64 / 1000.0;
            if s.y_theta > tau {
                s.y_theta -= tau;
            }
            s.x = self.w / 2 + (s.x_theta.cos() * (self.w as f64 / 2.0)) as i32;
            s.y = self.h / 2 + (s.y_theta.cos() * (self.h as f64 / 2.0)) as i32;
        }

        for j in 0..self.h_div_g {
            self.render_row(d, j);
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.w_div_g = (w + self.grid_size - 1) / self.grid_size;
        self.h_div_g = (h + self.grid_size - 1) / self.grid_size;
        self.result_row = vec![0; self.w_div_g.max(1) as usize];
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*count: 3",
    "*gridsize: 2",
    "*ncolors: 192",
    "*hue: 0",
    "*speed: 30",
    "*delay: 30000",
    "*color-shift: 60",
    "*radius: 800",
    "*gray: false",
    "*mono: false",
    "*doubleBuffer: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Wave speed", 1.0, 100.0, 1.0, 0, "30"),
    Opt::slider("radius", "Wave size", 50.0, 1500.0, 10.0, 0, "800"),
    Opt::slider("count", "Number of waves", 0.0, 20.0, 1.0, 0, "3"),
    Opt::slider("gridsize", "Magnification", 1.0, 20.0, 1.0, 0, "2"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "192"),
    Opt::slider("color-shift", "Color contrast", 0.0, 100.0, 1.0, 0, "60"),
    Opt::slider("hue", "Hue", 0.0, 360.0, 1.0, 0, "0"),
    Opt::boolean("gray", "Shades of grey", "false"),
    Opt::boolean("mono", "Black and white only", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "interference",
    label: "Interference",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Hannu Mallat",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=nEmvx4l1sHI"),
        blurb: "Decaying sinusoidal waves make colors.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
