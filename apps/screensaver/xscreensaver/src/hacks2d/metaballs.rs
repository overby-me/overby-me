//! Port of `hacks/metaballs.c`.
//!
//! ```text
//! MetaBalls, Copyright (c) 2002-2003 W.P. van Paassen <peter@paassen.tmfweb.nl>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Module - "metaballs.c"
//!
//! [01/24/03] - W.P. van Paassen: Additional features
//! [12/29/02] - W.P. van Paassen: Port to X for use with XScreenSaver, the
//!              shadebob hack by Shane Smit was used as a template
//! [12/26/02] - W.P. van Paassen: Creation for the Demo Effects Collection
//!              (http://demo-effects.sourceforge.net)
//! ```
//!
//! Two-dimensional metaballs. A disc of density values is stamped down once per
//! ball into an accumulation buffer, and the total at each pixel indexes a ramp
//! that runs black to a base colour to white. Because the densities add rather
//! than overwrite, two balls that come near each other grow a neck and merge,
//! which is the whole trick.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XColor, frand, random,
};

/// `BELLRAND(n)`: three samples averaged, so the result clusters in the middle.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

#[derive(Clone, Copy, Default)]
struct Blob {
    xpos: i32,
    ypos: i32,
}

struct MetaBalls {
    width: i32,
    height: i32,
    radius: i32,
    delta: i32,
    dradius: i32,
    /// One ball's density disc, `dradius` square.
    blob: Vec<u8>,
    blobs: Vec<Blob>,
    /// Accumulated density, one byte a pixel.
    blub: Vec<u8>,
    delay: u32,
    cycles: i32,
    ncolors: i32,
    colors: Vec<Pixel>,
    draw_i: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let nblobs = d.res.int("count").clamp(2, 255) as usize;

    let mut st = MetaBalls {
        width: d.width(),
        height: d.height(),
        radius: 2,
        delta: d.res.int("delta").clamp(1, 20),
        dradius: 4,
        blob: Vec::new(),
        blobs: vec![Blob::default(); nblobs],
        blub: Vec::new(),
        delay: d.res.int("delay").max(0) as u32,
        cycles: d.res.int("cycles").max(1),
        ncolors: d.res.int("ncolors").clamp(2, 255),
        colors: Vec::new(),
        draw_i: -1,
    };
    st.initialize(d);
    Box::new(st)
}

impl MetaBalls {
    fn init_blob(&self, b: &mut Blob) {
        b.xpos = (self.width as f64 / 4.0 + bellrand(self.width as f64 / 2.0) - self.radius as f64)
            as i32;
        b.ypos = (self.height as f64 / 4.0 + bellrand(self.height as f64 / 2.0)
            - self.radius as f64) as i32;
    }

    /// Build the palette: black up to a random base colour, then on to white.
    fn set_palette(&mut self) {
        let base = (
            (random() % 0xFFFF) as f64,
            (random() % 0xFFFF) as f64,
            (random() % 0xFFFF) as f64,
        );
        let n = self.ncolors;
        let half = n as f64 / 2.0;

        self.colors = (0..n)
            .map(|i| {
                let f = i as f64;
                let (r, g, b) = if i < n / 2 {
                    (base.0 / half * f, base.1 / half * f, base.2 / half * f)
                } else {
                    (
                        ((0xFFFF as f64 - base.0) / half) * (f - half) + base.0,
                        ((0xFFFF as f64 - base.1) / half) * (f - half) + base.1,
                        ((0xFFFF as f64 - base.2) / half) * (f - half) + base.2,
                    )
                };
                XColor::from_rgb16(r as u16, g as u16, b as u16).pixel
            })
            .collect();
    }

    fn initialize(&mut self, d: &mut Dpy) {
        self.width = d.width();
        self.height = d.height();

        let mut radius = d.res.int("radius").clamp(2, 100);
        radius = (radius as f64 / 100.0 * (self.height >> 3) as f64) as i32;
        // `dradius` has to fit in the byte upstream keeps it in.
        if radius >= 128 {
            radius = 127;
        }
        if (self.width < 100 || self.height < 100) && radius < 20 {
            // Tiny window.
            radius = 20;
        }
        self.radius = radius.max(1);
        self.dradius = self.radius * 2;

        // The density disc: full in the middle, tailing off to nothing at the
        // rim on a fourth-power curve.
        let sradius = (self.radius * self.radius) as f64;
        let dr = self.dradius as usize;
        self.blob = vec![0; dr * dr];
        for i in -self.radius..self.radius {
            for j in -self.radius..self.radius {
                let distance_squared = (i * i + j * j) as f64;
                let v = if distance_squared <= sradius {
                    let fraction = distance_squared / sradius;
                    ((1.0 - fraction * fraction).powf(4.0) * 255.0) as u8
                } else {
                    0
                };
                let at = (i + self.radius) as usize * dr + (j + self.radius) as usize;
                self.blob[at] = v;
            }
        }

        self.blub = vec![0; (self.width * self.height) as usize];
        self.set_palette();

        let mut blobs = std::mem::take(&mut self.blobs);
        for b in blobs.iter_mut() {
            self.init_blob(b);
        }
        self.blobs = blobs;
    }

    fn execute(&mut self, d: &mut Dpy) {
        let (w, h) = (self.width, self.height);
        self.blub.fill(0);

        for b in self.blobs.iter_mut() {
            b.xpos += -self.delta + ((self.delta as f64 + 0.5) * frand(2.0)) as i32;
            b.ypos += -self.delta + ((self.delta as f64 + 0.5) * frand(2.0)) as i32;
        }

        let top = (self.ncolors - 1) as u8;
        let dr = self.dradius;
        for k in 0..self.blobs.len() {
            let b = self.blobs[k];
            if b.ypos > -dr && b.xpos > -dr && b.ypos < h && b.xpos < w {
                for i in 0..dr {
                    let y = b.ypos + i;
                    if y < 0 || y >= h {
                        continue;
                    }
                    for j in 0..dr {
                        let x = b.xpos + j;
                        if x < 0 || x >= w {
                            continue;
                        }
                        let at = (y * w + x) as usize;
                        if self.blub[at] < top {
                            let add = self.blob[(i * dr + j) as usize];
                            self.blub[at] = if self.blub[at] as u32 + add as u32 > top as u32 {
                                top
                            } else {
                                self.blub[at] + add
                            };
                        }
                    }
                }
            } else {
                let mut moved = self.blobs[k];
                self.init_blob(&mut moved);
                self.blobs[k] = moved;
            }
        }

        // Index zero of the ramp is black, which is also the background, so
        // painting the whole buffer is the same as upstream's skip.
        let px = d.win().pixels_mut();
        for (dst, level) in px.iter_mut().zip(self.blub.iter()) {
            *dst = self.colors[*level as usize];
        }
    }
}

impl Screenhack for MetaBalls {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // Upstream tests the old value and increments in the same expression,
        // so the counter only stands still on the very first frame.
        let expired = if self.draw_i < 0 {
            true
        } else {
            let old = self.draw_i;
            self.draw_i += 1;
            old >= self.cycles
        };

        if expired {
            self.draw_i = 0;
            self.set_palette();
            d.clear_window();
            let mut blobs = std::mem::take(&mut self.blobs);
            for b in blobs.iter_mut() {
                self.init_blob(b);
            }
            self.blobs = blobs;
        }

        self.execute(d);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, _width: i32, _height: i32) {
        self.initialize(d);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*count: 10",
    "*cycles: 1000",
    "*ncolors: 256",
    "*delay: 10000",
    "*radius: 100",
    "*delta: 3",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("cycles", "Duration", 100.0, 3000.0, 100.0, 0, "1000"),
    Opt::slider("ncolors", "Number of colors", 2.0, 256.0, 1.0, 0, "256"),
    Opt::slider("count", "Ball count", 2.0, 255.0, 1.0, 0, "10"),
    Opt::slider("radius", "Ball radius", 2.0, 100.0, 1.0, 0, "100"),
    Opt::slider("delta", "Ball movement", 1.0, 20.0, 1.0, 0, "3"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "metaballs",
    label: "Meta Balls",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "W.P. van Paassen",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=wcdKHCp9foY"),
        blurb: "Overlapping and merging balls with fuzzy edges.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
