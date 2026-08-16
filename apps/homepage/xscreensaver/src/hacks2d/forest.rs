//! Port of `hacks/forest.c`.
//!
//! ```text
//! forest.c (aka xtree.c), Copyright (c) 1999
//!  Peter Baumung <unn6@rz.uni-karlsruhe.de>
//!
//! Most code taken from
//!  xscreensaver, Copyright (c) 1992, 1995, 1997
//!  Jamie Zawinski <jwz@netscape.com>
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
//! Fractal trees, planted back to front. Each frame grows one tree from a
//! recursively branching trunk, and every branch is a fan of thin lines shaded
//! from bark to highlight, so the trunks look round. The twigs sprout blobs of
//! leaf colour picked from a seasonal palette, so a whole forest comes up in
//! autumn orange or high-summer green.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::fb::FULL_CIRCLE;
use crate::runtime::xlockmore::nrand;
use crate::runtime::{About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XArc, XColor};

/// Upstream's `static XColor colors[20]`: the palette is a fixed-size array and
/// the hack clamps `ncolors` to fit it.
const MAX_COLORS: usize = 20;

/// The twelve leaf hues, one per season step.
const COLOR_M: [i32; 12] = [
    0xff0000, 0xff8000, 0xffff00, 0x80ff00, 0x00ff00, 0x00ff80, 0x00ffff, 0x0080ff, 0x0000ff,
    0x8000ff, 0xff00ff, 0xff0080,
];

/// How far each of the four shades within a season steps away from its hue.
const COLOR_V: [i32; 12] = [
    0x0a0000, 0x0a0500, 0x0a0a00, 0x050a00, 0x000a00, 0x000a05, 0x000a0a, 0x00050a, 0x00000a,
    0x05000a, 0x0a000a, 0x0a0005,
];

/// `rRand(a, b)`.
fn rrand(a: f64, b: f64) -> f64 {
    a + (b - a) * nrand(10001) as f64 / 10000.0
}

struct Forest {
    gc: Gc,
    delay: u32,
    npixels: usize,
    colors: [XColor; MAX_COLORS],
    /// Upstream's `color` global: how many colours were allocated. On a
    /// TrueColor canvas `XAllocColor` never fails, so it is always `npixels`.
    color: usize,
    thick: i32,
    size: f64,
    /// Index of this tree's block of four leaf shades.
    tree_color: usize,
    /// Trees left to plant before the range pauses.
    to_do: i32,
    pause: i32,
    season: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // xlockmore's own clamping, then forest's: it refuses more than it has
    // room for.
    let mut npixels = d.res.int("ncolors");
    if npixels <= 0 {
        npixels = 64;
    }
    if d.mono_p {
        npixels = 2;
    }
    let npixels = npixels.min(MAX_COLORS as i32) as usize;

    let mut st = Forest {
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        delay: d.res.int("delay").max(0) as u32,
        npixels,
        colors: [XColor::default(); MAX_COLORS],
        color: 0,
        thick: 0,
        size: 1.0,
        tree_color: 0,
        to_do: 0,
        pause: 0,
        season: 0,
    };
    st.restart(d);
    Box::new(st)
}

impl Forest {
    /// The trunk shades, in the first four slots.
    ///
    /// Upstream builds these only on the very first init, but the seasonal loop
    /// overwrites everything from slot four up on every init, so filling the
    /// whole table each time lands on the same palette.
    fn base_colors(&mut self) {
        for i in 0..self.npixels {
            let m = (i % 4) as u16;
            self.colors[i] = if self.npixels < 4 {
                let v = 65535 * (i as u16 & 1);
                XColor::from_rgb16(v, v, v)
            } else if self.npixels < 8 {
                let v = 32768 + 4096 * m;
                XColor::from_rgb16(v, v, v)
            } else {
                XColor::from_rgb16(24576 + 4096 * m, 10240 + 2048 * m, 0)
            };
        }
    }

    /// `init_trees`: a new season, a new stand of trees.
    fn restart(&mut self, d: &mut Dpy) {
        d.clear_window();
        self.gc.set_line_width(2);

        self.to_do = 25;
        self.season = nrand(12);
        self.base_colors();

        // Four shades per season, walking forward through the year as the
        // palette runs out.
        for i in 4..self.npixels {
            let s = ((self.season + ((i - 4) / 4) as i32) % 12) as usize;
            let c = COLOR_M[s] - 2 * COLOR_V[s] * (i % 4) as i32;
            self.colors[i] = XColor::from_rgb16(
                ((c & 0xff0000) / 256) as u16,
                (c & 0x00ff00) as u16,
                ((c & 0x0000ff) * 256) as u16,
            );
        }

        self.color = self.npixels;
        let p = self.colors[1].pixel;
        self.gc.set_foreground(p);
    }

    /// One branch: a fan of `widths` lines from a `widths`-wide start to a
    /// `widthe`-wide end, shaded across the trunk colours.
    fn draw_branch(
        &mut self,
        d: &mut Dpy,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        angle: f64,
        widths: i32,
        widthe: i32,
    ) {
        let across = angle + std::f64::consts::FRAC_PI_2;
        let sns = 0.5 * widths as f64 * across.sin();
        let css = 0.5 * widths as f64 * across.cos();
        let sne = 0.5 * widthe as f64 * across.sin();
        let cse = 0.5 * widthe as f64 * across.cos();

        let xs1 = (x1 as f64 - sns) as i32;
        let xs2 = (x1 as f64 + sns) as i32;
        let ys1 = (y1 as f64 - css) as i32;
        let ys2 = (y1 as f64 + css) as i32;
        let xe1 = (x2 as f64 - sne) as i32;
        let xe2 = (x2 as f64 + sne) as i32;
        let ye1 = (y2 as f64 - cse) as i32;
        let ye2 = (y2 as f64 + cse) as i32;

        for i in 0..widths {
            if self.color >= 4 {
                let p = self.colors[(i * 4 / widths) as usize].pixel;
                self.gc.set_foreground(p);
            }
            d.win().draw_line(
                &self.gc,
                xs1 + (xs2 - xs1) * i / widths,
                ys1 + (ys2 - ys1) * i / widths,
                xe1 + (xe2 - xe1) * i / widths,
                ye1 + (ye2 - ye1) * i / widths,
            );
        }
    }

    fn draw_tree_rec(&mut self, d: &mut Dpy, thick: f64, x: i32, y: i32, angle: f64) {
        let length = ((24 + nrand(12)) as f64 * self.size) as i32;
        let a = (x as f64 - length as f64 * angle.sin()) as i32;
        let b = (y as f64 - length as f64 * angle.cos()) as i32;

        self.draw_branch(
            d,
            x,
            y,
            a,
            b,
            angle,
            (thick * self.size) as i32,
            (0.68 * thick * self.size) as i32,
        );

        if thick > 2.0 {
            self.draw_tree_rec(d, 0.68 * thick, a, b, 0.8 * angle + rrand(-0.2, 0.2));
            if thick < self.thick as f64 - 1.0 {
                self.draw_tree_rec(d, 0.68 * thick, a, b, angle + rrand(0.2, 0.9));
                self.draw_tree_rec(
                    d,
                    0.68 * thick,
                    (a + x) / 2,
                    (b + y) / 2,
                    angle - rrand(0.2, 0.9),
                );
            }
        }

        // Anything thin enough to be a twig carries leaves.
        if thick < 0.5 * self.thick as f64 {
            let nleaf = 12 + nrand(4);
            let mut leaf = Vec::with_capacity(nleaf as usize);
            for _ in 0..nleaf {
                let lx = a + (self.size * rrand(-12.0, 12.0)) as i32;
                let ly = b + (self.size * rrand(-12.0, 12.0)) as i32;
                let w = (self.size * rrand(2.0, 6.0)) as i32;
                leaf.push(XArc {
                    x: lx,
                    y: ly,
                    width: w,
                    height: w,
                    angle1: 0,
                    angle2: FULL_CIRCLE,
                });
            }
            if self.npixels >= 4 {
                let p = self.colors[self.tree_color + nrand(4) as usize].pixel;
                self.gc.set_foreground(p);
            }
            d.win().fill_arcs(&self.gc, &leaf);
        }
    }
}

impl Screenhack for Forest {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let width = d.width();
        let height = d.height();

        if self.pause == 1 {
            self.pause -= 1;
            self.restart(d);
        } else if self.pause > 1 {
            self.pause -= 1;
            return self.delay;
        } else {
            self.to_do -= 1;
            if self.to_do == 0 {
                self.pause = 6;
                return self.delay;
            }
        }

        // Trees planted from the back of the stand forward, so the near ones
        // paint over the far ones.
        let x = nrand(width);
        let y = (1.25 * height as f64 * (1.0 - self.to_do as f64 / 23.0)) as i32;
        self.thick = rrand(7.0, 12.0) as i32;
        self.size = height as f64 / 480.0;
        self.tree_color = if self.color < 8 {
            0
        } else {
            4 * (1 + nrand(self.color as i32 / 4 - 1) as usize)
        };

        let thick = self.thick as f64;
        self.draw_tree_rec(d, thick, x, y, rrand(-0.1, 0.1));
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, _width: i32, _height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.restart(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 500000",
    "*ncolors: 20",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider(
        "delay",
        "Frame rate",
        0.0,
        3_000_000.0,
        50000.0,
        0,
        "500000",
    )
    .inverted(),
    Opt::slider("ncolors", "Number of colors", 1.0, 20.0, 1.0, 0, "20"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "forest",
    label: "Forest",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Peter Baumung",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=EEK2qbAmKWs"),
        blurb: "Fractal trees, planted back to front.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
