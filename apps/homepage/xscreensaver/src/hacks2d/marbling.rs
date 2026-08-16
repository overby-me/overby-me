//! Port of `hacks/marbling.c`.
//!
//! ```text
//! marbling, Copyright © 2021-2022 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! This generates a random field with Perlin Noise, then permutes it with
//! Fractal Brownian Motion to create images that somewhat resemble clouds,
//! or the striations in marble, depending on the parameters selected and
//! the colors chosen.
//!
//! Perlin Noise, SIGGRAPH 2002:
//!
//!     https://mrl.cs.nyu.edu/~perlin/noise/
//!     https://en.wikipedia.org/wiki/Perlin_noise
//!
//! Fractal Brownian Motion:
//!
//!     https://en.wikipedia.org/wiki/Fractional_Brownian_motion
//!
//! Initial version by jwz; black magic for pthreads and CPU-specific vector
//! ops added by Dave Odell <dmo2118@gmail.com>.  Here be parallel monsters.
//! ```
//!
//! Perlin noise fed back into itself a few times, which is what turns smooth
//! cloud into the folded striations of marble. All of it is sixteen-bit fixed
//! point: the fade curve, the gradients, the interpolations and the octave
//! weights are integers throughout, so the whole field is computed without a
//! single float in the inner loop.
//!
//! Upstream carries three copies of the arithmetic, one for the vector units
//! of x86, one for those of ARM, and a plain one for everything else. This is
//! the plain one.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XEvent,
    screenhack_event_helper,
};

/// Maximum 13.
const NOISE_WORK_BITS: u32 = 13;
/// Minimum 2.
const LERP_LOSS: u32 = 2;
const NOISE_IN_BITS: u32 = 8;

/// Upstream's comment says this is eight on this path, and the line it
/// replaced would have made it eight. As written it is twenty-one, which
/// makes the rounding term in `scale` shift clean out of the sixteen bits it
/// is cast to, so that step does nothing. Kept as it ships, because that is
/// the picture.
const NOISE_OUT_BITS: u32 = (NOISE_WORK_BITS - 3) * LERP_LOSS + 1;

fn mul_hi(a: u16, b: u16) -> u16 {
    (((a as u32) * (b as u32)) >> 16) as u16
}

fn imul_hi(a: i16, b: i16) -> i16 {
    (((a as i32) * (b as i32)) >> 16) as i16
}

fn lerp(t: i16, a: i16, b: i16) -> i16 {
    (a >> LERP_LOSS).wrapping_add(imul_hi(t, b.wrapping_sub(a)))
}

fn scale(n: i16) -> u16 {
    (n as u16).wrapping_add((1u32 << (NOISE_OUT_BITS - 1)) as u16)
}

fn fade(t: u16) -> u16 {
    const F: u16 = 256;
    let t2 = t.wrapping_mul(t);
    let inner = (15u16.wrapping_mul(F)).wrapping_sub(t.wrapping_mul(6));
    let outer = (10u16.wrapping_mul(t)).wrapping_sub(mul_hi(t2, inner));
    mul_hi(t2, outer) << (NOISE_WORK_BITS - NOISE_IN_BITS + 1)
}

/// Perlin's code used pre-computed random numbers; this is an eight-bit
/// minimal perfect hash instead. Not a good source of randomness, but good
/// enough here.
fn noise_rand(mut x: u16) -> u16 {
    x ^= x >> 3;
    x ^= x << 1;
    (x << 5).wrapping_sub(x)
}

fn p(x: u16) -> u16 {
    noise_rand(x & 0xff)
}

/// Convert the low four bits of the hash code into twelve gradient
/// directions.
fn grad(hash: u16, x: u16, y: u16, z: u16) -> i16 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if (h & !2) == 12 {
        x
    } else {
        z
    };
    let a = if h & 1 == 0 { u as i32 } else { -(u as i32) };
    let b = if h & 2 == 0 { v as i32 } else { -(v as i32) };
    a.wrapping_add(b) as i16
}

fn noise(mut x: u16, mut y: u16, mut z: u16) -> u16 {
    let one: u16 = 1 << NOISE_WORK_BITS;
    // Find the unit cube that contains the point, and the point within it.
    let bx = x >> NOISE_IN_BITS;
    let by = y >> NOISE_IN_BITS;
    let bz = z >> NOISE_IN_BITS;
    x &= (1 << NOISE_IN_BITS) - 1;
    y &= (1 << NOISE_IN_BITS) - 1;
    z &= (1 << NOISE_IN_BITS) - 1;
    // Compute the fade curves for each of x, y and z.
    let u = fade(x) as i16;
    let v = fade(y) as i16;
    let w = fade(z) as i16;

    // Hash the coordinates of the eight cube corners. Perlin calls these
    // by two-letter names; named here for which corner of the square they
    // are instead.
    let a = noise_rand(bx).wrapping_add(by);
    let x0y0 = p(a).wrapping_add(bz);
    let x0y1 = p(a.wrapping_add(1)).wrapping_add(bz);
    let b = p(bx.wrapping_add(1)).wrapping_add(by);
    let x1y0 = p(b).wrapping_add(bz);
    let x1y1 = p(b.wrapping_add(1)).wrapping_add(bz);

    let sh = NOISE_WORK_BITS - NOISE_IN_BITS;
    x <<= sh;
    y <<= sh;
    z <<= sh;
    let xo = x.wrapping_sub(one);
    let yo = y.wrapping_sub(one);
    let zo = z.wrapping_sub(one);

    let c0 = grad(p(x0y0), x, y, z);
    let c1 = grad(p(x1y0), xo, y, z);
    let c2 = grad(p(x0y1), x, yo, z);
    let c3 = grad(p(x1y1), xo, yo, z);
    let c4 = grad(p(x0y0.wrapping_add(1)), x, y, zo);
    let c5 = grad(p(x1y0.wrapping_add(1)), xo, y, zo);
    let c6 = grad(p(x0y1.wrapping_add(1)), x, yo, zo);
    let c7 = grad(p(x1y1.wrapping_add(1)), xo, yo, zo);

    // Add the blended results from the eight corners of the cube.
    scale(lerp(
        w,
        lerp(v, lerp(u, c0, c1), lerp(u, c2, c3)),
        lerp(v, lerp(u, c4, c5), lerp(u, c6, c7)),
    ))
}

/// Two octaves of noise, the second at half the weight and twice the
/// frequency.
fn fbm(x: u16, y: u16, z: u16) -> u16 {
    const OCTAVES: usize = 2;
    // exp2(-0.5), as a sixteen-bit fraction.
    let ig: u16 = ((std::f64::consts::FRAC_1_SQRT_2) * 65536.0) as i32 as u16;
    let mut f: u16 = 1;
    let mut a: u16 = 0xffff;
    let mut t: u16 = 0;
    for _ in 0..OCTAVES {
        t = t.wrapping_add(mul_hi(
            noise(f.wrapping_mul(x), f.wrapping_mul(y), f.wrapping_mul(z)),
            a,
        ));
        a = mul_hi(a, ig);
        f = f.wrapping_mul(2);
    }
    t
}

struct State {
    gc: Gc,
    delay: u32,
    ncolors: usize,
    colors: Vec<Pixel>,
    grid_size: i32,
    w: i32,
    h: i32,
    scale_res: i32,
    iterations: i32,
    z: u16,
}

impl State {
    fn recolor(&mut self) {
        self.ncolors = 256;
        self.colors = make_smooth_colormap(self.ncolors)
            .iter()
            .map(|c| c.pixel)
            .collect();
        self.ncolors = self.colors.len().max(1);
    }

    fn reset(&mut self, width: i32, height: i32) {
        let g = self.grid_size;
        self.w = (width + g - 1) / g;
        self.h = (height + g - 1) / g;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut st = State {
        gc: Gc::default(),
        delay: d.res.int("delay").max(0) as u32,
        ncolors: 256,
        colors: Vec::new(),
        grid_size: d.res.int("gridsize").max(1),
        w: 1,
        h: 1,
        scale_res: d.res.int("gridScale").max(1),
        iterations: d.res.int("iterations").max(1),
        z: 0,
    };
    st.recolor();
    st.reset(d.width(), d.height());
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let g = self.grid_size;
        let s = (self.scale_res << NOISE_IN_BITS) as f32;
        let xd: u32 = (0x10000u32 / self.w.max(1) as u32).wrapping_mul(s as u32);

        for y in 0..self.h {
            let yv = (y as f32 / self.h as f32 * s) as u16;
            let mut xfx: u32 = 0;
            for x in 0..self.w {
                let x0 = (xfx >> 16) as u16;

                // Feed the field back into itself, which is what folds the
                // smooth cloud into striations.
                let mut pv: u16 = 0;
                for _ in 0..self.iterations {
                    pv = fbm(
                        pv.wrapping_add(x0),
                        pv.wrapping_add(yv),
                        pv.wrapping_add(self.z),
                    );
                }

                let idx =
                    ((pv & ((1 << NOISE_IN_BITS) - 1)) as usize * self.ncolors) >> NOISE_IN_BITS;
                self.gc
                    .set_foreground(self.colors[idx.min(self.ncolors - 1)]);
                d.win().fill_rectangle(&self.gc, x * g, y * g, g, g);

                xfx = xfx.wrapping_add(xd);
            }
        }

        self.z = self
            .z
            .wrapping_add((0.01 * (1 << NOISE_IN_BITS) as f64) as i16 as u16);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.reset(width, height);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.recolor();
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    "*delay: 10000",
    "*background: black",
    "*gridsize: 2",
    // Using "scale" screws up the fps fonts.
    "*gridScale: 10",
    "*iterations: 5",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("gridsize", "Magnification", 1.0, 20.0, 1.0, 0, "2"),
    Opt::slider("gridScale", "Scale", 1.0, 20.0, 1.0, 0, "10"),
    Opt::slider("iterations", "Complexity", 1.0, 10.0, 1.0, 0, "5"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "marbling",
    label: "Marbling",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski and Dave Odell",
        year: "2021",
        video: Some("https://www.youtube.com/watch?v=D20sPMLwS1c"),
        blurb: "Marble-like or cloud-like patterns generated using Perlin Noise and Fractal Brownian Motion.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
