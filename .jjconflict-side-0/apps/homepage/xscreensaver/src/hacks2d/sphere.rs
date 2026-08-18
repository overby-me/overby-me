//! Port of `hacks/sphere.c`.
//!
//! ```text
//! Copyright (c) 1988 by Sun Microsystems
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! 01-Nov-2000: Allocation checks
//! 30-May-1997: <jwz@jwz.org> made it go vertically as well as horizontally.
//! 27-May-1997: <jwz@jwz.org> turned into a standalone program.
//! 02-Sep-1993: xlock version David Bagley <bagleyd@tux.org>
//! 1988: Revised to use SunView canvas instead of gfxsw Sun Microsystems
//! 1982: Orignal Algorithm Tom Duff Lucasfilm Ltd.
//! ```
//!
//! Shaded spheres, drawn one scanline at a time. The shade at a point is the
//! dot product of the surface normal with a fixed light vector, and rather than
//! dimming the colour, the hack dithers: a pixel is lit with probability equal
//! to its brightness. A leading black line sweeps ahead of the drawing edge, so
//! each sphere wipes the one before it away as it goes.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

/// The light source vector, length 100.
const NX: i32 = 48;
const NY: i32 = -36;
const NZ: i32 = 80;
const NR: i32 = 100;

/// `SQRT(a)`: an integer square root by way of the double one.
fn isqrt(a: i32) -> i32 {
    (a as f64).sqrt() as i32
}

struct Sphere {
    mi: ModeInfo,
    width: i32,
    height: i32,
    radius: i32,
    /// Centre of the sphere being drawn.
    x0: i32,
    y0: i32,
    color: usize,
    /// The drawing edge, relative to the centre.
    x: i32,
    y: i32,
    /// Which way the edge sweeps. Exactly one of these is non-zero.
    dirx: i32,
    diry: i32,
    /// Which way the light is coming from, per axis.
    shadowx: i32,
    shadowy: i32,
    maxx: i32,
    maxy: i32,
    points: Vec<XPoint>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // BRIGHT_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Bright);
    let mut st = Sphere {
        mi,
        width: 0,
        height: 0,
        radius: 0,
        x0: 0,
        y0: 0,
        color: 0,
        x: 0,
        y: 0,
        dirx: 0,
        diry: 0,
        shadowx: 0,
        shadowy: 0,
        maxx: 0,
        maxy: 0,
        points: Vec::new(),
    };
    st.restart(d);
    Box::new(st)
}

impl Sphere {
    fn restart(&mut self, d: &mut Dpy) {
        self.width = d.width().max(4);
        self.height = d.height().max(4);
        self.points = Vec::with_capacity(self.width.min(self.height) as usize);

        d.clear_window();

        self.dirx = 1;
        self.x = self.radius;
        self.shadowx = if lrand() & 1 == 1 { 1 } else { -1 };
        self.shadowy = if lrand() & 1 == 1 { 1 } else { -1 };
    }
}

impl Screenhack for Sphere {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (mut minx, mut maxx, mut miny, mut maxy) = (0, 0, 0, 0);

        // The edge has run off the far side of the sphere: start another one.
        if (self.dirx != 0 && self.x.abs() >= self.radius)
            || (self.diry != 0 && self.y.abs() >= self.radius)
        {
            self.radius = nrand((self.width / 2).min(self.height / 2) - 1) + 1;

            if lrand() & 1 == 1 {
                self.dirx = (lrand() & 1) as i32 * 2 - 1;
                self.diry = 0;
            } else {
                self.dirx = 0;
                self.diry = (lrand() & 1) as i32 * 2 - 1;
            }
            self.x0 = nrand(self.width);
            self.y0 = nrand(self.height);

            self.x = -self.radius * self.dirx;
            self.y = -self.radius * self.diry;

            if self.mi.npixels() > 2 {
                self.color = nrand(self.mi.npixels()) as usize;
            }
        }

        // Clip the edge to the window.
        if self.dirx == 1 {
            if self.x0 + self.x < 0 {
                self.x = -self.x0;
            }
        } else if self.dirx == -1 && self.x0 + self.x >= self.width {
            self.x = self.width - self.x0 - 1;
        }
        if self.diry == 1 {
            if self.y0 + self.y < 0 {
                self.y = -self.y0;
            }
        } else if self.diry == -1 && self.y0 + self.y >= self.height {
            self.y = self.height - self.y0 - 1;
        }

        if self.dirx != 0 {
            self.maxy = isqrt(self.radius * self.radius - self.x * self.x);
            miny = -self.maxy;
            if self.y0 - self.maxy < 0 {
                miny = -self.y0;
            }
            maxy = self.maxy;
        }
        if self.diry != 0 {
            self.maxx = isqrt(self.radius * self.radius - self.y * self.y);
            minx = -self.maxx;
            if self.x0 - self.maxx < 0 {
                minx = -self.x0;
            }
            maxx = self.maxx;
        }
        if self.dirx != 0 && self.y0 + self.maxy >= self.height {
            maxy = self.height - self.y0;
        }
        if self.diry != 0 && self.x0 + self.maxx >= self.width {
            maxx = self.width - self.x0;
        }

        // The black line one step ahead of the shading, which is what erases
        // whatever was on screen before.
        let black = self.mi.black;
        self.mi.gc.set_foreground(black);
        if self.dirx != 0 {
            let (x, y0, x0) = (self.x, self.y0, self.x0);
            d.win()
                .draw_line(&self.mi.gc, x0 + x, y0 + miny, x0 + x, y0 + maxy);
        }
        if self.diry != 0 {
            let (y, y0, x0) = (self.y, self.y0, self.x0);
            d.win()
                .draw_line(&self.mi.gc, x0 + minx, y0 + y, x0 + maxx, y0 + y);
        }

        let color = if self.mi.npixels() > 2 {
            self.mi.pixel(self.color)
        } else {
            self.mi.white
        };
        self.mi.gc.set_foreground(color);

        self.points.clear();
        if self.dirx != 0 {
            let sqrd = self.radius * self.radius - self.x * self.x;
            let nd = NX * self.shadowx * self.x;
            self.y = miny;
            while self.y <= maxy {
                if nrand(self.radius * NR)
                    <= nd + NY * self.shadowy * self.y + NZ * isqrt(sqrd - self.y * self.y)
                {
                    self.points.push(XPoint {
                        x: self.x + self.x0,
                        y: self.y + self.y0,
                    });
                }
                self.y += 1;
            }
        }
        if self.diry != 0 {
            let sqrd = self.radius * self.radius - self.y * self.y;
            let nd = NY * self.shadowy * self.y;
            self.x = minx;
            while self.x <= maxx {
                if nrand(self.radius * NR)
                    <= NX * self.shadowx * self.x + nd + NZ * isqrt(sqrd - self.x * self.x)
                {
                    self.points.push(XPoint {
                        x: self.x + self.x0,
                        y: self.y + self.y0,
                    });
                }
                self.x += 1;
            }
        }
        d.win().draw_points(&self.mi.gc, &self.points);

        // Step the edge, wrapping to the far side when it leaves the window.
        if self.dirx == 1 {
            self.x += 1;
            if self.x0 + self.x >= self.width {
                self.x = self.radius;
            }
        } else if self.dirx == -1 {
            self.x -= 1;
            if self.x0 + self.x < 0 {
                self.x = -self.radius;
            }
        }
        if self.diry == 1 {
            self.y += 1;
            if self.y0 + self.y >= self.height {
                self.y = self.radius;
            }
        } else if self.diry == -1 {
            self.y -= 1;
            if self.y0 + self.y < 0 {
                self.y = -self.radius;
            }
        }

        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.restart(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 20000",
    "*cycles: 20",
    "*size: 0",
    "*ncolors: 64",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "sphere",
    label: "Sphere",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tom Duff and Jamie Zawinski",
        year: "1982",
        video: Some("https://www.youtube.com/watch?v=FswhxIVXdt8"),
        blurb: "Dither-shaded spheres, drawn one scanline at a time.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
