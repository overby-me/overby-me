//! Port of `hacks/goop.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1997-2008 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! This is pretty compute-intensive, probably due to the large number of
//! polygon fills.  I tried introducing a scaling factor to make the spline
//! code emit fewer line segments, but that made the edges very rough.
//! However, tuning *maxVelocity, *elasticity and *delay can result in much
//! smoother looking animation.
//!
//! The more planes the better -- SGIs have a 12-bit pseudocolor display
//! (4096 colormap cells) which is mostly useless, except for this program,
//! where it means you can have 11 or 12 mutually-transparent objects instead
//! of only 7 or 8.
//!
//! Oh, for an alpha channel... maybe I should rewrite this in GL.  Then the
//! blobs could have thickness, and curved edges with specular reflections...
//! ```
//!
//! Blobs are closed splines whose control points sit on rays from a centre,
//! each ray breathing in and out on its own, so the outline wobbles like
//! something in a lava lamp. Each blob lives on its own plane of the
//! framebuffer, so where two overlap the bits add and you see through one to
//! the other.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::hsv_to_rgb;
use crate::runtime::spline::Spline;
use crate::runtime::{
    About, Dpy, GXFunc, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, SelectItem,
    StartArgs, frand, random, random_below,
};

/// Fixed-point math, for sub-pixel motion.
const SCALE: i64 = 10000;
/// When planes and count are 0, how many blobs.
const DEF_COUNT: usize = 12;

fn rand_below(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    (((random() as i64) << 15 | (random() as i64 & 0x7fff)) & 0x7fff_ffff) % n
}

fn randsign() -> i64 {
    if random() & 1 == 1 { 1 } else { -1 }
}

struct Blob {
    /// Position of the midpoint, and velocity.
    x: i64,
    y: i64,
    dx: i64,
    dy: i64,
    /// Rotational speed, and the angle of rotation.
    torque: f64,
    th: f64,
    /// How fast they deform, and the speed limit.
    elasticity: i64,
    max_velocity: i64,
    min_r: i64,
    max_r: i64,
    /// One radius per control point. Its sign is the direction it is going.
    r: Vec<i64>,
    spline: Spline,
}

impl Blob {
    fn make(
        maxx: i64,
        maxy: i64,
        size: i64,
        torque: f64,
        elasticity: f64,
        max_velocity: f64,
    ) -> Self {
        let maxx = maxx * SCALE;
        let maxy = maxy * SCALE;
        let size = size * SCALE;

        let max_r = size / 2;
        let min_r = (size / 10).max(5 * SCALE);
        let mid = (min_r + max_r) / 2;

        let elasticity = (SCALE as f64 * elasticity) as i64;
        let max_velocity = (SCALE as f64 * max_velocity) as i64;

        let npoints = (random_below(5) + 5) as usize;
        let r = (0..npoints)
            .map(|_| (rand_below(mid.max(1)) + mid / 2) * randsign())
            .collect();

        Self {
            x: rand_below(maxx),
            y: rand_below(maxy),
            dx: rand_below(max_velocity.max(1)) * randsign(),
            dy: rand_below(max_velocity.max(1)) * randsign(),
            torque,
            th: frand(std::f64::consts::PI * 2.0) * randsign() as f64,
            elasticity,
            max_velocity,
            min_r,
            max_r,
            r,
            spline: Spline::new(npoints),
        }
    }

    fn throb(&mut self) {
        let npoints = self.r.len();
        let frac = (std::f64::consts::PI * 2.0) / npoints as f64;
        for i in 0..npoints {
            let r = self.r[i];
            let mut ra = r.abs();
            let th = self.th.abs();

            // Place control points evenly around the perimeter, shifted by
            // theta.
            let x = self.x + (ra as f64 * (i as f64 * frac + th).cos()) as i64;
            let y = self.y + (ra as f64 * (i as f64 * frac + th).sin()) as i64;
            self.spline.control_x[i] = (x / SCALE) as f64;
            self.spline.control_y[i] = (y / SCALE) as f64;

            // Alter the radius by a random amount, in the direction it had
            // been going; the sign of the radius is that direction.
            ra += rand_below(self.elasticity.max(1)) * if r > 0 { 1 } else { -1 };
            let mut r = ra * if r >= 0 { 1 } else { -1 };

            // If we have reached the end, too long or too short, reverse.
            if (ra > self.max_r && r >= 0) || (ra < self.min_r && r < 0) {
                r = -r;
            } else if random_below(50) == 0 {
                // And reverse in mid-course once every fifty times.
                r = -r;
            }
            self.r[i] = r;
        }
    }

    fn move_it(&mut self, maxx: i64, maxy: i64) {
        let maxx = maxx * SCALE;
        let maxy = maxy * SCALE;
        self.x += self.dx;
        self.y += self.dy;

        // If we have reached the edge of the box, reverse direction.
        if (self.x > maxx && self.dx >= 0) || (self.x < 0 && self.dx < 0) {
            self.dx = -self.dx;
        }
        if (self.y > maxy && self.dy >= 0) || (self.y < 0 && self.dy < 0) {
            self.dy = -self.dy;
        }

        // Alter velocity randomly, then throttle it.
        if random_below(10) == 0 {
            self.dx += rand_below((self.max_velocity / 2).max(1)) * randsign();
            self.dy += rand_below((self.max_velocity / 2).max(1)) * randsign();
            if self.dx > self.max_velocity || self.dx < -self.max_velocity {
                self.dx /= 2;
            }
            if self.dy > self.max_velocity || self.dy < -self.max_velocity {
                self.dy /= 2;
            }
        }

        let mut th = self.th;
        let d = if self.torque == 0.0 {
            0.0
        } else {
            frand(self.torque)
        };
        if th < 0.0 {
            th = -(th + d);
        } else {
            th += d;
        }
        let tau = std::f64::consts::PI * 2.0;
        if th > tau {
            th -= tau;
        } else if th < 0.0 {
            th += tau;
        }
        self.th = if self.th > 0.0 { th } else { -th };

        // Alter the direction of rotation randomly.
        if random_below(100) == 0 {
            self.th *= -1.0;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Transparent,
    Opaque,
    Xor,
}

struct Layer {
    blobs: Vec<Blob>,
    /// The colour in opaque mode; the plane mask in transparent mode.
    pixel: Pixel,
}

struct State {
    mode: Mode,
    width: i32,
    height: i32,
    layers: Vec<Layer>,
    background: Pixel,
    /// Where the frame is built, so nothing half-drawn reaches the screen.
    pixmap: Pixmap,
    /// A one-bit drawable, for exclusive-or mode.
    bitmap: Pixmap,
    gc: Gc,
    window_gc: Gc,
    additive_p: bool,
    delay: u32,
    thickness: i32,
}

impl State {
    fn draw_blob(pixmap: &mut Pixmap, gc: &Gc, b: &mut Blob, fill_p: bool) {
        b.spline.compute_closed();
        if fill_p {
            pixmap.fill_polygon(gc, &b.spline.points);
        } else {
            pixmap.draw_lines(gc, &b.spline.points);
        }
    }

    /// Draw every blob of one layer, then advance them.
    fn draw_layer_blobs(&mut self, li: usize, gc: &Gc, fill_p: bool) {
        let (w, h) = (self.width as i64, self.height as i64);
        for i in 0..self.layers[li].blobs.len() {
            let b = &mut self.layers[li].blobs[i];
            Self::draw_blob(&mut self.pixmap, gc, b, fill_p);
            b.throb();
            b.move_it(w, h);
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (width, height) = (d.width(), d.height());
    let background = d.res.pixel("background");
    let foreground = d.res.pixel("foreground");

    let mode = match d.res.string("mode").to_ascii_lowercase().as_str() {
        "opaque" => Mode::Opaque,
        "xor" => Mode::Xor,
        _ => Mode::Transparent,
    };

    let mut nlayers = d.res.int("planes").max(1) as usize;
    // Upstream asks X for one plane per layer. The planes here are cut out of
    // the framebuffer instead: layer i takes one bit of one channel, most
    // significant first, so a blob contributes a pure red, green or blue and
    // overlapping blobs add. Twenty-four bits means twenty-four layers.
    if mode == Mode::Transparent {
        nlayers = nlayers.min(24);
    }

    let nblobs = d.res.int("count");
    let mut lblobs = vec![0usize; nlayers];
    if nblobs <= 0 {
        let mut total = DEF_COUNT;
        while total > 0 {
            for slot in lblobs.iter_mut() {
                if total == 0 {
                    break;
                }
                *slot += 1;
                total -= 1;
            }
        }
    }

    let torque = d.res.float("torque");
    let elasticity = d.res.float("elasticity");
    let max_velocity = d.res.float("maxVelocity");

    let blob_max = {
        let m = width.min(height) as i64 / 2;
        // Tiny window.
        if width < 100 || height < 100 {
            m * 10
        } else {
            m
        }
    };
    let blob_min = (blob_max * 2) / 3;

    let layers: Vec<Layer> = (0..nlayers)
        .map(|i| {
            let n = if nblobs > 0 {
                nblobs as usize
            } else {
                lblobs[i]
            };
            let blobs = (0..n)
                .map(|_| {
                    let j = blob_max - blob_min;
                    let size = if j != 0 { rand_below(j) } else { 0 } + blob_min;
                    Blob::make(
                        width as i64,
                        height as i64,
                        size,
                        torque,
                        elasticity,
                        max_velocity,
                    )
                })
                .collect();

            let pixel = if mode == Mode::Transparent {
                1u32 << ((i % 3) * 8 + (7 - (i / 3)))
            } else {
                // Hue anywhere, but never washed out or dark.
                let h = random_below(360);
                let s = (random_below(70) as f64 + 30.0) / 100.0;
                let v = (random_below(34) as f64 + 66.0) / 100.0;
                let (r, g, b) = hsv_to_rgb(h, s, v);
                crate::runtime::color::rgb((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8)
            };
            Layer { blobs, pixel }
        })
        .collect();

    let mut gc = Gc::new(foreground, background);
    let thickness = d.res.int("thickness").max(1);
    gc.set_line_width(thickness);

    Box::new(State {
        mode,
        width,
        height,
        layers,
        background,
        pixmap: Pixmap::new(width, height),
        bitmap: Pixmap::new_bitmap(width, height),
        gc,
        window_gc: Gc::new(foreground, background),
        additive_p: d.res.bool("additive"),
        delay: d.res.int("delay").max(0) as u32,
        thickness,
    })
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let (w, h) = (self.width, self.height);
        match self.mode {
            Mode::Transparent => {
                // Upstream first runs every layer through draw_layer_plane,
                // which advances the blobs and draws them into a one-bit
                // pixmap that nothing ever reads. Only the advancing shows, so
                // that is all that is kept: the blobs step twice a frame.
                let (bw, bh) = (w as i64, h as i64);
                for l in &mut self.layers {
                    for b in &mut l.blobs {
                        b.throb();
                        b.move_it(bw, bh);
                    }
                }

                self.gc.set_function(GXFunc::Copy);
                self.gc.set_plane_mask(!0);
                self.gc.set_foreground(self.background);
                self.pixmap.fill_rectangle(&self.gc, 0, 0, w, h);
                self.gc.set_foreground(!0);

                if !self.additive_p {
                    // Subtractive: light every plane inside the blobs, then
                    // have each layer take its own back out again.
                    for li in 0..self.layers.len() {
                        for i in 0..self.layers[li].blobs.len() {
                            let gc = self.gc.clone();
                            let b = &mut self.layers[li].blobs[i];
                            Self::draw_blob(&mut self.pixmap, &gc, b, true);
                        }
                    }
                    self.gc.set_function(GXFunc::Clear);
                }

                for li in 0..self.layers.len() {
                    let mut gc = self.gc.clone();
                    gc.set_plane_mask(self.layers[li].pixel);
                    self.draw_layer_blobs(li, &gc, true);
                }

                let pm = std::mem::replace(&mut self.pixmap, Pixmap::new(1, 1));
                d.win().copy_area(&self.window_gc, &pm, 0, 0, w, h, 0, 0);
                self.pixmap = pm;
            }

            Mode::Xor => {
                let mut clear = Gc::new(0, 0);
                clear.set_line_width(self.thickness);
                self.bitmap.fill_rectangle(&clear, 0, 0, w, h);
                let mut gc = Gc::new(1, 0);
                gc.set_line_width(self.thickness);
                gc.set_function(GXFunc::Xor);

                let (bw, bh) = (w as i64, h as i64);
                for li in 0..self.layers.len() {
                    for i in 0..self.layers[li].blobs.len() {
                        let b = &mut self.layers[li].blobs[i];
                        Self::draw_blob(&mut self.bitmap, &gc, b, true);
                        b.throb();
                        b.move_it(bw, bh);
                    }
                }

                let bm = std::mem::replace(&mut self.bitmap, Pixmap::new_bitmap(1, 1));
                d.win().copy_plane(&self.window_gc, &bm, 0, 0, w, h, 0, 0);
                self.bitmap = bm;
            }

            Mode::Opaque => {
                self.gc.set_function(GXFunc::Copy);
                self.gc.set_plane_mask(!0);
                self.gc.set_foreground(self.background);
                self.pixmap.fill_rectangle(&self.gc, 0, 0, w, h);
                for li in 0..self.layers.len() {
                    let mut gc = self.gc.clone();
                    gc.set_foreground(self.layers[li].pixel);
                    self.draw_layer_blobs(li, &gc, true);
                }
                let pm = std::mem::replace(&mut self.pixmap, Pixmap::new(1, 1));
                d.win().copy_area(&self.window_gc, &pm, 0, 0, w, h, 0, 0);
                self.pixmap = pm;
            }
        }
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
            self.pixmap = Pixmap::new(width, height);
            self.bitmap = Pixmap::new_bitmap(width, height);
        }
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: yellow",
    "*delay: 12000",
    "*additive: true",
    "*mode: transparent",
    "*count: 1",
    "*planes: 12",
    "*thickness: 5",
    "*torque: 0.0075",
    "*elasticity: 0.9",
    "*maxVelocity: 0.5",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "transparent",
        label: "Transparent blobs",
    },
    SelectItem {
        value: "opaque",
        label: "Opaque blobs",
    },
    SelectItem {
        value: "xor",
        label: "XOR blobs",
    },
];

const COLOR_MODE: &[SelectItem] = &[
    SelectItem {
        value: "true",
        label: "Additive colors (transmitted light)",
    },
    SelectItem {
        value: "false",
        label: "Subtractive colors (reflected light)",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "12000").inverted(),
    Opt::slider("torque", "Speed", 0.0002, 0.05, 0.0002, 4, "0.0075"),
    Opt::slider("planes", "Blobs", 1.0, 50.0, 1.0, 0, "12"),
    Opt::slider("elasticity", "Elasticity", 0.1, 5.0, 0.1, 1, "0.9"),
    Opt::slider("maxVelocity", "Speed limit", 0.1, 3.0, 0.1, 1, "0.5"),
    Opt::select("mode", "Mode", MODES, "transparent"),
    Opt::select("additive", "Color mode", COLOR_MODE, "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "goop",
    label: "Goop",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=bLMAF4Q-mGA"),
        blurb: "Translucent amoeba-like blobs wander the screen.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
