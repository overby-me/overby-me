//! Port of `hacks/glitchpeg.c`.
//!
//! ```text
//! glitchpeg, Copyright (c) 2018-2021 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Insert errors into an image file, then display the corrupted result.
//!
//! This only works on X11 and MacOS because iOS and Android don't have
//! access to the source files of images, only the decoded image data.
//! ```
//!
//! The last paragraph is this port's problem too: a picture arrives here
//! already decoded, from the browser or from the compiled-in test card, and
//! there is no file behind it. So [`crate::runtime::jpeg`] makes one. The
//! image is encoded to a JPEG once, and after that the hack does what upstream
//! does: copy the bytes, damage a few hundred of them at random, and show
//! whatever the decoder makes of the result.
//!
//! The damage is worth understanding, because it is why the picture smears
//! rather than speckles. A byte in the entropy-coded data is a piece of a
//! Huffman code, so changing it changes the *length* of everything after it:
//! the decoder carries on reading, but out of step, and every block from there
//! on is built from bits that meant something else. A damaged DC coefficient
//! shifts the colour of its whole block and of every block after it in that
//! component, which is the sliding colour that gives the effect its name.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::{
    About, Dpy, Fb, ImageLoad, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent, jpeg,
    random, screenhack_event_helper,
};

/// The longest side of the JPEG this makes, in pixels.
///
/// Upstream corrupts whatever file it found, which for a photograph is usually
/// a few thousand pixels across, and hands it to a decoder written to be fast.
/// This decoder is written to be read, and it runs once a frame, so the file it
/// is given is a smaller one. The glitch does not care: it lands on the same
/// eight-pixel blocks either way, and the result is drawn scaled to the window
/// as upstream's is.
const MAX_SIDE: i32 = 1024;

/// How hard the JPEG is squeezed. A file from a camera is around here, and the
/// quantisation is what decides how much a damaged coefficient moves.
const QUALITY: i32 = 90;

/// The size of file upstream's count was chosen for.
///
/// `count` is a number of byte errors, and upstream puts them into whatever
/// photograph it found, which is a megabyte or so. The file made here is a
/// fraction of that, and the same number of errors in it would be many times
/// the damage: enough to leave nothing recognisable. So the count is taken as
/// a proportion of this, which is what makes the slider mean the same thing.
const REFERENCE_SIZE: f64 = 1_000_000.0;

struct GlitchPeg {
    delay: u32,
    duration: f64,
    count: i32,
    /// When the current file was made, so it can be replaced.
    start_time: f64,
    /// The file itself.
    image_data: Vec<u8>,
    /// Where the picture is collected while the host is fetching it.
    canvas: Pixmap,
    load: Option<ImageLoad>,
    loading: bool,
    button_down_p: bool,
}

impl GlitchPeg {
    /// Ask for a picture, and turn it into a file once it lands.
    fn poll_image(&mut self, d: &mut Dpy) {
        let mut canvas = std::mem::replace(&mut self.canvas, Pixmap::new(1, 1));
        self.load = d.load_image_into(&mut canvas, self.load.take());
        self.canvas = canvas;
        if self.load.is_some() {
            return;
        }

        // Down to something this decoder can get through in a frame.
        let (w, h) = (self.canvas.width(), self.canvas.height());
        let scale = f64::from(MAX_SIDE) / f64::from(w.max(h));
        let source = if scale < 1.0 {
            let (sw, sh) = (
                ((f64::from(w) * scale) as i32).max(1),
                ((f64::from(h) * scale) as i32).max(1),
            );
            let mut small = Fb::new(sw, sh);
            for y in 0..sh {
                for x in 0..sw {
                    let p = self
                        .canvas
                        .get_pixel((f64::from(x) / scale) as i32, (f64::from(y) / scale) as i32);
                    small.put_pixel(x, y, p);
                }
            }
            small
        } else {
            std::mem::replace(&mut self.canvas, Pixmap::new(1, 1))
        };

        self.image_data = jpeg::encode(&source, QUALITY);
        self.canvas = source;
        self.start_time = d.time;
        self.loading = false;
    }

    /// Copy the file and break it.
    fn glitch(&self) -> Vec<u8> {
        let mut glitched = self.image_data.clone();
        let density = self.image_data.len() as f64 / REFERENCE_SIZE;
        let mut nn = (f64::from((random() % self.count.max(1) as u32) as i32) * density) as i32;
        if nn <= 0 {
            nn = 1;
        }
        if random().is_multiple_of(30) {
            nn *= 20;
        }

        let start = 255;
        let end = glitched.len() as i32 - 255;
        let size = end - start;
        if size <= 100 {
            return glitched;
        }

        for _ in 0..nn {
            let i = (start + (random() % size as u32) as i32) as usize;
            if random().is_multiple_of(10) {
                /* Take one random byte and randomize it. */
                glitched[i] = (random() % 0xFF) as u8;
            } else {
                /* Take one random byte and add 5% to it. */
                let delta = (1 + (random() % 0x0C) as i32) * if random() & 1 != 0 { 1 } else { -1 };
                glitched[i] = (i32::from(glitched[i]) + delta) as u8;
            }
        }
        glitched
    }
}

/// Renders a scaled, cropped version of the image onto the window.
///
/// Upstream reads the source from the bottom up, so the picture it puts on the
/// screen is upside down. That is not a rounding error or an artefact of some
/// image format's row order: it is what the arithmetic says, and it is kept.
fn draw_image(d: &mut Dpy, image: &Fb) {
    let (w, h) = (d.width(), d.height());
    let (iw, ih) = (image.width(), image.height());
    if iw <= 0 || ih <= 0 {
        return;
    }

    let xs = f64::from(iw) / f64::from(w.max(1));
    let ys = f64::from(ih) / f64::from(h.max(1));
    let s = xs.min(ys);
    let w2 = (f64::from(iw) / s) as i32;
    let h2 = (f64::from(ih) / s) as i32;
    let xoff = (w - w2) / 2;
    let yoff = (h - h2) / 2;

    let win = d.win();
    for y in 0..h {
        let iy = ((f64::from(h - y - yoff - 1)) * s) as i32;
        for x in 0..w {
            let ix = (f64::from(x - xoff) * s) as i32;
            let p = if ix >= 0 && ix < iw && iy >= 0 && iy < ih {
                image.get_pixel(ix, iy)
            } else {
                crate::runtime::color::BLACK
            };
            win.put_pixel(x, y, p);
        }
    }
}

impl Screenhack for GlitchPeg {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if (self.image_data.is_empty() || d.time >= self.start_time + self.duration)
            && !self.loading
        {
            /* Time to reload */
            self.loading = true;
            self.load = None;
        }

        if self.loading {
            self.poll_image(d);
        }

        if !self.image_data.is_empty() && !self.button_down_p {
            let glitched = self.glitch();
            /* Might be nothing at all if the header was the part that broke. */
            if let Some(image) = jpeg::decode(&glitched) {
                draw_image(d, &image);
            }
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, _width: i32, _height: i32) {}

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        match event {
            XEvent::ButtonPress { .. } => {
                self.button_down_p = true;
                true
            }
            XEvent::ButtonRelease { .. } => {
                self.button_down_p = false;
                true
            }
            e if screenhack_event_helper(e) => {
                self.start_time = 0.0; /* reload */
                true
            }
            _ => false,
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(GlitchPeg {
        delay: d.res.int("delay").max(1) as u32,
        duration: f64::from(d.res.int("duration").max(0)),
        count: d.res.int("count").max(1),
        start_time: 0.0,
        image_data: Vec::new(),
        canvas: Pixmap::new(d.width().max(1), d.height().max(1)),
        load: None,
        loading: true,
        button_down_p: false,
    })
}

const DEFAULTS: &[&str] = &[
    ".background:		black",
    ".foreground:		white",
    ".lowrez:                   True",
    "*fpsSolid:			true",
    "*delay:			30000",
    "*duration:			120",
    "*count:			400",
    "*grabDesktopImages:	False",
    "*chooseRandomImages:	True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("duration", "Duration", 1.0, 600.0, 1.0, 0, "120"),
    Opt::slider("count", "Glitchiness", 1.0, 1024.0, 1.0, 0, "400"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glitchpeg",
    label: "GlitchPEG",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2018",
        video: Some("https://www.youtube.com/watch?v=Xl5vKJ65_xM"),
        blurb: "Corrupts an image file and shows what the decoder makes of the wreckage.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
