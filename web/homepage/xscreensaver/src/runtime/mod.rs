//! The `screenhack.h` contract, in Rust.
//!
//! Upstream every 2D hack exports five C functions and a table:
//!
//! ```text
//! static void *NAME_init    (Display *, Window);
//! static unsigned long NAME_draw (Display *, Window, void *closure);
//! static void NAME_reshape  (Display *, Window, void *, unsigned, unsigned);
//! static Bool NAME_event    (Display *, Window, void *, XEvent *);
//! static void NAME_free     (Display *, Window, void *);
//! XSCREENSAVER_MODULE ("Name", name)
//! ```
//!
//! `init` returns the hack's private state, `draw` renders one step and returns
//! how many microseconds it would like to wait, and the driver loops. That maps
//! onto [`Screenhack`] with the state as `Self`, `free` as `Drop`, and the
//! module table as [`SaverDef`].
//!
//! [`Runner`] is the driver. The browser host and the tests both go through it,
//! so a saver behaves identically in a `cargo test` and on the page.

pub mod analogtv;
pub mod color;
pub mod delaunay;
pub mod easing;
pub mod erase;
pub mod fb;
pub mod font;
pub mod gl;
pub mod hack3d;
pub mod image;
pub mod involute;
pub mod jpeg;
pub mod opts;
pub mod png;
pub mod rand;
pub mod rotator;
pub mod shapes;
pub mod spline;
pub mod texfont;
pub mod text;
pub mod tty;
pub mod tube;
pub mod xlockmore;

pub use color::{Pixel, XColor};
pub use easing::{Ease, ease};
pub use fb::{Fb, GXFunc, Gc, Pixmap, XArc, XImage, XPoint, XRectangle, XSegment};
pub use hack3d::{Gl, Hack3d, Runner3d};
pub use image::ImageLoad;
pub use opts::{Opt, OptKind, Resources, SelectItem};
pub use rand::{frand, random, random_below, ya_rand_init};
pub use rotator::{Rotator, Trackball};
pub use shapes::{do_normal, unit_dome, unit_sphere};
pub use tube::{cone, tube, unit_cone, unit_tube};

/// An input event, reduced to what the hacks actually look at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum XEvent {
    ButtonPress { x: i32, y: i32, button: u32 },
    ButtonRelease { x: i32, y: i32, button: u32 },
    MotionNotify { x: i32, y: i32 },
    KeyPress { key: char },
}

/// `screenhack_event_helper`: upstream's "did the user poke it" test, which
/// most hacks use to mean "start over".
pub fn screenhack_event_helper(event: &XEvent) -> bool {
    matches!(event, XEvent::ButtonPress { .. } | XEvent::KeyPress { .. })
}

/// The display: the window's framebuffer, the resource database, and the clock.
///
/// The `Display *` and `Window` that every upstream call takes are both folded
/// in here, so `XFillRectangle (dpy, window, gc, ..)` ports to
/// `d.win().fill_rectangle(&gc, ..)`.
pub struct Dpy {
    window: Fb,
    /// The resolved resources. Hacks read these in `new`, as the C reads them
    /// in `init`.
    pub res: Resources,
    /// Seconds since the saver started. Set by [`Runner`] before each draw.
    pub time: f64,
    /// Upstream's `mono_p` global. Always false here (a canvas is TrueColor),
    /// but hacks branch on it and a few assign to it.
    pub mono_p: bool,
    /// What the local time was when the saver started, in seconds since
    /// midnight. Zero unless the host said otherwise.
    wall_clock_base: f64,
    images: image::ImageChannel,
    words: text::TextChannel,
}

/// Seconds in a day, which is the period [`Dpy::wall_clock`] wraps on.
const DAY: f64 = 24.0 * 60.0 * 60.0;

impl Dpy {
    pub fn new(width: i32, height: i32, res: Resources) -> Self {
        let background = res.pixel("background");
        let mut window = Fb::new(width, height);
        window.clear(background);
        Self {
            window,
            res,
            time: 0.0,
            mono_p: false,
            wall_clock_base: 0.0,
            images: image::ImageChannel::default(),
            words: text::TextChannel::default(),
        }
    }

    /// `localtime()`: the local time of day in seconds since midnight,
    /// fractional part included.
    ///
    /// A handful of hacks are clocks and have to say what time it is. The base
    /// comes from the host at startup and the rest is the saver's own elapsed
    /// time, which keeps a run reproducible from its seed: with no host the
    /// clock simply starts at midnight.
    pub fn wall_clock(&self) -> f64 {
        (self.wall_clock_base + self.time).rem_euclid(DAY)
    }

    fn set_wall_clock(&mut self, seconds_since_midnight: f64) {
        self.wall_clock_base = seconds_since_midnight;
    }

    /// `load_image_async_simple`: ask for a picture to work on.
    ///
    /// Pass `None` to start, then hand back what you get until it returns
    /// `None`, at which point the image has been drawn into the window. If no
    /// host is supplying images you get colour bars, exactly as upstream does
    /// when it cannot grab a screen or find a file.
    pub fn load_image_async_simple(&mut self, pending: Option<ImageLoad>) -> Option<ImageLoad> {
        // Split the borrow: the channel draws into the window it is told about.
        let Dpy {
            window,
            images,
            time,
            ..
        } = self;
        images.poll(window, *time, pending)
    }

    /// `load_image_async`: the same, into a drawable of your own rather than
    /// the window.
    ///
    /// Upstream's takes a `Drawable` too. A hack that wants several pictures at
    /// once, or wants to do something to one before it is seen, loads into a
    /// pixmap; the window is only the common case.
    pub fn load_image_into(
        &mut self,
        target: &mut Fb,
        pending: Option<ImageLoad>,
    ) -> Option<ImageLoad> {
        let Dpy { images, time, .. } = self;
        images.poll(target, *time, pending)
    }

    /// Where in the window the last picture landed, which upstream's loader
    /// returns through an out-parameter. Only meaningful once a load has
    /// finished; before that it is the whole window.
    pub fn image_geometry(&self) -> XRectangle {
        self.images.geometry
    }

    /// Host side: tell the runtime that images can be fetched. Without this a
    /// request is answered immediately with colour bars, which is what makes
    /// the native tests work with no host at all.
    pub fn set_image_host(&mut self, supplies: bool) {
        self.images.host_supplies = supplies;
    }

    /// Host side: has a hack asked for an image since the last check?
    pub fn take_image_request(&mut self) -> bool {
        std::mem::take(&mut self.images.requested)
    }

    /// Host side: hand over a decoded image, and optionally what to call it.
    pub fn deliver_image(&mut self, image: XImage, title: Option<String>) {
        self.images.ready = Some(image);
        self.images.title = title;
    }

    /// The caption of the image on screen, if the host gave one. The panel
    /// shows it, so you can tell whose picture you are looking at.
    pub fn image_title(&self) -> Option<&str> {
        self.images.title.as_deref()
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

    /// Host side: tell the runtime that text can be fetched. Without this the
    /// compiled-in passage is served, which is what makes the native tests
    /// work with no host at all.
    pub fn set_text_host(&mut self, supplies: bool) {
        self.words.host_supplies = supplies;
    }

    /// Host side: has a hack asked for text since the last check?
    pub fn take_text_request(&mut self) -> bool {
        std::mem::take(&mut self.words.requested)
    }

    /// Host side: hand over some more text to read.
    pub fn deliver_text(&mut self, s: &str) {
        self.words.pending.extend(s.as_bytes());
    }

    /// The window, as a drawable.
    #[inline]
    pub fn win(&mut self) -> &mut Fb {
        &mut self.window
    }

    /// The window, read-only. `XGetImage`-ish reads go through here.
    #[inline]
    pub fn win_ref(&self) -> &Fb {
        &self.window
    }

    /// `XGetWindowAttributes(..).width`.
    #[inline]
    pub fn width(&self) -> i32 {
        self.window.width()
    }

    /// `XGetWindowAttributes(..).height`.
    #[inline]
    pub fn height(&self) -> i32 {
        self.window.height()
    }

    /// `XCreatePixmap`, initialised to opaque black as X leaves it undefined.
    pub fn new_pixmap(&self, width: i32, height: i32) -> Pixmap {
        Pixmap::new(width, height)
    }

    /// `XClearWindow`.
    pub fn clear_window(&mut self) {
        let bg = self.res.pixel("background");
        self.window.clear(bg);
    }

    fn resize(&mut self, width: i32, height: i32) {
        let bg = self.res.pixel("background");
        self.window.resize(width, height);
        self.window.clear(bg);
    }
}

/// A ported hack.
///
/// `init` has no place here: construction is the port's own constructor, which
/// is what the `new` field of [`SaverDef`] points at. `free` has no place
/// either; `Drop` covers it, and most hacks only `free` their own state.
pub trait Screenhack {
    /// `NAME_draw`: render one step, and return how many microseconds to wait
    /// before the next one.
    fn draw(&mut self, d: &mut Dpy) -> u32;

    /// `NAME_reshape`.
    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        let _ = (d, width, height);
    }

    /// `NAME_event`: return true if the event was consumed.
    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        let _ = (d, event);
        false
    }
}

/// Attribution, parsed out of the `<_description>` in the upstream config XML.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct About {
    pub author: &'static str,
    pub year: &'static str,
    /// The upstream demo video, from `<video href>`.
    pub video: Option<&'static str>,
    pub blurb: &'static str,
}

/// What a saver needs to know to start: everything the host can decide without
/// having loaded the saver yet.
///
/// A struct rather than five parameters because it crosses the code-splitting
/// boundary, and a `LazyLoader` entry point takes exactly one argument.
#[derive(Clone, Debug, PartialEq)]
pub struct StartArgs {
    pub width: i32,
    pub height: i32,
    /// The URL query, which is the settings the panel has changed.
    pub query: String,
    /// Fixes the random stream: the same seed and size reproduce the same
    /// frames. The host passes a random seed; the tests pass a constant.
    pub seed: u32,
    /// Whether the host can supply pictures for the hacks that want one.
    ///
    /// It has to be known before the hack starts, not after: several of them
    /// ask for their image in `init`, and a request made before the host has
    /// spoken up is answered on the spot with colour bars.
    pub image_host: bool,
    /// The local time of day when the saver starts, in seconds since midnight.
    /// Only the clocks read it, and the tests leave it at midnight.
    pub wall_clock: f64,
}

impl StartArgs {
    pub fn new(width: i32, height: i32, query: &str, seed: u32) -> Self {
        Self {
            width,
            height,
            query: query.to_string(),
            seed,
            image_host: false,
            wall_clock: 0.0,
        }
    }

    /// Declare that the host will answer image requests.
    pub fn with_image_host(mut self, image_host: bool) -> Self {
        self.image_host = image_host;
        self
    }

    /// Tell the clocks what time it is, in seconds since local midnight.
    pub fn with_wall_clock(mut self, seconds_since_midnight: f64) -> Self {
        self.wall_clock = seconds_since_midnight;
        self
    }
}

/// A saver's identity and its knobs: everything about it except its code.
pub struct SaverDef {
    /// URL slug, matching the upstream binary name.
    pub slug: &'static str,
    /// Display name, from the XML `_label`.
    pub label: &'static str,
    /// The hack's `NAME_defaults[]`, copied verbatim from the C.
    pub defaults: &'static [&'static str],
    /// The knobs the panel shows, derived from `hacks/config/NAME.xml`.
    pub opts: &'static [Opt],
    pub about: About,
}

/// A saver as a table can hold it: its definition plus its entry point.
///
/// Native only. Naming a saver's entry point from a shared table is exactly
/// what stops the web build from splitting it out, so the web host reaches each
/// saver through its own lazily-loaded chunk instead. See [`crate::all`].
#[cfg(not(target_arch = "wasm32"))]
pub struct Saver {
    pub def: &'static SaverDef,
    pub start: fn(StartArgs) -> Runner,
}

/// The same, for a saver that draws with OpenGL rather than into a
/// framebuffer.
#[cfg(not(target_arch = "wasm32"))]
pub struct Saver3d {
    pub def: &'static SaverDef,
    pub start: fn(StartArgs) -> Runner3d,
}

/// The most frames [`Runner::tick`] will draw for one call, however far behind
/// the clock it is. A backgrounded tab or a slow frame must not turn into an
/// unbounded catch-up burst.
const MAX_FRAMES_PER_TICK: u32 = 8;

/// A hack that asks for no delay still gets paced, or it would draw
/// [`MAX_FRAMES_PER_TICK`] times per animation frame for no visible gain.
const MIN_DELAY: f64 = 1.0 / 240.0;

/// Drives one saver: owns the display, the hack, and the frame pacing.
pub struct Runner {
    pub dpy: Dpy,
    hack: Box<dyn Screenhack>,
    def: &'static SaverDef,
    next_due: f64,
    started: bool,
}

impl Runner {
    /// Start a saver. Called by the saver itself, from inside its own module.
    ///
    /// The direction matters for code splitting. If the host called into a
    /// saver through a function pointer it fetched, the splitter would see the
    /// hack only as an indirect call from main-resident code and would leave it
    /// in the main module. Because the saver's own `start` names its
    /// constructor directly, everything the hack reaches is reachable from that
    /// one exported function and nowhere else, which is what lets it move into
    /// the saver's chunk. Everything in here is shared and stays in main.
    pub fn start(
        def: &'static SaverDef,
        new: fn(&mut Dpy) -> Box<dyn Screenhack>,
        args: StartArgs,
    ) -> Self {
        ya_rand_init(args.seed);
        let res = Resources::new(def.defaults, def.opts, &args.query);
        let mut dpy = Dpy::new(args.width, args.height, res);
        dpy.set_image_host(args.image_host);
        dpy.set_wall_clock(args.wall_clock);
        let hack = new(&mut dpy);
        Self {
            dpy,
            hack,
            def,
            next_due: 0.0,
            started: false,
        }
    }

    pub fn def(&self) -> &'static SaverDef {
        self.def
    }

    /// Draw exactly one step and advance the clock by however long the hack
    /// asked to wait.
    ///
    /// Tests use this instead of [`Runner::tick`]: it runs the saver at its own
    /// requested rate with no wall clock involved, so the output depends only
    /// on the seed. The clock still has to move, because anything time-based
    /// (the shared erasers, most obviously) would otherwise never finish.
    pub fn step(&mut self) -> u32 {
        let delay = self.hack.draw(&mut self.dpy);
        self.dpy.time += (delay as f64 / 1_000_000.0).max(MIN_DELAY);
        delay
    }

    /// Advance to wall-clock `now` (seconds), drawing as many steps as the
    /// hack's requested delays call for.
    pub fn tick(&mut self, now: f64) {
        self.dpy.time = now;
        if !self.started {
            self.started = true;
            self.next_due = now;
        }
        let mut budget = MAX_FRAMES_PER_TICK;
        while self.next_due <= now && budget > 0 {
            let delay = self.hack.draw(&mut self.dpy);
            self.next_due += (delay as f64 / 1_000_000.0).max(MIN_DELAY);
            budget -= 1;
        }
        // Whatever we could not keep up with is dropped rather than queued.
        if self.next_due < now {
            self.next_due = now;
        }
    }

    /// `NAME_reshape`. A no-op if the size did not actually change, because
    /// several hacks restart themselves on reshape.
    pub fn resize(&mut self, width: i32, height: i32) {
        if width == self.dpy.width() && height == self.dpy.height() {
            return;
        }
        self.dpy.resize(width, height);
        let (w, h) = (self.dpy.width(), self.dpy.height());
        self.hack.reshape(&mut self.dpy, w, h);
    }

    /// `NAME_event`.
    pub fn event(&mut self, event: XEvent) -> bool {
        self.hack.event(&mut self.dpy, &event)
    }

    /// The current frame as RGBA bytes, ready for `putImageData`.
    pub fn frame_bytes(&self) -> &[u8] {
        self.dpy.win_ref().as_bytes()
    }

    /// A cheap content hash of the current frame, for regression tests.
    pub fn frame_hash(&self) -> u64 {
        // FNV-1a over the pixels. Not cryptographic, just stable.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for p in self.dpy.win_ref().pixels() {
            for b in p.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        static DRAWS: Cell<u32> = const { Cell::new(0) };
    }

    struct Counter {
        delay: u32,
    }

    impl Screenhack for Counter {
        fn draw(&mut self, d: &mut Dpy) -> u32 {
            DRAWS.with(|c| c.set(c.get() + 1));
            let gc = Gc::new(color::WHITE, color::BLACK);
            d.win().draw_point(&gc, 0, 0);
            self.delay
        }
    }

    static SLOW: SaverDef = SaverDef {
        slug: "slow",
        label: "Slow",
        defaults: &[".background: black", ".foreground: white"],
        opts: &[],
        about: About {
            author: "test",
            year: "2026",
            video: None,
            blurb: "",
        },
    };

    static GREEDY: SaverDef = SaverDef {
        slug: "greedy",
        label: "Greedy",
        defaults: &[".background: black", ".foreground: white"],
        opts: &[],
        about: About {
            author: "test",
            year: "2026",
            video: None,
            blurb: "",
        },
    };

    fn slow(args: StartArgs) -> Runner {
        Runner::start(&SLOW, |_| Box::new(Counter { delay: 10_000 }), args)
    }

    fn greedy(args: StartArgs) -> Runner {
        Runner::start(&GREEDY, |_| Box::new(Counter { delay: 0 }), args)
    }

    fn draws_during(f: impl FnOnce()) -> u32 {
        DRAWS.with(|c| c.set(0));
        f();
        DRAWS.with(|c| c.get())
    }

    #[test]
    fn pacing_follows_the_requested_delay() {
        // 10ms delay over a 100ms tick is ten steps, not one.
        let n = draws_during(|| {
            let mut r = slow(StartArgs::new(64, 64, "", 1));
            r.tick(0.0);
            r.tick(0.1);
        });
        assert!((9..=MAX_FRAMES_PER_TICK + 1).contains(&n), "drew {n} times");
    }

    #[test]
    fn catch_up_is_bounded() {
        // A tab that was hidden for an hour must not draw an hour of frames.
        let n = draws_during(|| {
            let mut r = slow(StartArgs::new(64, 64, "", 1));
            r.tick(0.0);
            r.tick(3600.0);
        });
        assert!(
            n <= MAX_FRAMES_PER_TICK + 1,
            "drew {n} times after a long pause"
        );
    }

    #[test]
    fn a_zero_delay_hack_is_still_paced() {
        let n = draws_during(|| {
            let mut r = greedy(StartArgs::new(64, 64, "", 1));
            r.tick(0.0);
            r.tick(1.0 / 60.0);
        });
        assert!(n <= MAX_FRAMES_PER_TICK + 1, "drew {n} times in one frame");
    }

    #[test]
    fn the_same_seed_gives_the_same_frame() {
        let mut a = slow(StartArgs::new(64, 64, "", 7));
        let mut b = slow(StartArgs::new(64, 64, "", 7));
        for _ in 0..10 {
            a.step();
            b.step();
        }
        assert_eq!(a.frame_hash(), b.frame_hash());
    }

    #[test]
    fn resize_is_a_no_op_when_the_size_is_unchanged() {
        let mut r = slow(StartArgs::new(64, 64, "", 1));
        r.step();
        let before = r.frame_hash();
        r.resize(64, 64);
        assert_eq!(r.frame_hash(), before);
        r.resize(80, 40);
        assert_eq!(r.dpy.width(), 80);
        assert_eq!(r.dpy.height(), 40);
    }

    #[test]
    fn event_helper_only_fires_on_a_poke() {
        assert!(screenhack_event_helper(&XEvent::KeyPress { key: 'x' }));
        assert!(screenhack_event_helper(&XEvent::ButtonPress {
            x: 0,
            y: 0,
            button: 1
        }));
        assert!(!screenhack_event_helper(&XEvent::MotionNotify {
            x: 0,
            y: 0
        }));
    }
}
