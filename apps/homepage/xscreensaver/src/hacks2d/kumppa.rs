//! Port of `hacks/kumppa.c`.
//!
//! ```text
//! Copyright (C) Teemu Suutari (temisu@utu.fi) Feb 1998
//!
//! Permission is hereby granted, free of charge, to any person obtaining
//! a copy of this software and associated documentation files (the
//! "Software"), to deal in the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to
//! permit persons to whom the Software is furnished to do so, subject to
//! the following conditions:
//!
//! The above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
//! IN NO EVENT SHALL THE X CONSORTIUM BE LIABLE FOR ANY CLAIM, DAMAGES OR
//! OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
//! ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
//! OTHER DEALINGS IN THE SOFTWARE.
//!
//! Except as contained in this notice, the name of the X Consortium shall
//! not be used in advertising or otherwise to promote the sale, use or
//! other dealings in this Software without prior written authorization
//! from the X Consortium.
//!
//! *** This is contest-version. Don't look any further, code is *very* ugly.
//! ```
//!
//! Everything on screen spirals outwards from the middle. The whole picture is
//! turned a fraction of a degree each frame, not by transforming anything but
//! by copying a few hundred overlapping blocks one pixel across and one pixel
//! down from where they were, which shears the image around the centre. The
//! blocks are laid out so no seam ever falls in the same place twice, and the
//! table that decides where they go is built by repeatedly picking whichever
//! remaining column sits furthest from the ones already taken.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XColor, frand};

/// The thirty-two colours, walking the hue circle.
const COLORS: [(u8, u8, u8); 32] = [
    (0, 0, 255),
    (0, 51, 255),
    (0, 102, 255),
    (0, 153, 255),
    (0, 204, 255),
    (0, 255, 255),
    (0, 255, 204),
    (0, 255, 153),
    (0, 255, 102),
    (0, 255, 51),
    (0, 255, 0),
    (51, 255, 0),
    (102, 255, 0),
    (153, 255, 0),
    (204, 255, 0),
    (255, 255, 0),
    (255, 204, 0),
    (255, 153, 0),
    (255, 102, 0),
    (255, 51, 0),
    (255, 0, 0),
    (255, 0, 51),
    (255, 0, 102),
    (255, 0, 153),
    (255, 0, 204),
    (255, 0, 255),
    (219, 0, 255),
    (182, 0, 255),
    (146, 0, 255),
    (109, 0, 255),
    (73, 0, 255),
    (37, 0, 255),
];

/// Rates and amplitudes for the eight sine sums the pen follows.
const COSINUS: [[f64; 6]; 8] = [
    [-0.07, 0.12, -0.06, 32.0, 25.0, 37.0],
    [0.08, -0.03, 0.05, 51.0, 46.0, 32.0],
    [0.12, 0.07, -0.13, 27.0, 45.0, 36.0],
    [0.05, -0.04, -0.07, 36.0, 27.0, 39.0],
    [-0.02, -0.07, 0.1, 21.0, 43.0, 42.0],
    [-0.11, 0.06, 0.02, 51.0, 25.0, 34.0],
    [0.04, -0.15, 0.02, 42.0, 32.0, 25.0],
    [-0.02, -0.04, -0.13, 34.0, 20.0, 15.0],
];

fn satnum(maxi: i32) -> i32 {
    (maxi as f64 * frand(1.0)) as i32
}

struct Kumppa {
    acosinus: [[f64; 3]; 8],
    coords: [i32; 8],
    ocoords: [i32; 8],
    /// Thirty-two pen colours plus, at the end, the background.
    fgc: Vec<Gc>,
    cgc: Gc,
    sizx: i32,
    sizy: i32,
    midx: i32,
    midy: i32,
    delay: u32,
    /// True to draw the wandering pen, false to spatter dots instead.
    cosilines: bool,
    xrotations: Vec<i32>,
    yrotations: Vec<i32>,
    xrottable: Vec<i32>,
    yrottable: Vec<i32>,
    rotate_x: Vec<i32>,
    rotate_y: Vec<i32>,
    rotsize_x: usize,
    rotsize_y: usize,
    state_x: usize,
    state_y: usize,
    rx: i32,
    ry: i32,
    draw_count: i32,
    pscale: i32,
}

/// Build one axis's rotation table: for each group, the columns whose seams
/// that group will use, chosen so each new column sits as far as possible from
/// the ones already taken.
fn make_axis(mid: i32, rotsize: usize) -> (Vec<i32>, Vec<i32>, i32) {
    let mid = mid.max(1);
    let n = mid as usize;
    let mut rotations = vec![0i32; n + 2];
    let mut rottable = vec![0i32; rotsize + 1];
    let mut chks = vec![true; n];
    let inc = (mid + 1) as f64 / rotsize as f64;

    let mut maxi = 0;
    let mut c = 0i32;
    let mut d = 0.0f64;
    let mut g = 0i32;

    for slot in rottable.iter_mut().take(rotsize) {
        *slot = c;
        let start = c;
        // How many seams this group gets.
        let mut f = (d + inc) as i32 - g;
        g += f;
        if g > mid {
            f -= g - mid;
            g = mid;
        }

        for _ in 0..f {
            // Score every free column by how crowded its neighbourhood is,
            // weighting nearer neighbours more, and take the emptiest.
            let mut m = 0.0f64;
            let mut k = 0usize;
            for j in 0..n {
                if !chks[j] {
                    continue;
                }
                let mut om = 0.0f64;
                let mut ok = 1.0f64;
                let mut l = 0usize;
                while j + l < n && om + 12.0 * ok > m {
                    if j >= l {
                        if chks[j - l] {
                            om += ok;
                        }
                    } else if chks[l - j] {
                        om += ok;
                    }
                    if chks[j + l] {
                        om += ok;
                    }
                    ok /= 1.5;
                    l += 1;
                }
                if om >= m {
                    k = j;
                    m = om;
                }
            }
            chks[k] = false;

            // Insert it into this group's run, which is kept sorted.
            let mut l = c;
            while l >= start {
                if l != start {
                    rotations[l as usize] = rotations[(l - 1) as usize];
                }
                if k as i32 > rotations[l as usize] || l == start {
                    rotations[l as usize] = k as i32;
                    c += 1;
                    l = start;
                }
                l -= 1;
            }
        }

        d += inc;
        if maxi < c - start {
            maxi = c - start;
        }
    }
    rottable[rotsize] = c;
    (rotations, rottable, maxi)
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut pscale = 1;
    if d.width() > 2560 || d.height() > 2560 {
        pscale *= 3; // Retina displays
    }

    let bg = d.res.pixel("background");
    let mut fgc: Vec<Gc> = COLORS
        .iter()
        .map(|(r, g, b)| {
            let mut gc = Gc::new(
                XColor::from_rgb16(*r as u16 * 256, *g as u16 * 256, *b as u16 * 256).pixel,
                bg,
            );
            gc.set_line_width(pscale);
            gc
        })
        .collect();
    let mut back = Gc::new(bg, bg);
    back.set_line_width(pscale);
    fgc.push(back);

    let mut cgc = Gc::new(bg, bg);
    cgc.set_line_width(pscale);

    let mut speed = d.res.float("speed");
    if !(0.0001..=0.2).contains(&speed) {
        // Upstream complains and falls back rather than trusting the value.
        speed = 0.1;
    }

    let (sizx, sizy) = (d.width(), d.height());
    let (midx, midy) = (sizx >> 1, sizy >> 1);
    let rotsize_x = (2.0 / speed + 1.0) as usize;
    let rotsize_y = rotsize_x;

    let (xrotations, xrottable, maxi_x) = make_axis(midx, rotsize_x);
    let (yrotations, yrottable, maxi_y) = make_axis(midy, rotsize_y);

    Box::new(Kumppa {
        acosinus: [[0.0; 3]; 8],
        coords: [0; 8],
        ocoords: [0; 8],
        fgc,
        cgc,
        sizx,
        sizy,
        midx,
        midy,
        delay: d.res.int("delay").max(0) as u32,
        cosilines: d.res.bool("random"),
        xrotations,
        yrotations,
        xrottable,
        yrottable,
        rotate_x: vec![0; (maxi_x as usize + 2) * 2],
        rotate_y: vec![0; (maxi_y as usize + 2) * 2],
        rotsize_x,
        rotsize_y,
        state_x: 0,
        state_y: 0,
        rx: 0,
        ry: 0,
        draw_count: 0,
        pscale,
    })
}

impl Kumppa {
    /// Copy one block a pixel across and a pixel down from where it was,
    /// clipped to the window.
    fn pala_rotate(&self, d: &mut Dpy, x: i32, y: i32) {
        let (x, y) = (x as usize, y as usize);
        let mut ax = self.rotate_x[x];
        let mut ay = self.rotate_y[y];
        let mut bx = self.rotate_x[x + 1] + 2;
        let mut by = self.rotate_y[y + 1] + 2;
        let mut cx = self.rotate_x[x] - (y as i32 - self.ry) + x as i32 - self.rx;
        let mut cy = self.rotate_y[y] + (x as i32 - self.rx) + y as i32 - self.ry;

        if cx < 0 {
            ax -= cx;
            cx = 0;
        }
        if cy < 0 {
            ay -= cy;
            cy = 0;
        }
        if cx + bx - ax > self.sizx {
            bx = ax - cx + self.sizx;
        }
        if cy + by - ay > self.sizy {
            by = ay - cy + self.sizy;
        }
        if ax < bx && ay < by {
            d.win()
                .copy_area_self(&self.cgc, ax, ay, bx - ax, by - ay, cx, cy);
        }
    }

    fn rotate(&mut self, d: &mut Dpy) {
        self.rx = self.xrottable[self.state_x + 1] - self.xrottable[self.state_x];
        self.ry = self.yrottable[self.state_y + 1] - self.yrottable[self.state_y];

        // The seams for this frame, working outwards from the middle in both
        // directions and pinned to the window edges at the ends.
        for x in 0..=self.rx {
            self.rotate_x[x as usize] = if x != 0 {
                let at = self.xrottable[self.state_x + 1] - x;
                self.midx - 1 - self.xrotations[at as usize]
            } else {
                0
            };
        }
        for x in 0..=self.rx {
            self.rotate_x[(x + self.rx + 1) as usize] = if x == self.rx {
                self.sizx - 1
            } else {
                let at = self.xrottable[self.state_x] + x;
                self.midx + self.xrotations[at as usize]
            };
        }
        for y in 0..=self.ry {
            self.rotate_y[y as usize] = if y != 0 {
                let at = self.yrottable[self.state_y + 1] - y;
                self.midy - 1 - self.yrotations[at as usize]
            } else {
                0
            };
        }
        for y in 0..=self.ry {
            self.rotate_y[(y + self.ry + 1) as usize] = if y == self.ry {
                self.sizy - 1
            } else {
                let at = self.yrottable[self.state_y] + y;
                self.midy + self.yrotations[at as usize]
            };
        }

        let big = self.rx.max(self.ry);
        for dy in 0..(big + 1) * 2 {
            for dx in 0..(big + 1) * 2 {
                let y = if self.rx > self.ry {
                    self.ry - self.rx
                } else {
                    0
                };
                if dy + y >= 0
                    && dy < (self.ry + 1) * 2
                    && dx < (self.rx + 1) * 2
                    && dy + y + dx <= self.ry + self.rx
                    && dy + y - dx <= self.ry - self.rx
                {
                    self.pala_rotate(d, (self.rx << 1) + 1 - dx, dy + y);
                    self.pala_rotate(d, dx, (self.ry << 1) + 1 - dy - y);
                }

                let y = if self.ry > self.rx {
                    self.rx - self.ry
                } else {
                    0
                };
                if dy + y >= 0
                    && dx < (self.ry + 1) * 2
                    && dy < (self.rx + 1) * 2
                    && dy + y + dx <= self.ry + self.rx
                    && dx - dy - y >= self.ry - self.rx
                {
                    self.pala_rotate(d, dy + y, dx);
                    self.pala_rotate(d, (self.rx << 1) + 1 - dy - y, (self.ry << 1) + 1 - dx);
                }
            }
        }

        self.state_x += 1;
        if self.state_x == self.rotsize_x {
            self.state_x = 0;
        }
        self.state_y += 1;
        if self.state_y == self.rotsize_y {
            self.state_y = 0;
        }
    }
}

impl Screenhack for Kumppa {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.cosilines {
            self.draw_count += 1;
            let pairs = self.acosinus.iter_mut().zip(COSINUS.iter());
            for ((ac, co), coord) in pairs.zip(self.coords.iter_mut()) {
                let mut f = 0.0;
                for b in 0..3 {
                    ac[b] += co[b];
                    f += co[b + 3] * ac[b].sin();
                }
                *coord = f as i32;
            }
            // Four pens, each following two of the eight sine sums.
            for a in 0..4 {
                let gc = &self.fgc[(((a << 2) + self.draw_count as usize) & 31) % self.fgc.len()];
                let (midx, midy) = (self.midx, self.midy);
                d.win().draw_line(
                    gc,
                    midx + self.ocoords[a << 1],
                    midy + self.ocoords[(a << 1) + 1],
                    midx + self.coords[a << 1],
                    midy + self.coords[(a << 1) + 1],
                );
                self.ocoords[a << 1] = self.coords[a << 1];
                self.ocoords[(a << 1) + 1] = self.coords[(a << 1) + 1];
            }
        } else {
            for _ in 0..8 {
                let mut a = satnum(50);
                if a >= 32 {
                    a = 32;
                }
                let b = satnum(32) - 16 + self.midx;
                self.draw_count = satnum(32) - 16 + self.midy;
                let (gc, y, s) = (&self.fgc[a as usize], self.draw_count, 2 * self.pscale);
                d.win().fill_rectangle(gc, b, y, s, s);
            }
        }

        // A hole punched in the middle, which is what everything spirals out
        // of.
        let (midx, midy, s) = (self.midx, self.midy, 4 * self.pscale);
        d.win()
            .fill_rectangle(&self.fgc[32], midx - 2, midy - 2, s, s);

        self.rotate(d);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        // Upstream keeps the tables it built for the old size, so the seams
        // stay where they were; only the extents move.
        self.sizx = width;
        self.sizy = height;
        self.midx = self.sizx >> 1;
        self.midy = self.sizy >> 1;
        self.state_x = 0;
        self.state_y = 0;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*speed: 0.1",
    "*delay: 10000",
    "*random: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("speed", "Density", 0.0001, 0.2, 0.005, 4, "0.1"),
    Opt::boolean("random", "Randomize", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "kumppa",
    label: "Kumppa",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Teemu Suutari",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=64ULSfxhkDY"),
        blurb: "Spiraling, spinning splashes of colour rush toward the screen.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
