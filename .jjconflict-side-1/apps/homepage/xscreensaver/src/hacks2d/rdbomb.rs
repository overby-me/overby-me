//! Port of `hacks/rdbomb.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1992-2014 Jamie Zawinski <jwz@jwz.org>
//!
//!  reaction/diffusion textures
//!  Copyright (c) 1997 Scott Draves spot@transmeta.com
//!  this code is derived from Bomb
//!  see http://www.cs.cmu.edu/~spot/bomb.html
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! And remember: X Windows is to graphics hacking as roman numerals are to
//! the square root of pi.
//! ```
//!
//! The Gray-Scott reaction-diffusion system, from John E. Pearson's "Complex
//! Patterns in a Simple System" (Science, July 1993), run on a wrapping grid
//! in sixteen-bit fixed point. Two chemicals sit at equilibrium until a blob
//! of the second is dropped in the middle; it eats outwards, and what happens
//! when the growing fronts meet each other depends on which of the three
//! reaction and three diffusion kernels came up. The grid wraps, so one tile
//! is computed and repeated across the screen.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, StartArgs, random,
};

const BPS: i64 = 16;
const MX: i32 = (1 << 16) - 1;

/// `R`: upstream strips the top bit, and wonders why in a comment.
fn r() -> i32 {
    (random() & ((1 << 30) - 1)) as i32
}

fn rmod(n: i32) -> i32 {
    if n <= 0 { 0 } else { r() % n }
}

/// `BELLRAND(x)`: three samples averaged, so sizes cluster in the middle.
fn bellrand(x: i32) -> i32 {
    (rmod(x) + rmod(x) + rmod(x)) / 3
}

struct State {
    ncolors: usize,
    colors: Vec<Pixel>,
    /// Dither table: sixteen bits down to eight, with the rounding jittered so
    /// a slow gradient does not band.
    mc: Vec<u8>,

    frame: i64,
    epoch_time: i64,
    r1: Vec<u16>,
    r2: Vec<u16>,
    r1b: Vec<u16>,
    r2b: Vec<u16>,
    width: i32,
    height: i32,
    npix: usize,
    radius: i32,
    radius_res: i32,
    reaction: i32,
    reaction_res: i32,
    diffusion: i32,
    diffusion_res: i32,

    tile: Pixmap,
    array_width: i32,
    array_height: i32,
    array_x: f64,
    array_y: f64,
    array_dx: f64,
    array_dy: f64,

    gc: Gc,
    win_width: i32,
    win_height: i32,
    delay: u32,
}

impl State {
    fn random_colors(&mut self) {
        let map = make_smooth_colormap(self.ncolors.max(2));
        // Scale it up so that there are exactly 255 colours. That keeps the
        // animation speed consistent however many were allocatable.
        let n = 255usize;
        let scale = map.len() as f64 / (n + 1) as f64;
        self.colors = (0..n)
            .map(|i| map[((i as f64 * scale) as usize).min(map.len() - 1)].pixel)
            .collect();
        self.ncolors = n;
    }

    /// One step of the simulation, and the tile it paints.
    fn pixack_frame(&mut self) {
        let w2 = (self.width + 2) as usize;
        let (w, h) = (self.width as usize, self.height as usize);

        if self.frame % self.epoch_time == 0 {
            for i in 0..self.npix {
                // Equilibrium.
                self.r1[i] = 65500;
                self.r2[i] = 11;
            }
            self.random_colors();

            let s = w2 * (h / 2) + w / 2;
            self.radius = self.radius_res;
            let maxr = (self.width / 2 - 2).min(self.height / 2 - 2).max(1);
            if self.radius < 0 {
                self.radius = 1 + if r() % 10 != 0 { r() % 5 } else { rmod(maxr) };
            }
            self.radius = self.radius.min(maxr);
            for i in -self.radius..=self.radius {
                for j in -self.radius..=self.radius {
                    let idx = s as i64 + i as i64 + j as i64 * w2 as i64;
                    if idx >= 0 && (idx as usize) < self.npix {
                        self.r2[idx as usize] = (MX - (r() & 63)) as u16;
                    }
                }
            }
            self.reaction = self.reaction_res;
            if !(0..=2).contains(&self.reaction) {
                self.reaction = r() & 1;
            }
            self.diffusion = self.diffusion_res;
            if !(0..=2).contains(&self.diffusion) {
                self.diffusion = if r() % 5 != 0 {
                    if r() % 3 != 0 { 0 } else { 1 }
                } else {
                    2
                };
            }
            if self.reaction == 2 && self.diffusion == 2 {
                self.reaction = 0;
                self.diffusion = 0;
            }
        }

        // Wrap the edges round.
        for i in 0..=w + 1 {
            self.r1[i] = self.r1[i + w2 * h];
            self.r2[i] = self.r2[i + w2 * h];
            self.r1[i + w2 * (h + 1)] = self.r1[i + w2];
            self.r2[i + w2 * (h + 1)] = self.r2[i + w2];
        }
        for i in 0..=h + 1 {
            self.r1[w2 * i] = self.r1[w + w2 * i];
            self.r2[w2 * i] = self.r2[w + w2 * i];
            self.r1[w2 * i + w + 1] = self.r1[w2 * i + 1];
            self.r2[w2 * i + w + 1] = self.r2[w2 * i + 1];
        }

        for i in 0..h {
            let base = 1 + w2 * (i + 1);
            for j in 0..w {
                let k = base + j;
                let (i1, i2) = (&self.r1, &self.r2);
                let (mut a, mut b);
                match self.diffusion {
                    0 => {
                        a = i1[k] as i32
                            + i1[k + 1] as i32
                            + i1[k - 1] as i32
                            + i1[k + w2] as i32
                            + i1[k - w2] as i32;
                        a /= 5;
                        b = ((i2[k] as i32) << 3)
                            + i2[k + 1] as i32
                            + i2[k - 1] as i32
                            + i2[k + w2] as i32
                            + i2[k - w2] as i32;
                        b /= 12;
                    }
                    1 => {
                        a = i1[k + 1] as i32
                            + i1[k - 1] as i32
                            + i1[k + w2] as i32
                            + i1[k - w2] as i32;
                        a >>= 2;
                        b = ((i2[k] as i32) << 2)
                            + i2[k + 1] as i32
                            + i2[k - 1] as i32
                            + i2[k + w2] as i32
                            + i2[k - w2] as i32;
                        b >>= 3;
                    }
                    _ => {
                        a = ((i1[k] as i32) << 1)
                            + ((i1[k + 1] as i32) << 1)
                            + ((i1[k - 1] as i32) << 1)
                            + i1[k + w2] as i32
                            + i1[k - w2] as i32;
                        a >>= 3;
                        b = ((i2[k] as i32) << 2)
                            + i2[k + 1] as i32
                            + i2[k - 1] as i32
                            + i2[k + w2] as i32
                            + i2[k - w2] as i32;
                        b >>= 3;
                    }
                }

                // Upstream halves the first term before multiplying to keep
                // the product inside a signed int, and puts the bit back at
                // the end.
                let uvv = (((((a >> 1) as i64 * b as i64) >> BPS) * b as i64) >> (BPS - 1)) as i32;
                match self.reaction {
                    0 => {
                        a += 4 * (((28 * (MX - a)) >> 10) - uvv);
                        b += 4 * (uvv - ((80 * b) >> 10));
                    }
                    1 => {
                        a += 3 * (((27 * (MX - a)) >> 10) - uvv);
                        b += 3 * (uvv - ((80 * b) >> 10));
                    }
                    _ => {
                        a += 2 * (((28 * (MX - a)) >> 10) - uvv);
                        b += 3 * (uvv - ((80 * b) >> 10));
                    }
                }
                let a = a.clamp(0, MX);
                let b = b.clamp(0, MX);
                self.r1b[k] = a as u16;
                self.r2b[k] = b as u16;

                let c = self.colors[self.mc[a as usize] as usize % self.ncolors];
                self.tile.put_pixel(j as i32, i as i32, c);
            }
        }

        std::mem::swap(&mut self.r1, &mut self.r1b);
        std::mem::swap(&mut self.r2, &mut self.r2b);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (win_width, win_height) = (d.width(), d.height());

    let mut width = d.res.int("width");
    let mut height = d.res.int("height");
    if width <= 0 && height <= 0 && r() & 1 == 1 {
        width = 64 + bellrand(512);
        height = width;
    }
    if width <= 0 {
        width = 64 + bellrand(512);
    }
    if height <= 0 {
        height = 64 + bellrand(512);
    }
    width = width.min(win_width).max(10);
    height = height.min(win_height).max(10);

    let npix = ((width + 2) * (height + 2)) as usize;

    // Upstream asks for 255 and then, because that is not less than 255,
    // builds a 2047-entry map and samples it back down to 255.
    let asked = d.res.int("ncolors");
    let ncolors = if !(1..255).contains(&asked) {
        2047
    } else {
        asked.max(2)
    } as usize;

    let mut mc = vec![0u8; 1 << 16];
    for (i, slot) in mc.iter_mut().enumerate() {
        *slot = (((i as i32 + (r() & 255)) >> 8).min(255)) as u8;
    }

    let s = d.res.float("size").clamp(0.01, 1.0).sqrt();
    let p = d.res.float("speed");
    let mut array_width = (win_width as f64 * s) as i32;
    let mut array_height = (win_height as f64 * s) as i32;
    if s < 0.99 {
        array_width = (array_width / width) * width;
        array_height = (array_height / height) * height;
    }
    array_width = array_width.max(width);
    array_height = array_height.max(height);

    let mut array_dx = p;
    let mut array_dy = 0.314_159_26 * p;
    // Start in a random direction.
    if random() & 1 == 1 {
        array_dx = -array_dx;
    }
    if random() & 1 == 1 {
        array_dy = -array_dy;
    }

    let mut st = State {
        ncolors,
        colors: vec![0xFF00_0000; 1],
        mc,
        frame: 0,
        epoch_time: d.res.int("epoch").max(1) as i64,
        r1: vec![0; npix],
        r2: vec![0; npix],
        r1b: vec![0; npix],
        r2b: vec![0; npix],
        width,
        height,
        npix,
        radius: -1,
        radius_res: d.res.int("radius"),
        reaction: -1,
        reaction_res: d.res.int("reaction"),
        diffusion: -1,
        diffusion_res: d.res.int("diffusion"),
        tile: Pixmap::new(width, height),
        array_width,
        array_height,
        array_x: ((win_width - array_width) / 2) as f64,
        array_y: ((win_height - array_height) / 2) as f64,
        array_dx,
        array_dy,
        gc: Gc::default(),
        win_width,
        win_height,
        delay: d.res.int("delay").max(0) as u32,
    };
    st.random_colors();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // Compute N frames at once. This speeds up the progress of the
        // animation and the seething, but does not appreciably affect the
        // frame rate.
        const CHUNK: usize = 3;
        for ii in 0..CHUNK {
            self.pixack_frame();

            if ii == CHUNK - 1 {
                // Only need to put the image on the final frame.
                let (tw, th) = (self.width, self.height);
                let (ax, ay) = (self.array_x as i32, self.array_y as i32);
                let mut i = 0;
                while i < self.array_width {
                    let mut j = 0;
                    while j < self.array_height {
                        d.win()
                            .copy_area(&self.gc, &self.tile, 0, 0, tw, th, i + ax, j + ay);
                        j += th;
                    }
                    i += tw;
                }
            }

            self.array_x += self.array_dx;
            self.array_y += self.array_dy;
            let mut bump = false;
            if self.array_x < 0.0 {
                self.array_x = 0.0;
                self.array_dx = -self.array_dx;
                bump = true;
            } else if self.array_x > (self.win_width - self.array_width) as f64 {
                self.array_x = (self.win_width - self.array_width) as f64;
                self.array_dx = -self.array_dx;
                bump = true;
            }
            if self.array_y < 0.0 {
                self.array_y = 0.0;
                self.array_dy = -self.array_dy;
                bump = true;
            } else if self.array_y > (self.win_height - self.array_height) as f64 {
                self.array_y = (self.win_height - self.array_height) as f64;
                self.array_dy = -self.array_dy;
                bump = true;
            }
            if bump && random() & 1 == 1 {
                std::mem::swap(&mut self.array_dx, &mut self.array_dy);
            }

            self.frame += 1;
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.win_width = width;
        self.win_height = height;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*fpsSolid: true",
    "*width: 0",
    "*height: 0",
    "*epoch: 40000",
    "*reaction: -1",
    "*diffusion: -1",
    "*radius: -1",
    "*speed: 0.0",
    "*size: 1.0",
    "*delay: 30000",
    "*ncolors: 255",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 250_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Wander speed", 0.0, 10.0, 0.1, 1, "0.0"),
    Opt::slider("size", "Fill screen", 0.01, 1.0, 0.01, 2, "1.0"),
    Opt::slider("epoch", "Epoch", 1000.0, 300_000.0, 1000.0, 0, "40000"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "255"),
    Opt::spin("width", "X tile size", 0.0, 500.0, "0"),
    Opt::spin("height", "Y tile size", 0.0, 500.0, "0"),
    Opt::spin("reaction", "Reaction", -1.0, 2.0, "-1"),
    Opt::spin("diffusion", "Diffusion", -1.0, 2.0, "-1"),
    Opt::spin("radius", "Seed radius", -1.0, 60.0, "-1"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "rdbomb",
    label: "RD-Bomb",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Scott Draves",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=xiooDyrZSsY"),
        blurb: "Reaction-diffusion: draws a grid of growing square-like shapes that, once they overtake each other, react in unpredictable ways.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
