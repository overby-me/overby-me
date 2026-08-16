//! Port of `hacks/xlyap.c`.
//!
//! ```text
//! Lyap - calculate and display Lyapunov exponents
//!
//! Written by Ron Record (rr@sco) 03 Sep 1991
//!
//! The idea here is to calculate the Lyapunov exponent for a periodically
//! forced logistic map (later i added several other nonlinear maps of the unit
//! interval). In order to turn the 1-dimensional parameter space of the
//! logistic map into a 2-dimensional parameter space, select two parameter
//! values ('a' and 'b') then alternate the iterations of the logistic map using
//! first 'a' then 'b' as the parameter. This program accepts an argument to
//! specify a forcing function, so instead of just alternating 'a' and 'b', you
//! can use 'a' as the parameter for say 6 iterations, then 'b' for 6 iterations
//! and so on. An interesting forcing function to look at is abbabaab (the
//! Morse-Thue sequence, an aperiodic self-similar, self-generating sequence).
//! Anyway, step through all the values of 'a' and 'b' in the ranges you want,
//! calculating the Lyapunov exponent for each pair of values. The exponent
//! is calculated by iterating out a ways (specified by the variable "settle")
//! then on subsequent iterations calculating an average of the logarithm of
//! the absolute value of the derivative at that point. Points in parameter
//! space with a negative Lyapunov exponent are colored one way (using the
//! value of the exponent to index into a color map) while points with a
//! non-negative exponent are colored differently.
//!
//! The algorithm was taken from the September 1991 Scientific American article
//! by A. K. Dewdney who gives credit to Mario Markus of the Max Planck
//! Institute for its creation. Additional information and ideas were gleaned
//! from the discussion on alt.fractals involving Stephen Hall, Ed Kubaitis,
//! Dave Platt and Baback Moghaddam. Assistance with colormaps and spinning
//! color wheels and X was gleaned from Hiram Clawson. Rubber banding code was
//! adapted from an existing Mandelbrot program written by Stacey Campbell.
//! ```
//!
//! The picture is built one pixel at a time, left to right and top to bottom,
//! two thousand pixels a frame, and when the last one lands the saver waits a
//! few seconds and starts over on one of twenty-two stored parameter windows.
//!
//! It does not appear in that order, though. Points are held back until two
//! hundred and fifty-six of one colour have accumulated and then drawn in one
//! go, so the picture arrives as a shower of one shade at a time, and a window
//! that has been running for only a few seconds can still be empty.
//!
//! Upstream carries a good deal of machinery this port leaves out, all of it
//! for a display this one does not have: the colour wheels and their spin
//! (`rgbMax`, `spinLength`, `colorOffset`, `wheels`) worked by writing a
//! read-write colormap, which a canvas has no equivalent of, so the colours
//! here come from `make_smooth_colormap` as they do in the modern C; the
//! rubber-band zoom and its stack of stored views were already `#if 0` in the
//! source; and `-o` wrote the exponents to a file.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::make_smooth_colormap;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XEvent, XPoint, random,
    screenhack_event_helper,
};

/// How many points are buffered per colour before they are drawn.
const MAXPOINTS: usize = 256;
/// Stored views. Upstream's `BIGMEM` would make this eight; it is never
/// defined, and with the rubber-band zoom gone only the first is ever used.
const MAXFRAMES: usize = 2;
/// The hard cap on `maxColor`, which upstream asserts on three times over.
const MAXCOLOR: usize = 256;
/// The longest forcing function.
const MAXINDEX: usize = 64;
/// The stored parameter windows `-builtin` selects between.
const NBUILTINS: u32 = 22;

/// The five maps of the unit interval, indexed as upstream's `Maps[]`:
/// logistic, sine hump, left-skewed logistic, right-skewed, double.
fn map(which: usize, x: f64, r: f64) -> f64 {
    match which {
        1 => r * (std::f64::consts::PI * x).sin(),
        2 => {
            let d = 1.0 - x;
            r * x * d * d
        }
        3 => r * x * x * (1.0 - x),
        4 => {
            let d = 1.0 - x;
            r * x * x * d * d
        }
        _ => r * x * (1.0 - x),
    }
}

/// `Derivs[]`, the derivative of each of the above with respect to x.
fn deriv(which: usize, x: f64, r: f64) -> f64 {
    match which {
        1 => r * std::f64::consts::PI * (std::f64::consts::PI * x).cos(),
        2 => r * (1.0 - (4.0 * x) + (3.0 * x * x)),
        3 => r * ((2.0 * x) - (3.0 * x * x)),
        4 => {
            let d = x * x;
            r * ((2.0 * x) - (6.0 * d) + (4.0 * x * d))
        }
        _ => r - (2.0 * r * x),
    }
}

struct Xlyap {
    gc: Gc,
    delay: u32,
    linger: i32,

    /// Colours the picture is drawn in, one per index. Upstream keeps a `GC`
    /// per colour; a single GC whose foreground is set per flush is the same
    /// thing with less bookkeeping.
    pixels: Vec<Pixel>,
    maxcolor: usize,
    /// Where the positive-exponent colours start, and where the negative ones
    /// do. At the defaults the first is above the second, so `lowrange` below
    /// comes out negative and the positive band is indexed downwards.
    startcolor: i32,
    mincolindex: i32,
    numcolors: i32,
    numfreecols: i32,
    lowrange: i32,

    dwell: i32,
    settle: i32,
    width: i32,
    height: i32,

    /// The points waiting to be drawn, one bucket per colour.
    points: Vec<Vec<XPoint>>,

    /// The map being iterated, upstream's `map` and `deriv` pointers.
    curmap: usize,
    /// The map *index*, which is not the same thing: the stored views set this
    /// without setting the pair above, so they do not in fact change the map.
    /// Only the resource and the map key move both.
    mapindex: usize,
    /// Set once a parameter has been pinned by a resource or a preset, which
    /// stops the map-cycling key from moving it.
    aflag: bool,
    bflag: bool,
    wflag: bool,
    hflag: bool,
    /// Draw a fresh forcing function every time the old one runs out.
    rflag: bool,

    maxindex: usize,
    min_a: f64,
    min_b: f64,
    max_a: f64,
    max_b: f64,
    a_range: f64,
    b_range: f64,
    a_inc: f64,
    b_inc: f64,
    a: f64,
    b: f64,
    start_x: f64,
    lyapunov: f64,
    minlyap: f64,
    minexp: f64,
    maxexp: f64,
    prob: f64,
    useprod: bool,

    point: XPoint,
    /// Every exponent computed for a view, so the picture can be recoloured
    /// without recomputing it. Two views of `width * (height + 1)` doubles is
    /// what upstream mallocs, and it is the one large allocation here.
    exponents: [Vec<f64>; MAXFRAMES],
    expind: [usize; MAXFRAMES],
    resized: [bool; MAXFRAMES],
    a_minimums: [f64; MAXFRAMES],
    b_minimums: [f64; MAXFRAMES],
    a_maximums: [f64; MAXFRAMES],
    b_maximums: [f64; MAXFRAMES],
    maxframe: usize,
    frame: usize,

    forcing: [bool; MAXINDEX],
    negative: bool,
    stripe_interval: i32,
    save: bool,
    dorecalc: bool,
    run: bool,
    reset_countdown: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut st = Xlyap {
        gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
        delay: 0,
        linger: 1,
        pixels: Vec::new(),
        maxcolor: MAXCOLOR,
        startcolor: 0,
        mincolindex: 1,
        numcolors: 16,
        numfreecols: 1,
        lowrange: 1,
        dwell: 50,
        settle: 50,
        width: d.width(),
        height: d.height(),
        points: Vec::new(),
        curmap: 0,
        mapindex: 0,
        aflag: false,
        bflag: false,
        wflag: false,
        hflag: false,
        rflag: false,
        maxindex: MAXINDEX,
        min_a: 2.0,
        min_b: 2.0,
        max_a: 4.0,
        max_b: 4.0,
        a_range: 2.0,
        b_range: 2.0,
        a_inc: 0.0,
        b_inc: 0.0,
        a: 2.0,
        b: 2.0,
        start_x: 0.65,
        lyapunov: 0.0,
        minlyap: 1.0,
        minexp: 0.0,
        maxexp: 0.0,
        prob: 0.5,
        useprod: true,
        point: XPoint { x: -1, y: 0 },
        exponents: [Vec::new(), Vec::new()],
        expind: [0; MAXFRAMES],
        resized: [false; MAXFRAMES],
        a_minimums: [0.0; MAXFRAMES],
        b_minimums: [0.0; MAXFRAMES],
        a_maximums: [0.0; MAXFRAMES],
        b_maximums: [0.0; MAXFRAMES],
        maxframe: 0,
        frame: 0,
        forcing: [false; MAXINDEX],
        negative: true,
        stripe_interval: 7,
        save: true,
        dorecalc: false,
        run: true,
        reset_countdown: 0,
    };

    st.do_defaults();
    st.parseargs(d);

    // `-builtin` names one of the stored windows; `-randomize`, which is on,
    // rolls for one instead.
    let builtin = if d.res.bool("randomize") {
        (random() % NBUILTINS) as i32
    } else {
        d.res.string("builtin").trim().parse::<i32>().unwrap_or(-1)
    };
    if builtin >= 0 {
        st.do_preset(builtin as u32);
    }

    st.setupmem();
    st.init_data(d);
    st.init_color();
    st.clear(d);

    st.delay = d.res.int("delay").max(0) as u32;
    st.linger = d.res.int("linger").max(1);
    Box::new(st)
}

impl Xlyap {
    /// `do_defaults`. Note what this does *not* restore: the resources are read
    /// once, in `parseargs`, so after the first picture the hack runs on these
    /// values rather than the ones the panel was set to.
    fn do_defaults(&mut self) {
        self.expind = [0; MAXFRAMES];
        self.resized = [false; MAXFRAMES];

        self.aflag = false;
        self.bflag = false;
        self.hflag = false;
        self.wflag = false;
        self.minexp = 0.0;
        self.mapindex = 0;

        self.maxcolor = 256;
        self.startcolor = 17;
        self.mincolindex = 33;
        self.dwell = 100;
        self.settle = 50;

        self.maxindex = MAXINDEX;
        self.min_a = 2.0;
        self.min_b = 2.0;
        self.a_range = 2.0;
        self.b_range = 2.0;
        self.minlyap = 1.0;
        self.max_a = 4.0;
        self.max_b = 4.0;
        self.numcolors = 16;
        self.prob = 0.5;
        self.negative = true;
        self.stripe_interval = 7;
        self.save = true;
        self.useprod = true;
        self.run = true;

        for (i, f) in self.forcing.iter_mut().enumerate() {
            *f = i % 2 == 1;
        }
    }

    /// `parseargs`, less the options that only fed the parts of the hack this
    /// port leaves out.
    fn parseargs(&mut self, d: &Dpy) {
        self.curmap = 0;

        self.mincolindex = d.res.int("minColor");
        self.dwell = d.res.int("dwell");

        if d.res.bool("useLog") {
            self.useprod = false;
        }

        self.minlyap = d.res.float("colorExponent").abs();
        self.maxexp = self.minlyap;
        self.minexp = -self.minlyap;

        // Upstream aborts on a colour count above its table; clamping is the
        // same picture without the crash, and a count of zero would leave every
        // index out of range.
        self.maxcolor = (d.res.int("maxColor").unsigned_abs() as usize).clamp(1, MAXCOLOR);
        if self.maxcolor as i32 - self.startcolor <= 0 {
            // Upstream's own confusion of a colour index with a pixel value:
            // this assigns the background *colour* to an index. Unreachable at
            // the defaults, where the cap is 256 and the index 17.
            self.startcolor = d.res.pixel("background") as i32;
        }
        if self.maxcolor as i32 - self.mincolindex <= 0 {
            self.mincolindex = 1;
        }

        let force = d.res.string("randomForce").trim().to_string();
        if !force.is_empty() {
            self.prob = force.parse::<f64>().unwrap_or(0.5);
            self.rflag = true;
            self.setforcing();
        }

        self.settle = d.res.int("settle");
        self.min_a = d.res.float("minA");
        self.aflag = true;
        self.min_b = d.res.float("minB");
        self.bflag = true;

        let ff = d.res.string("forcingFunction").trim().to_string();
        self.set_forcing_string(&ff);

        let range = d.res.string("bRange").trim().to_string();
        if !range.is_empty() {
            self.b_range = range.parse::<f64>().unwrap_or(self.b_range);
            self.hflag = true;
        }

        self.start_x = d.res.float("startX");

        let index = d.res.string("mapIndex").trim().to_string();
        if !index.is_empty()
            && let Ok(m) = index.parse::<usize>()
            && m < 5
        {
            self.mapindex = m;
            self.curmap = m;
            self.take_map_ranges();
        }

        if d.res.bool("beNegative") {
            self.negative = false;
        }

        let range = d.res.string("aRange").trim().to_string();
        if !range.is_empty() {
            self.a_range = range.parse::<f64>().unwrap_or(self.a_range);
            self.wflag = true;
        }

        self.max_a = self.min_a + self.a_range;
        self.max_b = self.min_b + self.b_range;

        self.a_minimums[0] = self.min_a;
        self.b_minimums[0] = self.min_b;
        self.a_maximums[0] = self.max_a;
        self.b_maximums[0] = self.max_b;

        // Upstream ends by checking the window against the range its map is
        // defined over, and warns on stderr when it runs off the edge. There is
        // nowhere to warn here and the exponent is computed either way.
    }

    /// The parameter window a map wants, for the parameters the caller has not
    /// pinned. Upstream applies this when `-m` names a map and when the map
    /// key cycles to the next one.
    fn take_map_ranges(&mut self) {
        const AMINS: [f64; 5] = [2.0, 0.0, 0.0, 0.0, 0.0];
        const ARANGES: [f64; 5] = [2.0, 1.0, 6.75, 6.75, 16.0];
        if !self.aflag {
            self.min_a = AMINS[self.mapindex];
        }
        if !self.wflag {
            self.a_range = ARANGES[self.mapindex];
        }
        if !self.bflag {
            self.min_b = AMINS[self.mapindex];
        }
        if !self.hflag {
            self.b_range = ARANGES[self.mapindex];
        }
    }

    /// `-f abbabaab`: which of the two parameters each iteration uses.
    fn set_forcing_string(&mut self, s: &str) {
        if s.is_empty() || s.len() > MAXINDEX || !s.bytes().all(|c| c == b'a' || c == b'b') {
            // Upstream prints its usage message and exits.
            return;
        }
        self.maxindex = s.len();
        for (i, c) in s.bytes().enumerate() {
            self.forcing[i] = c == b'b';
        }
    }

    /// `setforcing`. `random()` is enormous and `prob` is a probability, so at
    /// any sane setting this fills the whole function with 'a'.
    fn setforcing(&mut self) {
        for f in self.forcing.iter_mut() {
            *f = f64::from(random()) <= self.prob;
        }
    }

    /// `setupmem`, and `freemem` with it: the old arrays go when the new ones
    /// are assigned.
    fn setupmem(&mut self) {
        let n = (self.width.max(1) * (self.height.max(1) + 1)) as usize;
        self.exponents = [vec![0.0; n], vec![0.0; n]];
    }

    fn init_data(&mut self, d: &Dpy) {
        self.numcolors = d.res.int("colors");
        if self.numcolors < 2 {
            self.numcolors = 2;
        }
        if self.numcolors > self.maxcolor as i32 {
            self.numcolors = self.maxcolor as i32;
        }
        self.numfreecols = self.numcolors - self.mincolindex;
        self.lowrange = self.mincolindex - self.startcolor;
        self.a_inc = self.a_range / f64::from(self.width);
        self.b_inc = self.b_range / f64::from(self.height);
        self.point = XPoint { x: -1, y: 0 };
        self.a = self.min_a;
        self.b = self.min_b;
        self.init_buffer();
    }

    fn init_color(&mut self) {
        let colors = make_smooth_colormap(self.maxcolor);
        let ncolors = colors.len();
        self.pixels = (0..self.maxcolor)
            .map(|i| {
                let j = ((i as f32 / self.maxcolor as f32) * ncolors as f32) as usize;
                colors[j.min(ncolors - 1)].pixel
            })
            .collect();
    }

    fn init_buffer(&mut self) {
        self.points.resize_with(self.maxcolor, Vec::new);
        for p in self.points.iter_mut() {
            p.clear();
        }
    }

    fn buffer_point(&mut self, d: &mut Dpy, color: i32, x: i32, y: i32) {
        // Upstream calls this paranoia, and it is, but the index is computed
        // from a logarithm and every so often the paranoia pays.
        let color = color.clamp(0, self.maxcolor as i32 - 1) as usize;
        if self.points[color].len() >= MAXPOINTS {
            self.gc.set_foreground(self.pixels[color]);
            d.win().draw_points(&self.gc, &self.points[color]);
            self.points[color].clear();
        }
        self.points[color].push(XPoint { x, y });
    }

    fn flush_buffer(&mut self, d: &mut Dpy) {
        for color in 0..self.points.len() {
            if !self.points[color].is_empty() {
                self.gc.set_foreground(self.pixels[color]);
                d.win().draw_points(&self.gc, &self.points[color]);
                self.points[color].clear();
            }
        }
    }

    /// Here's where we index into a color map. After the Lyapunov exponent is
    /// calculated, it is used to determine what color to use for that point.
    /// If it is non-negative then there is a reserved area at the lower range
    /// of the color map that we index into, in the ratio of the exponent to
    /// some minimum exponent value; if it is negative the same ratio indexes
    /// into the rest of the map. Upstream's note on this is worth keeping: the
    /// indexing algorithm makes as much difference to what you can see in the
    /// picture as the colour map does.
    ///
    /// Returns whether there is any picture left to draw.
    fn sendpoint(&mut self, d: &mut Dpy, expo: f64) -> bool {
        self.point.x += 1;
        let tmpexpo = if self.negative { expo } else { -expo };
        let index = if tmpexpo > 0.0 {
            if self.lowrange != 0 {
                let i = (tmpexpo * f64::from(self.lowrange) / self.maxexp) as i32;
                (i % self.lowrange) + self.startcolor
            } else {
                self.startcolor
            }
        } else if self.numfreecols != 0 {
            // After the first picture `minexp` is zero (upstream's defaults
            // reset it and never run the resource pass again), so this divides
            // by zero and the whole negative half of the plane comes out one
            // flat colour. C leaves the cast of an infinity undefined; Rust
            // saturates, so the flat colour differs from a given C build's.
            let i = (tmpexpo * f64::from(self.numfreecols) / self.minexp) as i32;
            (i % self.numfreecols) + self.mincolindex
        } else {
            self.mincolindex
        };

        let (x, y) = (self.point.x, self.point.y);
        self.buffer_point(d, index, x, y);

        if self.save {
            let f = self.frame;
            let i = self.expind[f];
            if i < self.exponents[f].len() {
                self.exponents[f][i] = expo;
                self.expind[f] = i + 1;
            }
        }
        if self.point.x >= self.width {
            self.point.y += 1;
            self.point.x = 0;
            if self.save {
                self.b += self.b_inc;
                self.a = self.min_a;
            }
            return self.point.y < self.height;
        }
        true
    }

    /// The guts of the program: the Lyapunov exponent of one point of the
    /// parameter plane. For each iteration past `settle` take the logarithm of
    /// the absolute value of the derivative, and average them. Some small
    /// speed up is achieved by utilizing the fact that log(a*b) = log(a) +
    /// log(b).
    ///
    /// Returns whether the picture is finished.
    fn complyap(&mut self, d: &mut Dpy) -> bool {
        if !self.run {
            return true;
        }
        self.a += self.a_inc;
        if self.a >= self.max_a {
            // The end of a row: one more point at the old exponent, which is
            // what wraps `point` onto the next line.
            let last = self.lyapunov;
            if self.sendpoint(d, last) {
                return false;
            }
            self.flush_buffer(d);
            return true;
        }
        if self.b >= self.max_b {
            self.flush_buffer(d);
            return true;
        }

        let mut prod = 1.0f64;
        let mut total = 0.0f64;
        let mut bindex = 0usize;
        let mut x = self.start_x;
        let mut r = if self.forcing[bindex] { self.b } else { self.a };

        // Here's where we let the thing "settle down". There is usually some
        // initial "noise" in the iterations.
        for _ in 0..self.settle {
            x = map(self.curmap, x, r);
            bindex += 1;
            if bindex >= self.maxindex {
                bindex = 0;
                if self.rflag {
                    self.setforcing();
                }
            }
            r = if self.forcing[bindex] { self.b } else { self.a };
        }

        let mut i = 0i32;
        if self.useprod {
            // Using log(a*b).
            while i < self.dwell {
                x = map(self.curmap, x, r);
                let dx = deriv(self.curmap, x, r).abs();
                if dx == 0.0 {
                    // log(0) is nasty so break out.
                    i += 1;
                    break;
                }
                prod *= dx;
                // We need to prevent overflow and underflow.
                if !(1.0e-12..=1.0e12).contains(&prod) {
                    total += prod.ln();
                    prod = 1.0;
                }
                bindex += 1;
                if bindex >= self.maxindex {
                    bindex = 0;
                    if self.rflag {
                        self.setforcing();
                    }
                }
                r = if self.forcing[bindex] { self.b } else { self.a };
                i += 1;
            }
            total += prod.ln();
        } else {
            // Use log(a) + log(b).
            while i < self.dwell {
                x = map(self.curmap, x, r);
                let dx = deriv(self.curmap, x, r).abs();
                if x == 0.0 {
                    i += 1;
                    break;
                }
                total += dx.ln();
                bindex += 1;
                if bindex >= self.maxindex {
                    bindex = 0;
                    if self.rflag {
                        self.setforcing();
                    }
                }
                r = if self.forcing[bindex] { self.b } else { self.a };
                i += 1;
            }
        }
        self.lyapunov = (total * std::f64::consts::LOG2_E) / f64::from(i);

        if self.sendpoint(d, self.lyapunov) {
            return false;
        }
        self.flush_buffer(d);
        true
    }

    fn clear(&mut self, d: &mut Dpy) {
        d.clear_window();
        self.init_buffer();
    }

    /// `Redraw`: start the picture again from the top, recomputing it.
    fn redraw_all(&mut self, d: &mut Dpy) {
        self.flush_buffer(d);
        self.point = XPoint { x: -1, y: 0 };
        self.run = true;
        self.a = self.min_a;
        self.b = self.min_b;
        self.expind[self.frame] = 0;
        self.resized[self.frame] = false;
    }

    /// `redraw`: repaint a stored view from its exponents, without computing
    /// anything. `cont` keeps the position it was interrupted at, so an
    /// unfinished picture carries on where it left off.
    fn redraw(&mut self, d: &mut Dpy, frame: usize, cont: bool) {
        let (x_saved, y_saved) = (self.point.x, self.point.y);
        self.point = XPoint { x: -1, y: 0 };

        self.save = false;
        for i in 0..self.expind[frame] {
            let expo = self.exponents[frame][i];
            self.sendpoint(d, expo);
        }
        self.save = true;

        if cont {
            self.point.x = x_saved;
            self.point.y = y_saved;
        } else {
            self.a = f64::from(self.point.x) * self.a_inc + self.min_a;
            self.b = f64::from(self.point.y) * self.b_inc + self.min_b;
        }
        self.flush_buffer(d);
    }

    /// `recalc`: take the colour range from the exponents actually found,
    /// which pulls detail out of a picture that is nearly all one colour.
    fn recalc(&mut self) {
        self.minexp = 0.0;
        self.maxexp = 0.0;
        for i in 0..self.expind[self.frame] {
            let e = self.exponents[self.frame][i];
            if e < self.minexp {
                self.minexp = e;
            }
            if e > self.maxexp {
                self.maxexp = e;
            }
        }
    }

    fn cycle_frames(&mut self, d: &mut Dpy) {
        for i in 0..=self.maxframe {
            self.redraw(d, i, true);
        }
    }

    fn jumpwin(&mut self, d: &mut Dpy) {
        self.min_a = self.a_minimums[self.frame];
        self.min_b = self.b_minimums[self.frame];
        self.max_a = self.a_maximums[self.frame];
        self.max_b = self.b_maximums[self.frame];
        self.a_range = self.max_a - self.min_a;
        self.b_range = self.max_b - self.min_b;
        self.a_inc = self.a_range / f64::from(self.width);
        self.b_inc = self.b_range / f64::from(self.height);
        self.point = XPoint { x: -1, y: 0 };
        self.a = self.min_a;
        self.b = self.min_b;
        self.clear(d);
        if self.resized[self.frame] {
            self.redraw_all(d);
        } else {
            let frame = self.frame;
            self.redraw(d, frame, false);
        }
    }

    fn go_down(&mut self, d: &mut Dpy) {
        self.frame += 1;
        if self.frame > self.maxframe {
            self.frame = 0;
        }
        self.jumpwin(d);
    }

    fn go_back(&mut self, d: &mut Dpy) {
        self.frame = if self.frame == 0 {
            self.maxframe
        } else {
            self.frame - 1
        };
        self.jumpwin(d);
    }

    fn destroy_frame(&mut self, d: &mut Dpy) {
        for i in self.frame..self.maxframe {
            self.exponents.swap(i, i + 1);
            self.expind[i] = self.expind[i + 1];
            self.a_minimums[i] = self.a_minimums[i + 1];
            self.b_minimums[i] = self.b_minimums[i + 1];
            self.a_maximums[i] = self.a_maximums[i + 1];
            self.b_maximums[i] = self.b_maximums[i + 1];
        }
        // Upstream decrements past zero here and then indexes with it; with
        // the zoom stack gone there is only ever the one view to destroy.
        self.maxframe = self.maxframe.saturating_sub(1);
        self.go_back(d);
    }

    /// `resize`. Upstream reads the geometry back and returns early when it has
    /// not changed, which is what makes calling this on every restart harmless.
    fn resize(&mut self, d: &mut Dpy) {
        let (new_w, new_h) = (d.width(), d.height());
        if new_w == self.width && new_h == self.height {
            return;
        }
        self.width = new_w;
        self.height = new_h;
        d.clear_window();
        self.a_inc = self.a_range / f64::from(self.width);
        self.b_inc = self.b_range / f64::from(self.height);
        self.point = XPoint { x: -1, y: 0 };
        self.run = true;
        self.a = self.min_a;
        self.b = self.min_b;
        self.setupmem();
        for n in 0..MAXFRAMES {
            if n <= self.maxframe && n != self.frame {
                self.resized[n] = true;
            }
        }
        self.init_buffer();
        self.clear(d);
        self.redraw_all(d);
    }

    /// One of the stored parameter windows. Several of these differ only in a
    /// map index that nothing downstream reads: `do_preset` sets the index but
    /// not the map, and only the resource pass and the map key do that, so
    /// nine of the twenty-two draw the logistic map like their neighbours.
    fn do_preset(&mut self, builtin: u32) {
        let mut ff = "";
        match builtin {
            0 => {
                self.min_a = 3.75;
                self.aflag = true;
                self.min_b = 3.299999;
                self.bflag = true;
                self.a_range = 0.05;
                self.wflag = true;
                self.b_range = 0.05;
                self.hflag = true;
                self.dwell = 200;
                self.settle = 100;
                ff = "abaabbaaabbb";
            }
            1 => {
                self.min_a = 3.8;
                self.aflag = true;
                self.min_b = 3.2;
                self.bflag = true;
                self.b_range = 0.05;
                self.hflag = true;
                self.a_range = 0.05;
                self.wflag = true;
                ff = "bbbbbaaaaa";
            }
            2 => {
                self.min_a = 3.4;
                self.aflag = true;
                self.min_b = 3.04;
                self.bflag = true;
                self.a_range = 0.5;
                self.wflag = true;
                self.b_range = 0.5;
                self.hflag = true;
                ff = "abbbbbbbbb";
                self.settle = 500;
                self.dwell = 1000;
            }
            3 => {
                self.min_a = 3.5;
                self.aflag = true;
                self.min_b = 3.0;
                self.bflag = true;
                self.a_range = 0.2;
                self.wflag = true;
                self.b_range = 0.2;
                self.hflag = true;
                self.dwell = 600;
                self.settle = 300;
                ff = "aaabbbab";
            }
            4 => {
                self.min_a = 3.55667;
                self.aflag = true;
                self.min_b = 3.2;
                self.bflag = true;
                self.b_range = 0.05;
                self.hflag = true;
                self.a_range = 0.05;
                self.wflag = true;
                ff = "bbbbbaaaaa";
            }
            5 => {
                self.min_a = 3.79;
                self.aflag = true;
                self.min_b = 3.22;
                self.bflag = true;
                self.b_range = 0.02999;
                self.hflag = true;
                self.a_range = 0.02999;
                self.wflag = true;
                ff = "bbbbbaaaaa";
            }
            6 => {
                self.min_a = 3.7999;
                self.aflag = true;
                self.min_b = 3.299999;
                self.bflag = true;
                self.a_range = 0.2;
                self.wflag = true;
                self.b_range = 0.2;
                self.hflag = true;
                self.dwell = 300;
                self.settle = 150;
                ff = "abaabbaaabbb";
            }
            7 => {
                self.min_a = 3.89;
                self.aflag = true;
                self.min_b = 3.22;
                self.bflag = true;
                self.b_range = 0.028;
                self.hflag = true;
                self.a_range = 0.02999;
                self.wflag = true;
                ff = "bbbbbaaaaa";
                self.settle = 600;
                self.dwell = 1000;
            }
            8 => {
                self.min_a = 3.2;
                self.aflag = true;
                self.min_b = 3.7;
                self.bflag = true;
                self.a_range = 0.05;
                self.wflag = true;
                self.b_range = 0.005;
                self.hflag = true;
                ff = "abbbbaa";
            }
            9 | 10 => {
                ff = "aaaaaabbbbbb";
                self.mapindex = 1;
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            11 => {
                self.mapindex = 1;
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            12 => {
                ff = "abbb";
                self.mapindex = 1;
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            13 => {
                ff = "abbabaab";
                self.mapindex = 1;
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            14 => {
                ff = "abbabaab";
                self.dwell = 800;
                self.settle = 200;
                self.set_exponent_range(0.85);
                self.min_a = 3.91;
                self.aflag = true;
                self.a_range = 0.0899999999;
                self.wflag = true;
                self.min_b = 3.28;
                self.bflag = true;
                self.b_range = 0.35;
                self.hflag = true;
            }
            15 => {
                ff = "aaaaaabbbbbb";
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            16 => {
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            17 => {
                ff = "abbb";
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            18 => {
                ff = "abbabaab";
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            19 => {
                self.mapindex = 2;
                ff = "aaaaaabbbbbb";
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            20 => {
                self.mapindex = 2;
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            21 => {
                self.mapindex = 2;
                ff = "abbb";
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
            _ => {
                self.mapindex = 2;
                ff = "abbabaab";
                self.dwell = 400;
                self.settle = 200;
                self.set_exponent_range(0.85);
            }
        }
        self.set_forcing_string(ff);
    }

    fn set_exponent_range(&mut self, e: f64) {
        self.minlyap = e;
        self.maxexp = e;
        self.minexp = -e;
    }

    /// `Getkey`. Everything the hack can be told to do while it runs, minus the
    /// keys that printed to a terminal or exited.
    fn getkey(&mut self, d: &mut Dpy, key: char) -> bool {
        if self.reset_countdown != 0 {
            self.reset_countdown = self.linger;
        }
        match key {
            '<' => self.dwell = (self.dwell / 2).max(1),
            '>' => self.dwell *= 2,
            '[' => self.settle = (self.settle / 2).max(1),
            ']' => self.settle *= 2,
            'd' => self.go_down(d),
            'D' => self.flush_buffer(d),
            'e' | 'E' => {
                self.flush_buffer(d);
                self.dorecalc = !self.dorecalc;
                if self.dorecalc {
                    self.recalc();
                } else {
                    self.maxexp = self.minlyap;
                    self.minexp = -self.minlyap;
                }
                let frame = self.frame;
                self.redraw(d, frame, true);
            }
            'i' => {
                if self.stripe_interval > 0 {
                    self.stripe_interval -= 1;
                    self.init_color();
                }
            }
            'I' => {
                self.stripe_interval += 1;
                self.init_color();
            }
            'K' => {
                if self.minlyap > 0.05 {
                    self.minlyap -= 0.05;
                }
            }
            'J' => self.minlyap += 0.05,
            'm' => {
                self.mapindex = (self.mapindex + 1) % 5;
                self.curmap = self.mapindex;
                self.take_map_ranges();
                self.max_a = self.min_a + self.a_range;
                self.max_b = self.min_b + self.b_range;
                self.a_minimums[0] = self.min_a;
                self.b_minimums[0] = self.min_b;
                self.a_maximums[0] = self.max_a;
                self.b_maximums[0] = self.max_b;
                self.a_inc = self.a_range / f64::from(self.width);
                self.b_inc = self.b_range / f64::from(self.height);
                self.point = XPoint { x: -1, y: 0 };
                self.a = self.min_a;
                self.b = self.min_b;
                self.clear(d);
            }
            'M' => {
                if self.minlyap > 0.005 {
                    self.minlyap -= 0.005;
                }
            }
            'N' => self.minlyap += 0.005,
            'p' | 'P' => {
                self.negative = !self.negative;
                self.flush_buffer(d);
                let frame = self.frame;
                self.redraw(d, frame, true);
            }
            'r' => {
                self.flush_buffer(d);
                let frame = self.frame;
                self.redraw(d, frame, true);
            }
            'R' => {
                self.flush_buffer(d);
                self.redraw_all(d);
            }
            // 's' halved the length of a colour-wheel spin that has been
            // compiled out since before this hack came to xscreensaver, so
            // upstream's case falls through to the one below it.
            's' | 'u' => self.go_back(d),
            'U' => {
                self.frame = 0;
                self.jumpwin(d);
            }
            // Printed the current settings, and the help.
            'v' | 'V' | '?' | 'h' | 'H' => {}
            // Rerolls the colour map. Upstream picks a wheel out of seven here.
            'w' | 'W' => self.init_color(),
            'x' => self.clear(d),
            'X' => self.destroy_frame(d),
            'z' => {
                self.cycle_frames(d);
                let frame = self.frame;
                self.redraw(d, frame, true);
            }
            // Quit, which a page does not do.
            'q' | 'Q' => {}
            _ => return false,
        }
        true
    }
}

impl Screenhack for Xlyap {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if !self.run && self.reset_countdown != 0 {
            self.reset_countdown -= 1;
            if self.reset_countdown != 0 {
                return 1_000_000;
            }
            self.do_defaults();
            self.do_preset(random() % NBUILTINS);
            self.clear(d);
            self.init_data(d);
            self.init_color();
            self.resize(d);
            self.frame = 0;
            self.run = true;
        }

        for _ in 0..2000 {
            if self.complyap(d) {
                self.run = false;
                self.reset_countdown = self.linger;
                break;
            }
        }
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, _width: i32, _height: i32) {
        self.resize(d);
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        if let XEvent::KeyPress { key } = event
            && self.getkey(d, *key)
        {
            return true;
        }
        if screenhack_event_helper(event) {
            self.clear(d);
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background:         black",
    ".foreground:         white",
    "*fpsSolid:		true",
    "*randomize:          true",
    "*builtin:            -1",
    "*minColor:           1",
    "*maxColor:           256",
    "*dwell:              50",
    "*useLog:             false",
    "*colorExponent:      1.0",
    "*colorOffset:        0",
    "*randomForce:        ",
    "*settle:             50",
    "*minA:               2.0",
    "*minB:               2.0",
    "*wheels:             7",
    "*function:           10101010",
    "*forcingFunction:    abbabaab",
    "*bRange:             ",
    "*startX:             0.65",
    "*mapIndex:           ",
    "*outputFile:         ",
    "*beNegative:         false",
    "*rgbMax:             65000",
    "*spinLength:         256",
    "*show:               false",
    "*aRange:             ",
    "*delay:              10000",
    "*linger:             5",
    "*colors:             200",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("linger", "Linger", 0.0, 10.0, 1.0, 0, "5"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "xlyap",
    label: "XLyap",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Ron Record",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=5MrEaXnhEPg"),
        blurb: "The Lyapunov exponent makes pretty fractal pictures.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
