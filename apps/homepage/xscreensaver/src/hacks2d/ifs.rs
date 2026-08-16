//! Port of `hacks/ifs.c`.
//!
//! ```text
//! Copyright © Chris Le Sueur and Robby Griffin, 2005-2006
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
//! Ultimate thanks go to Massimino Pascal, who created the original
//! xscreensaver hack, and inspired me with it's swirly goodness. This
//! version adds things like variable quality, number of functions and also
//! a groovier colouring mode.
//!
//! This version by Chris Le Sueur <thefishface@gmail.com>, Feb 2005
//! Many improvements by Robby Griffin <rmg@terc.edu>, Mar 2006
//! Multi-coloured mode added by Jack Grahl <j.grahl@ucl.ac.uk>, Jan 2007
//! ```
//!
//! An iterated function system: a handful of affine maps, applied over and
//! over to a wandering point, whose orbit settles onto the fractal they share.
//! Each map drifts through its own rotation, scale and offset, so the cloud
//! twists and folds from frame to frame.
//!
//! The arithmetic is fixed point, as upstream's is: coordinates carry an extra
//! factor of 256 and the matrix an extra 1024, which is what the `>> 10` in
//! `step_x`/`step_y` takes back out.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XColor, XEvent,
    XRectangle, frand, random, screenhack_event_helper,
};

/// One affine map. `r`/`s`/`tx`/`ty` are what the mutation moves; the `u*`
/// fields are the same thing precomputed as integers, ready to multiply.
#[derive(Clone, Copy, Default)]
struct Lens {
    /// Rotation, scale, translation.
    r: f32,
    s: f32,
    tx: f32,
    ty: f32,
    /// Old rotation, its target, and how far between the two we are.
    ro: f32,
    rt: f32,
    rc: f32,
    /// The same three for the scale.
    so: f32,
    st: f32,
    sc: f32,
    /// Rate of change of the translation.
    txa: f32,
    tya: f32,
    /// The combined matrix and offset, in fixed point.
    ua: i64,
    ub: i64,
    utx: i64,
    uc: i64,
    ud: i64,
    uty: i64,
}

struct State {
    gc: Gc,
    back: Pixmap,
    colours: Vec<XColor>,
    ncolours: usize,
    ccolour: usize,
    black: u32,

    width: i32,
    widthb: usize,
    height: i32,
    width8: i32,
    height8: i32,
    /// One bit per pixel, so a point already drawn this frame costs nothing.
    board: Vec<u32>,
    pointbuf: Vec<XRectangle>,
    xmin: i32,
    xmax: i32,
    ymin: i32,
    ymax: i32,
    x: i32,
    y: i32,
    pscale: i32,

    delay: u32,
    lensnum: usize,
    lenses: Vec<Lens>,
    length: i32,
    recurse: bool,
    multi: bool,
    translate: bool,
    scale: bool,
    rotate: bool,
}

/// The most points one frame will plot, however high the detail is turned up.
const FRAME_BUDGET: i64 = 1_000_000;

fn step_x(l: &Lens, x: i32, y: i32) -> i32 {
    ((l.ua * x as i64 + l.ub * y as i64 + l.utx) >> 10) as i32
}

fn step_y(l: &Lens, x: i32, y: i32) -> i32 {
    ((l.uc * x as i64 + l.ud * y as i64 + l.uty) >> 10) as i32
}

impl State {
    /// `lensmatrix`: fold rotation, scale and translation into the integer
    /// matrix the inner loop uses.
    fn lensmatrix(&self, l: &mut Lens) {
        let (r, s) = (l.r as f64, l.s as f64);
        l.ua = (1024.0 * s * r.cos()) as i64;
        l.ub = (-1024.0 * s * r.sin()) as i64;
        l.uc = -l.ub;
        l.ud = l.ua;
        l.utx = (131072.0
            * self.width as f64
            * (s * (r.sin() - r.cos()) + l.tx as f64 / 16.0 + 1.0)) as i64;
        l.uty = (-131072.0
            * self.height as f64
            * (s * (r.sin() + r.cos()) + l.ty as f64 / 16.0 - 1.0)) as i64;
    }

    fn create_lens(&self, nr: f32, ns: f32, nx: f32, ny: f32) -> Lens {
        let mut l = Lens {
            tx: nx,
            ty: ny,
            ..Lens::default()
        };
        if self.rotate {
            l.r = nr;
            l.ro = nr;
            l.rt = nr;
            l.rc = 1.0;
        }
        if self.scale {
            l.s = ns;
            l.so = ns;
            l.st = ns;
            l.sc = 1.0;
        } else {
            l.s = 0.5;
        }
        self.lensmatrix(&mut l);
        l
    }

    fn random_lens(&self) -> Lens {
        self.create_lens(
            frand(1.0) as f32 - 0.5,
            frand(1.0) as f32,
            frand(4.0) as f32 - 2.0,
            frand(4.0) as f32 + 2.0,
        )
    }

    /// `mutate`: walk each lens towards a fresh random target, on a sine so the
    /// arrival and departure are both smooth.
    fn mutate(&self, l: &mut Lens) {
        if self.rotate {
            if l.rc >= 1.0 {
                l.rc = 0.0;
                l.ro = l.rt;
                l.rt = frand(4.0) as f32 - 2.0;
            }
            let factor =
                ((-std::f32::consts::PI / 2.0 + std::f32::consts::PI * l.rc).sin() + 1.0) / 2.0;
            l.r = l.ro + (l.rt - l.ro) * factor;
            l.rc += 0.01;
        }
        if self.scale {
            if l.sc >= 1.0 {
                l.sc = 0.0;
                l.so = l.st;
                l.st = frand(2.0) as f32 - 1.0;
            }
            let factor =
                ((-std::f32::consts::PI / 2.0 + std::f32::consts::PI * l.sc).sin() + 1.0) / 2.0;
            l.s = l.so + (l.st - l.so) * factor;
            l.sc += 0.01;
        }
        if self.translate {
            l.txa += frand(0.004) as f32 - 0.002;
            l.tya += frand(0.004) as f32 - 0.002;
            l.tx += l.txa;
            l.ty += l.tya;
            if l.tx > 6.0 {
                l.txa -= 0.004;
            }
            if l.ty > 6.0 {
                l.tya -= 0.004;
            }
            if l.tx < -6.0 {
                l.txa += 0.004;
            }
            if l.ty < -6.0 {
                l.tya += 0.004;
            }
            if l.txa > 0.05 || l.txa < -0.05 {
                l.txa /= 1.7;
            }
            if l.tya > 0.05 || l.tya < -0.05 {
                l.tya /= 1.7;
            }
        }
        if self.rotate || self.scale || self.translate {
            self.lensmatrix(l);
        }
    }

    /// `drawpoints`: flush the queue onto the back buffer.
    fn drawpoints(&mut self) {
        self.back.fill_rectangles(&self.gc, &self.pointbuf);
        self.pointbuf.clear();
    }

    /// `sp`: mark a point, unless this frame has drawn it already. The
    /// coordinates arrive in units of 1/256 of a pixel.
    fn sp(&mut self, x: i32, y: i32) {
        if x < 0 || x >= self.width8 || y < 0 || y >= self.height8 {
            return;
        }
        let x = x >> 8;
        let y = y >> 8;

        let word = y as usize * self.widthb + (x as usize >> 5);
        let bit = 1u32 << (x & 31);
        if self.board[word] & bit != 0 {
            return;
        }
        self.board[word] |= bit;

        self.xmin = self.xmin.min(x);
        self.xmax = self.xmax.max(x);
        self.ymin = self.ymin.min(y);
        self.ymax = self.ymax.max(y);

        self.pointbuf.push(XRectangle {
            x,
            y,
            width: self.pscale,
            height: self.pscale,
        });
        if self.pointbuf.len() >= 1000 {
            self.drawpoints();
        }
    }

    /// `recurse`: apply every lens at every level, so the whole tree of orbits
    /// is drawn rather than a random walk through it.
    fn recurse(&mut self, x: i32, y: i32, length: i32, p: usize) {
        if length == 0 {
            if p == 0 {
                self.sp(x, y);
            } else {
                let l = self.lenses[p];
                self.sp(step_x(&l, x, y), step_y(&l, x, y));
            }
            return;
        }
        for i in 0..self.lensnum {
            let l = self.lenses[i];
            self.recurse(step_x(&l, x, y), step_y(&l, x, y), length - 1, p);
        }
    }

    /// `iterate`: the chaos game. Ten steps to settle onto the attractor, then
    /// a point per step after that.
    fn iterate(&mut self, count: i64, p: usize) {
        let mut x = self.x;
        let mut y = self.y;
        let mut i: i64 = 0;

        while i < 10 {
            let l = self.lenses[(random() as usize) % self.lensnum];
            let tx = step_x(&l, x, y);
            y = step_y(&l, x, y);
            x = tx;
            i += 1;
        }
        while i < count {
            let l = self.lenses[(random() as usize) % self.lensnum];
            let tx = step_x(&l, x, y);
            y = step_y(&l, x, y);
            x = tx;
            if p == 0 {
                self.sp(x, y);
            } else {
                let l = self.lenses[p];
                self.sp(step_x(&l, x, y), step_y(&l, x, y));
            }
            i += 1;
        }

        self.x = x;
        self.y = y;
    }

    /// How many points one pass draws: `lensnum ^ length`, but no more than a
    /// frame's share of [`FRAME_BUDGET`].
    ///
    /// Upstream has no such limit, and says so in its own config file: detail
    /// 14 runs at eight frames a second on a fast desktop, detail 15 at two.
    /// A browser tab that stops answering for half a second is worse than a
    /// slightly sparser cloud, so the top of the slider is where this bites.
    /// The default, detail 9 with three functions, is nowhere near it.
    fn point_count(&self, length: i32, passes: usize) -> i64 {
        let cap = (FRAME_BUDGET / passes.max(1) as i64).max(1);
        let mut n: i64 = 1;
        for _ in 0..length.max(0) {
            n = n.saturating_mul(self.lensnum as i64);
            if n > cap {
                return cap;
            }
        }
        n
    }

    fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        self.widthb = ((width + 31) >> 5) as usize;
        self.height = height;
        self.width8 = width << 8;
        self.height8 = height << 8;

        if self.xmax == 0 && self.ymax == 0 && self.xmin == 0 && self.ymin == 0 {
            self.xmin = width + 1;
            self.xmax = -1;
            self.ymax = -1;
            self.ymin = height + 1;
        }

        self.back = Pixmap::new(width, height);
        self.back.clear(self.black);
        self.board = vec![0; self.widthb * height.max(0) as usize];
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let black = d.res.pixel("background");
    let mut st = State {
        gc: Gc::new(black, black),
        back: Pixmap::new(1, 1),
        colours: Vec::new(),
        ncolours: 0,
        ccolour: 0,
        black,
        width: 0,
        widthb: 0,
        height: 0,
        width8: 0,
        height8: 0,
        board: Vec::new(),
        pointbuf: Vec::with_capacity(1000),
        xmin: 0,
        xmax: 0,
        ymin: 0,
        ymax: 0,
        x: 0,
        y: 0,
        pscale: 1,
        delay: d.res.int("delay").max(0) as u32,
        lensnum: d.res.int("lensnum").clamp(1, 8) as usize,
        lenses: Vec::new(),
        length: d.res.int("length").max(0),
        recurse: d.res.bool("recurse"),
        multi: d.res.bool("multi"),
        translate: d.res.bool("translate"),
        scale: d.res.bool("ifsScale"),
        rotate: d.res.bool("rotate"),
    };
    st.resize(d.width(), d.height());

    if d.width() > 2560 || d.height() > 2560 {
        st.pscale *= 3; // Retina displays.
    }

    st.ncolours = d.res.int("colors").max(1) as usize;
    st.ncolours = st.ncolours.max(st.lensnum);
    st.colours = make_smooth_colormap(st.ncolours);
    st.ncolours = st.colours.len().max(1);

    st.lenses = (0..st.lensnum).map(|_| st.random_lens()).collect();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (xmin, xmax, ymin, ymax) = (self.xmin, self.xmax, self.ymin, self.ymax);

        // Erase whatever was drawn in the previous frame.
        if xmin <= xmax && ymin <= ymax {
            self.gc.set_foreground(self.black);
            self.back.fill_rectangle(
                &self.gc,
                xmin,
                ymin,
                xmax - xmin + self.pscale,
                ymax - ymin + self.pscale,
            );
            self.xmin = self.width + 1;
            self.xmax = -1;
            self.ymax = -1;
            self.ymin = self.height + 1;
        }

        self.ccolour = (self.ccolour + 1) % self.ncolours;

        let x = self.width << 7;
        let y = self.height << 7;

        if self.multi {
            for i in 0..self.lensnum {
                let partcolor = (self.ccolour * (i + 1)) % self.ncolours;
                self.gc.set_foreground(self.colours[partcolor].pixel);
                self.board.fill(0);
                if self.recurse {
                    self.recurse(x, y, self.length - 1, i);
                } else {
                    let n = self.point_count(self.length - 1, self.lensnum);
                    self.iterate(n, i);
                }
                if !self.pointbuf.is_empty() {
                    self.drawpoints();
                }
            }
        } else {
            self.gc.set_foreground(self.colours[self.ccolour].pixel);
            self.board.fill(0);
            if self.recurse {
                self.recurse(x, y, self.length, 0);
            } else {
                let n = self.point_count(self.length, 1);
                self.iterate(n, 0);
            }
            if !self.pointbuf.is_empty() {
                self.drawpoints();
            }
        }

        // Copy the changed area, erasure included, to the screen.
        if (self.xmin <= self.xmax && self.ymin <= self.ymax) || (xmin <= xmax && ymin <= ymax) {
            let x0 = xmin.min(self.xmin).max(0);
            let x1 = xmax.max(self.xmax).min(self.width - 1);
            let y0 = ymin.min(self.ymin).max(0);
            let y1 = ymax.max(self.ymax).min(self.height - 1);
            let copy = Gc::new(self.black, self.black);
            d.win()
                .copy_area(&copy, &self.back, x0, y0, x1 - x0 + 1, y1 - y0 + 1, x0, y0);
        }

        for i in 0..self.lensnum {
            let mut l = self.lenses[i];
            self.mutate(&mut l);
            self.lenses[i] = l;
        }

        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.resize(width, height);
        d.clear_window();
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.lenses = (0..self.lensnum).map(|_| self.random_lens()).collect();
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    "*lensnum: 3",
    "*fpsSolid: true",
    "*length: 9",
    "*mode: 0",
    "*colors: 200",
    "*delay: 20000",
    "*translate: True",
    "*ifsScale: True",
    "*rotate: True",
    "*recurse: False",
    "*multi: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("lensnum", "Number of functions", 2.0, 6.0, 1.0, 0, "3"),
    Opt::slider("length", "Detail", 4.0, 14.0, 1.0, 0, "9"),
    Opt::slider("colors", "Number of colors", 2.0, 255.0, 1.0, 0, "200"),
    Opt::boolean("translate", "Translate", "True"),
    Opt::boolean("ifsScale", "Scale", "True"),
    Opt::boolean("rotate", "Rotate", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "ifs",
    label: "IFS",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Chris Le Sueur and Robby Griffin",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=0uOIrVFsECM"),
        blurb: "Clouds of iterated function systems spin and collide.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
