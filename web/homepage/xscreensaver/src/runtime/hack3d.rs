//! The contract the OpenGL savers are written against.
//!
//! The same shape as [`crate::runtime::Runner`] and for the same reasons, with
//! the framebuffer swapped for a [`Glx`]: `init` builds the hack, `draw`
//! returns how many microseconds it would like to wait, `reshape` and `event`
//! do what they say. Upstream's are `init_NAME`, `draw_NAME`, `reshape_NAME`
//! and `NAME_handle_event`, reached through `xlockmore.h`'s `ModeInfo`.
//!
//! A frame does not go anywhere on its own. The hack draws into the `Glx`, and
//! the host asks for [`Runner3d::frame`] afterwards and hands it to WebGL2.

use super::gl::{Frame, Glx, Texture};
use super::{Resources, SaverDef, StartArgs, XEvent, ya_rand_init};

/// The display a GL saver draws into: the GL context, the resources it was
/// started with, and the clock.
pub struct Gl {
    pub glx: Glx,
    /// The resolved resources. Hacks read these when they start, as the C does.
    pub res: Resources,
    /// Seconds since the saver started.
    pub time: f64,
    width: i32,
    height: i32,
    /// Upstream's `MI_IS_MONO`. Always false here, but hacks branch on it.
    pub mono_p: bool,
    /// What the local time was when the saver started, in seconds since
    /// midnight. Zero unless the host said otherwise.
    wall_clock_base: f64,
    /// The words a saver reads, the same channel [`crate::runtime::Dpy`]
    /// carries: `winduprobot` puts them in a word bubble over a robot. With no
    /// host pushing text in, the compiled-in passage is served.
    words: super::text::TextChannel,
    /// The pictures a saver puts on things, the same channel `Dpy` carries.
    /// With no host pushing pictures in, colour bars are served, which is what
    /// upstream shows when it cannot grab a screen or find a file.
    images: super::image::ImageChannel,
    /// The load in flight, if any. One at a time is all any saver asks for.
    image_pending: Option<super::image::ImageLoad>,
}

/// A picture for a saver to make a texture out of: `grab-ximage.c`'s
/// `load_texture_async` hands over an `XImage` and the rectangle of it the
/// picture actually landed in, and so does this.
pub struct GlImage {
    pub width: i32,
    pub height: i32,
    /// RGBA bytes, the first row at the top, ready for `tex_image_2d`.
    pub pixels: Vec<u8>,
    /// Where in that the picture is. The rest is the black it was centred on.
    pub geometry: super::XRectangle,
    /// What to call it, if the host said.
    pub title: Option<String>,
}

/// Seconds in a day, which is the period [`Gl::wall_clock`] wraps on.
const DAY: f64 = 24.0 * 60.0 * 60.0;

impl Gl {
    /// A display with nothing behind it, for the tests of savers whose
    /// interesting code takes a `&mut Gl` and would otherwise be reachable
    /// only by driving a whole [`Runner3d`].
    #[cfg(test)]
    pub fn for_test(width: i32, height: i32) -> Gl {
        let mut glx = Glx::new();
        glx.start_frame(width, height);
        Gl {
            glx,
            res: Resources::new(&[], &[], ""),
            time: 0.0,
            width,
            height,
            mono_p: false,
            wall_clock_base: 0.0,
            words: super::text::TextChannel::default(),
            images: super::image::ImageChannel::default(),
            image_pending: None,
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// The local time of day in seconds since midnight, as
    /// [`crate::runtime::Dpy::wall_clock`] gives it: the host's clock at
    /// startup plus the saver's own elapsed time, so a run stays reproducible
    /// from its seed and a run with no host simply starts at midnight.
    pub fn wall_clock(&self) -> f64 {
        (self.wall_clock_base + self.time).rem_euclid(DAY)
    }

    /// `textclient_getc`: the next character of the text this saver is
    /// reading, or `None` if there is none to be had this instant.
    pub fn text_getc(&mut self) -> Option<u8> {
        self.words.getc()
    }

    /// `textclient_reshape`: how wide the page is now, so the source can wrap
    /// to it.
    pub fn text_reshape(&mut self, columns: i32, max_lines: i32) {
        self.words.reshape(columns, max_lines);
    }

    /// `load_texture_async`: ask for a picture of about this size to put on
    /// something.
    ///
    /// `None` means it is still being fetched, so ask again next frame.
    /// Without a host to ask there is nothing to wait for and the first call
    /// answers, with colour bars.
    pub fn load_image(&mut self, width: i32, height: i32) -> Option<GlImage> {
        let pending = self.image_pending.take();
        let mut fb = super::Fb::new(width, height);
        self.image_pending = self.images.poll(&mut fb, self.time, pending);
        if self.image_pending.is_some() {
            return None;
        }

        let mut pixels = Vec::with_capacity((fb.width() * fb.height() * 4) as usize);
        for p in fb.pixels() {
            let (r, g, b) = super::color::unrgb(*p);
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
        Some(GlImage {
            width: fb.width(),
            height: fb.height(),
            pixels,
            geometry: self.images.geometry,
            title: self.images.title.clone(),
        })
    }
}

/// `screenhack.h`'s contract, for a saver that draws with OpenGL.
pub trait Hack3d {
    /// Draw one frame, and say how many microseconds to wait before the next.
    fn draw(&mut self, g: &mut Gl) -> u32;
    /// `reshape_NAME`. Called once at startup and again on every resize.
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32);
    /// `NAME_handle_event`. True if the event was used.
    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }
}

/// The smallest step the clock is allowed to take, so a hack asking for no
/// delay still advances. Matches [`crate::runtime::Runner`]'s.
const MIN_DELAY: f64 = 1.0 / 240.0;

/// How many frames one tick may draw before giving up and dropping the rest.
const MAX_FRAMES_PER_TICK: u32 = 4;

pub struct Runner3d {
    gl: Gl,
    hack: Box<dyn Hack3d>,
    def: &'static SaverDef,
    next_due: f64,
    started: bool,
}

impl Runner3d {
    /// Start a saver. Called by the saver itself, from inside its own module,
    /// for the code-splitting reason [`crate::runtime::Runner::start`] explains.
    pub fn start(
        def: &'static SaverDef,
        new: fn(&mut Gl) -> Box<dyn Hack3d>,
        args: StartArgs,
    ) -> Self {
        ya_rand_init(args.seed);
        let res = Resources::new(def.defaults, def.opts, &args.query);
        let mut gl = Gl {
            glx: Glx::new(),
            res,
            time: 0.0,
            width: args.width.max(1),
            height: args.height.max(1),
            mono_p: false,
            wall_clock_base: args.wall_clock,
            words: super::text::TextChannel::default(),
            images: super::image::ImageChannel::default(),
            image_pending: None,
        };
        gl.glx.start_frame(gl.width, gl.height);
        let hack = new(&mut gl);
        Self {
            gl,
            hack,
            def,
            next_due: 0.0,
            started: false,
        }
    }

    pub fn def(&self) -> &'static SaverDef {
        self.def
    }

    /// Draw exactly one frame and advance the clock by the delay it asked for.
    /// Tests use this, so a run depends only on the seed.
    pub fn step(&mut self) -> u32 {
        self.gl.glx.start_frame(self.gl.width, self.gl.height);
        let delay = self.hack.draw(&mut self.gl);
        self.gl.time += (f64::from(delay) / 1_000_000.0).max(MIN_DELAY);
        delay
    }

    /// Advance to wall-clock `now`, drawing as many frames as the hack's
    /// requested delays call for. Only the last one is kept: the frames in
    /// between were never going to be seen.
    pub fn tick(&mut self, now: f64) {
        self.gl.time = now;
        if !self.started {
            self.started = true;
            self.next_due = now;
        }
        let mut budget = MAX_FRAMES_PER_TICK;
        while self.next_due <= now && budget > 0 {
            self.gl.glx.start_frame(self.gl.width, self.gl.height);
            let delay = self.hack.draw(&mut self.gl);
            self.next_due += (f64::from(delay) / 1_000_000.0).max(MIN_DELAY);
            budget -= 1;
        }
        if self.next_due < now {
            self.next_due = now;
        }
    }

    /// `NAME_reshape`. A no-op if the size did not actually change.
    pub fn resize(&mut self, width: i32, height: i32) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.gl.width, self.gl.height) {
            return;
        }
        self.gl.width = width;
        self.gl.height = height;
        self.hack.reshape(&mut self.gl, width, height);
    }

    pub fn event(&mut self, event: XEvent) -> bool {
        self.hack.event(&mut self.gl, &event)
    }

    /// What the last frame drew.
    pub fn frame(&self) -> &Frame {
        self.gl.glx.frame()
    }

    /// A texture the saver built, for the host to upload. Textures are made
    /// once and referred to by name from then on, so the host keeps its own
    /// uploaded copy and only asks for a name it has not seen.
    pub fn texture(&self, id: u32) -> Option<&Texture> {
        self.gl.glx.texture(id)
    }

    pub fn width(&self) -> i32 {
        self.gl.width
    }

    pub fn height(&self) -> i32 {
        self.gl.height
    }
}
