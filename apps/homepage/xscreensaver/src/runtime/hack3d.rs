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
    /// Map tiles, which are the one thing a saver wants many of at once.
    /// Only `mapscroller` asks.
    tiles: super::tiles::TileChannel,
    /// Letters the compiled-in font does not have. Only `unicrud` asks.
    glyphs: super::glyph::GlyphChannel,
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
            tiles: super::tiles::TileChannel::default(),
            glyphs: super::glyph::GlyphChannel::default(),
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// How long this saver has been running, in seconds.
    ///
    /// A saver that animates against a clock rather than a frame count wants
    /// the difference between two of these. [`Gl::wall_clock`] would do, but
    /// it wraps at midnight, and a saver differencing it would see one frame a
    /// day run backwards.
    pub fn elapsed(&self) -> f64 {
        self.time
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
        self.words.getc(self.time)
    }

    /// `textclient_reshape`: how wide the page is now, so the source can wrap
    /// to it.
    /// Host side: tell the runtime that text can be fetched. Without this the
    /// compiled-in passage is served, which is what the native tests get.
    pub fn set_text_host(&mut self, supplies: bool) {
        self.words.host_supplies = supplies;
    }

    /// Ask the host to draw one codepoint, about `size` pixels tall.
    pub fn request_glyph(&mut self, codepoint: u32, size: i32) {
        self.glyphs.request(codepoint, size);
    }

    /// The glyph the host drew, if it has. A `None` image means the host has
    /// no glyph for that codepoint.
    pub fn take_glyph(&mut self) -> Option<(u32, Option<super::XImage>)> {
        self.glyphs.take()
    }

    /// Whether anything is going to draw a glyph at all.
    pub fn glyphs_available(&self) -> bool {
        self.glyphs.host_supplies
    }

    /// Host side: tell the runtime that glyphs can be drawn.
    pub fn set_glyph_host(&mut self, supplies: bool) {
        self.glyphs.host_supplies = supplies;
    }

    /// Host side: what codepoint the saver is waiting for.
    pub fn take_glyph_request(&mut self) -> Option<(u32, i32)> {
        self.glyphs.wanted.take()
    }

    /// Host side: hand back a drawn glyph, or `None` if there is no such
    /// character in any font the host has.
    pub fn deliver_glyph(&mut self, codepoint: u32, image: Option<super::XImage>) {
        self.glyphs.ready = Some((codepoint, image));
    }

    /// `mapscroller`'s loader: ask for the image at `url`, to be called `key`.
    pub fn request_tile(&mut self, key: u64, url: String) {
        self.tiles.request(key, url);
    }

    /// The next tile the host has answered with, if any. A `None` image means
    /// the fetch failed.
    pub fn take_tile(&mut self) -> Option<(u64, Option<super::XImage>)> {
        self.tiles.take()
    }

    /// Whether anything is going to answer a tile request at all.
    pub fn tiles_available(&self) -> bool {
        self.tiles.host_supplies
    }

    /// Host side: tell the runtime that tiles can be fetched.
    pub fn set_tile_host(&mut self, supplies: bool) {
        self.tiles.host_supplies = supplies;
    }

    /// Host side: what the saver is waiting for, taken off the queue.
    pub fn take_tile_requests(&mut self) -> Vec<(u64, String)> {
        std::mem::take(&mut self.tiles.wanted)
    }

    /// Host side: hand back a fetched tile, or `None` if it could not be had.
    pub fn deliver_tile(&mut self, key: u64, image: Option<super::XImage>) {
        self.tiles.ready.push((key, image));
    }

    /// Host side: tell the runtime that pictures can be fetched. Without it
    /// a request is answered on the spot with colour bars, which is what the
    /// native tests get.
    pub fn set_image_host(&mut self, supplies: bool) {
        self.images.host_supplies = supplies;
    }

    /// Host side: has a hack asked for a picture since the last check?
    pub fn take_image_request(&mut self) -> bool {
        std::mem::take(&mut self.images.requested)
    }

    /// Host side: does this hack work on a picture at all? See
    /// [`crate::Dpy::hack_uses_images`].
    pub fn hack_uses_images(&self) -> bool {
        self.images.ever_wanted
    }

    /// Host side: hand over a decoded picture, and optionally what to call it.
    pub fn deliver_image(&mut self, image: super::XImage, title: Option<String>) {
        self.images.ready = Some(image);
        self.images.title = title;
    }

    /// The caption of the picture on screen, if the host gave one.
    pub fn image_title(&self) -> Option<&str> {
        self.images.title.as_deref()
    }

    /// Host side: has a hack asked for text since the last check?
    pub fn take_text_request(&mut self) -> bool {
        std::mem::take(&mut self.words.requested)
    }

    /// Host side: hand over some words.
    pub fn deliver_text(&mut self, s: &str) {
        self.words.deliver(s);
    }

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
            tiles: super::tiles::TileChannel::default(),
            glyphs: super::glyph::GlyphChannel::default(),
        };
        gl.set_text_host(args.text_host);
        gl.set_image_host(args.image_host);
        gl.set_tile_host(args.tile_host);
        gl.set_glyph_host(args.glyph_host);
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

    /// Host side: has the hack asked for a picture since the last check?
    pub fn take_image_request(&mut self) -> bool {
        self.gl.take_image_request()
    }

    /// Host side: does this hack work on a picture at all? See
    /// [`crate::Dpy::hack_uses_images`].
    pub fn hack_uses_images(&self) -> bool {
        self.gl.hack_uses_images()
    }

    /// Host side: what map tiles the saver is waiting for.
    pub fn take_tile_requests(&mut self) -> Vec<(u64, String)> {
        self.gl.take_tile_requests()
    }

    /// Host side: what codepoint the saver is waiting for.
    pub fn take_glyph_request(&mut self) -> Option<(u32, i32)> {
        self.gl.take_glyph_request()
    }

    /// Host side: hand back a drawn glyph.
    pub fn deliver_glyph(&mut self, codepoint: u32, image: Option<super::XImage>) {
        self.gl.deliver_glyph(codepoint, image);
    }

    /// Host side: hand back a fetched tile, or `None` if it could not be had.
    pub fn deliver_tile(&mut self, key: u64, image: Option<super::XImage>) {
        self.gl.deliver_tile(key, image);
    }

    /// Host side: hand the saver a decoded picture.
    pub fn deliver_image(&mut self, image: super::XImage, title: Option<String>) {
        self.gl.deliver_image(image, title);
    }

    /// The caption of the picture on screen, if the host gave one.
    pub fn image_title(&self) -> Option<&str> {
        self.gl.image_title()
    }

    /// Host side: has the hack asked for text since the last check?
    pub fn take_text_request(&mut self) -> bool {
        self.gl.take_text_request()
    }

    /// Host side: hand over some words.
    pub fn deliver_text(&mut self, s: &str) {
        self.gl.deliver_text(s);
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
