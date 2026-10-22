//! Port of `hacks/droste.c`.
//!
//! ```text
//! xscreensaver, Copyright © 2023-2025 Jamie Zawinski <jwz@jwz.org>
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
//! Mise en abyme: a picture that contains itself, spiralling inward forever.
//! Escher's trick, and it is four steps of complex arithmetic per pixel. Take
//! the pixel's position as a complex number, take its logarithm, which turns
//! the plane into a strip and every ring around the origin into a horizontal
//! line. Rotate the strip by the one angle that makes the tiling seamless, tile
//! it, then take the exponential to wrap it back into a plane. What was a
//! straight repeat along the strip comes back as a spiral, and the picture
//! feeds into itself. The zoom creeps a little every frame, so the spiral is
//! forever falling into its own middle.
//!
//! The angle is not free: the strip has to be sheared by exactly
//! `atan(log(r2/r1) / 2pi)` for one turn around the origin to land on one tile
//! width, and the sign of it is which way the spiral winds.
//!
//! Upstream ships two implementations of the same picture. This is the plain
//! one, written against C99 complex numbers, rather than Dave Odell's threaded
//! version built on logarithm, arctangent and sine tables. There are no
//! threads to feed here, and the tables buy nothing without them.
//!
//! One difference forced by the runtime: upstream loads the picture into a
//! pixmap two and a half times the size of the window, so the deep zoom at the
//! outer edge has resolution left to show. The image channel here delivers a
//! window-sized picture, so that is what the spiral is built from.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::ALPHA;
use crate::runtime::{
    About, Dpy, ImageLoad, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, XImage, random,
    screenhack_event_helper,
};

/// Zoom in a bit to hide the outermost edge of the spiral. If it goes too
/// large or too negative, we just get tiles, not spirals.
const DEF_ZOOM: f64 = 0.3;
const ZOOM_SCALE: f64 = 66.0;

/// Approximately the smallest magnitude double.
const TINY: f64 = 4.94e-324;

/// The per-frame constants, shared by every pixel.
#[derive(Default)]
struct Frame {
    scale: f64,
    /// The shear that makes one turn of the spiral land on one tile.
    i0_re: f64,
    i0_im: f64,
    i0_den: f64,
    ixr: f64,
    iyr: f64,
    oxr: f64,
    oyr: f64,
}

struct State {
    delay: u32,
    duration: f64,
    start_time: f64,
    r1: f64,
    r2: f64,
    zoom: f64,
    speed: f64,
    angle_sign: f64,
    /// The picture the spiral is made of.
    input: Option<XImage>,
    frame: Frame,
    img_loader: Option<ImageLoad>,
    loading: bool,
}

impl State {
    fn start_load(&mut self, d: &mut Dpy) {
        self.img_loader = d.load_image_async_simple(None);
        self.loading = true;
        self.start_time = d.time;
        if self.img_loader.is_none() {
            self.image_arrived(d);
        }
    }

    fn image_arrived(&mut self, d: &mut Dpy) {
        let (w, h) = (d.width(), d.height());
        self.input = Some(d.win_ref().sub_image(0, 0, w, h));
        self.start_time = d.time;
        self.loading = false;
        self.angle_sign = if random() & 1 != 0 { 1.0 } else { -1.0 };
        self.zoom = DEF_ZOOM;
    }

    /// The cached values shared by every pixel of one frame.
    fn frame_init(&mut self, d: &Dpy) {
        let Some(input) = &self.input else { return };
        let (iw, ih) = (input.width(), input.height());
        let (ow, oh) = (d.width().max(1), d.height().max(1));

        let iaspect = iw as f64 / ih.max(1) as f64;
        let oaspect = ow as f64 / oh as f64;

        // Fill the output rect with the input image, with no black margins.
        let (mut ixr, mut iyr) = if iaspect > oaspect {
            (oaspect / iaspect, 1.0)
        } else {
            (1.0, iaspect / oaspect)
        };

        // Since the spiral cut implicitly zooms in a bunch, favoring the
        // center of the image, zoom out a bit so that more of the image shows.
        // At extreme radii and aspect ratios this might show the image edges.
        let s = if iaspect == oaspect {
            0.7
        } else {
            0.8 * iaspect.max(oaspect)
        };
        ixr *= s;
        iyr *= s;

        // Make the spiral have a 1:1 aspect ratio, instead of being a
        // horizontal oval for landscape images and a vertical one for
        // portrait, then back that out of the input so the underlying picture
        // is not stretched by the correction.
        let (oxr, oyr) = (oaspect, 1.0);
        ixr /= oxr;
        iyr /= oyr;

        let scale = (self.r2 / self.r1).ln();
        let angle = (scale / std::f64::consts::TAU).atan() * self.angle_sign;
        // i0 = cexp(i angle) * cos(angle).
        let (sin_a, cos_a) = angle.sin_cos();
        let i0_re = cos_a * cos_a;
        let i0_im = sin_a * cos_a;

        self.frame = Frame {
            scale,
            i0_re,
            i0_im,
            i0_den: i0_re * i0_re + i0_im * i0_im,
            ixr,
            iyr,
            oxr,
            oyr,
        };
    }

    fn render(&mut self, d: &mut Dpy) {
        let Some(input) = self.input.take() else {
            return;
        };
        let f = &self.frame;
        let (iw, ih) = (input.width(), input.height());
        let (ow, oh) = (d.width(), d.height());
        let (zoom, r1) = (self.zoom, self.r1);

        for oy in 0..oh {
            for ox in 0..ow {
                // The pixel's position, as a complex number, zoomed.
                let mut zr = ((ox as f64 / ow as f64) - 0.5) * f.oxr * zoom;
                let mut zi = ((oy as f64 / oh as f64) - 0.5) * f.oyr * zoom;

                // clog: tile strips to ordinary space. `log(cabs(z))` is half
                // the log of the squared magnitude, which saves the root.
                let (lr, li) = (0.5 * (zr * zr + zi * zi).ln(), zi.atan2(zr));

                // Scale and rotate strips: divide by i0.
                zr = (lr * f.i0_re + li * f.i0_im) / f.i0_den;
                zi = (li * f.i0_re - lr * f.i0_im) / f.i0_den;

                // Tile strips. Upstream's own comment calls for GLSL's mod,
                // which is the floor-based one, and that is what its shipping
                // implementation does.
                zr -= f.scale * (zr / f.scale).floor();

                // Annulus to strip.
                let m = zr.exp() * r1;
                let (s, c) = zi.sin_cos();
                let (zr, zi) = (m * c, m * s);

                // [-0.5, 0.5] => [0, WH]
                let src_x = (iw as f64 * (zr * f.ixr + 0.5)) as i32;
                let src_y = (ih as f64 * (zi * f.iyr + 0.5)) as i32;

                let p = if src_x < 0 || src_y < 0 || src_x >= iw || src_y >= ih {
                    ALPHA // Clip.
                } else {
                    input.get_pixel(src_x, src_y)
                };
                d.win().put_pixel(ox, oy, p);
            }
        }

        self.input = Some(input);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut st = State {
        delay: d.res.int("delay").max(0) as u32,
        duration: d.res.int("duration").max(1) as f64,
        start_time: 0.0,
        r1: d.res.float("r1").clamp(0.0, 1.0),
        r2: d.res.float("r2").clamp(0.0, 1.0),
        zoom: DEF_ZOOM,
        speed: d.res.float("speed").min(ZOOM_SCALE - 1.0),
        angle_sign: if random() & 1 != 0 { 1.0 } else { -1.0 },
        input: None,
        frame: Frame::default(),
        img_loader: None,
        loading: false,
    };
    st.start_load(d);
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.loading {
            self.img_loader = d.load_image_async_simple(self.img_loader.take());
            if self.img_loader.is_none() {
                self.image_arrived(d);
            }
            return self.delay;
        }

        if self.input.is_none() || self.start_time + self.duration < d.time {
            self.start_load(d);
            return self.delay;
        }

        self.zoom *= 1.0 - ((1.0 / ZOOM_SCALE) * self.speed);
        if !self.zoom.is_finite() || self.zoom.abs() <= TINY {
            self.zoom = DEF_ZOOM; // Reset is our only option.
        }

        self.frame_init(d);
        self.render(d);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, _width: i32, _height: i32) {
        // Load a new image shortly after resizing stops, to avoid starting a
        // zillion image loaders as the resize events flood in.
        if self.start_time > 0.0 {
            self.start_time = d.time - self.duration + 0.25;
        }
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if let XEvent::KeyPress { key } = event {
            match key {
                '-' | '_' | '<' | ',' => {
                    self.speed -= 0.1;
                    return true;
                }
                '=' | '+' | '>' | '.' => {
                    self.speed += 0.1;
                    return true;
                }
                _ => {}
            }
        }
        if screenhack_event_helper(event) {
            self.start_time = f64::NEG_INFINITY;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    "*fpsSolid: true",
    ".background: Black",
    ".foreground: #BEBEBE",
    "*delay: 20000",
    "*duration: 120",
    "*r1: 0.2",
    "*r2: 0.7",
    "*speed: 1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("duration", "Duration", 10.0, 600.0, 10.0, 0, "120"),
    Opt::slider("speed", "Zoom speed", -10.0, 10.0, 0.5, 1, "1.0"),
    Opt::slider("r1", "Radius one", 0.0, 1.0, 0.05, 2, "0.2"),
    Opt::slider("r2", "Radius two", 0.0, 1.0, 0.05, 2, "0.7"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "droste",
    label: "Droste",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2023",
        video: Some("https://www.youtube.com/watch?v=C0ZXw3gG70c"),
        blurb: "Mise en abyme: infinite spiral recursion within images.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
