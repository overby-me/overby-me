//! The screensaver stage: `/screensaver` and `/screensaver/:name`.
//!
//! One canvas, one `requestAnimationFrame` loop, and one options panel behind a
//! button, driving whichever [`SaverDef`] the router asked for. None of this is
//! lazily loaded; only the savers themselves are (see [`crate::pages::savers`]),
//! so the shared runtime is downloaded once however many you look at.
//!
//! A 2D saver draws into a software framebuffer, which this blits to the canvas
//! with `putImageData` once per animation frame. A Shadertoy saver is a
//! fragment shader instead, and [`crate::pages::gl`] runs it. Everything around
//! them, the canvas, the frame loop, the panel and the URL, is shared: only the
//! engine differs, and it has to be chosen before the canvas is mounted because
//! a canvas has one context for its lifetime.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use dioxus::prelude::*;
use dioxus::web::WebEventExt;
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};
use xscreensaver::SaverDef;
use xscreensaver::runtime::{OptKind, Runner, StartArgs, XEvent, XImage};

use crate::Route;
use crate::images::{self, Source};
use crate::pages::gl::GlEngine;
use crate::pages::gl3d::Gl3dEngine;
use crate::pages::savers::{self, Start};
use crate::pages::ui::{Choice, Details, Slider, Toggle};
use crate::text::{self, Source as TextSource};
use crate::url::{captured_query, replace_query};

/// How long to wait before asking an image source again after it came up
/// empty. A live hashtag can be quiet for a while.
const IMAGE_RETRY_SECONDS: f64 = 2.0;

/// The largest framebuffer edge we will rasterise, in pixels.
///
/// These hacks are drawn a pixel at a time on the CPU, so cost is linear in
/// area: a 5K display at device resolution would be 15 million pixels per
/// frame. Past this the buffer is scaled up by CSS instead, which suits hacks
/// designed for 1990s screens anyway.
const MAX_BUFFER_EDGE: i32 = 1600;

/// Ratio of CSS pixels to framebuffer pixels, and the resulting buffer size.
fn buffer_size(css_w: i32, css_h: i32) -> (i32, i32, i32) {
    let longest = css_w.max(css_h).max(1);
    let scale = (longest as f64 / MAX_BUFFER_EDGE as f64).ceil().max(1.0) as i32;
    ((css_w / scale).max(1), (css_h / scale).max(1), scale)
}

/// A 2D hack and the context it blits through.
struct FbEngine {
    ctx: CanvasRenderingContext2d,
    runner: Runner,
}

/// The two ways a saver can reach the canvas. Both boxed: there is one of these
/// per page and the arms are nothing like the same size.
enum Engine {
    /// A 2D hack: a software framebuffer, blitted with `putImageData`.
    Fb(Box<FbEngine>),
    /// An OpenGL saver: vertex batches, drawn by WebGL2.
    Gl3d(Box<Gl3dEngine>),
    /// A Shadertoy program, drawn by WebGL2.
    Gl(Box<GlEngine>),
}

struct Host {
    canvas: HtmlCanvasElement,
    engine: Engine,
    paused: bool,
    /// CSS pixels per framebuffer pixel, for mapping pointer coordinates back.
    scale: i32,
    css_w: i32,
    css_h: i32,
    /// The drawing buffer's size in pixels, which is the canvas attribute
    /// rather than its CSS size.
    buf_w: i32,
    buf_h: i32,
    /// The query the running hack was started with, so that the settings effect
    /// firing for an unrelated reason does not restart it.
    query: String,
    /// Where this saver's pictures come from, for the hacks that want one.
    source: Source,
    /// A hack has asked for a picture and has not been given one yet.
    image_wanted: bool,
    /// A fetch is in flight, so we do not start a second one.
    image_fetching: bool,
    /// Do not hammer a hashtag firehose that has not produced anything yet.
    image_retry_at: f64,
    /// Where this saver's words come from, for the hacks that read text.
    text_source: TextSource,
    /// The same three flags again, for text. A saver reads text a character
    /// at a time and asks constantly, so this has the same shape as the
    /// picture bookkeeping rather than a different one.
    text_wanted: bool,
    text_fetching: bool,
    text_retry_at: f64,
    /// Whether this saver has ever asked for words, so the panel only offers
    /// a "Words" section for the ten or so that read any.
    reads_text: bool,
}

impl Host {
    fn def(&self) -> &'static SaverDef {
        match &self.engine {
            Engine::Fb(fb) => fb.runner.def(),
            Engine::Gl3d(gl) => gl.def(),
            Engine::Gl(gl) => gl.def(),
        }
    }

    /// Reset the picture bookkeeping after a restart. The saver itself was
    /// told about the source through its [`StartArgs`], because hacks ask for
    /// their image while starting up.
    fn announce_image_source(&mut self) {
        self.image_wanted = false;
        self.image_fetching = false;
        self.image_retry_at = 0.0;
        self.text_wanted = false;
        self.text_fetching = false;
        self.text_retry_at = 0.0;
    }

    /// Has the hack asked for words? Both the framebuffer and the 3D savers
    /// do: `starwars` and `fliptext` are the ones people notice.
    fn wants_text(&mut self) -> bool {
        match &mut self.engine {
            Engine::Fb(fb) => fb.runner.dpy.take_text_request(),
            Engine::Gl3d(gl) => gl.take_text_request(),
            Engine::Gl(_) => false,
        }
    }

    fn deliver_text(&mut self, s: &str) {
        match &mut self.engine {
            Engine::Fb(fb) => fb.runner.dpy.deliver_text(s),
            Engine::Gl3d(gl) => gl.deliver_text(s),
            Engine::Gl(_) => {}
        }
    }

    fn start_args(&self, query: &str) -> StartArgs {
        StartArgs::new(self.buf_w, self.buf_h, query, seed())
            .with_image_host(self.source != Source::None)
            .with_text_host(true)
            .with_wall_clock(wall_clock_seconds())
    }

    fn sync_size(&mut self) {
        let css_w = self.canvas.client_width();
        let css_h = self.canvas.client_height();
        if css_w == self.css_w && css_h == self.css_h {
            return;
        }
        if css_w <= 0 || css_h <= 0 {
            return;
        }
        let (w, h, scale) = buffer_size(css_w, css_h);
        self.css_w = css_w;
        self.css_h = css_h;
        self.scale = scale;
        self.buf_w = w;
        self.buf_h = h;
        self.canvas.set_width(w as u32);
        self.canvas.set_height(h as u32);
        match &mut self.engine {
            Engine::Fb(fb) => fb.runner.resize(w, h),
            Engine::Gl3d(gl) => gl.resize(w, h),
            Engine::Gl(gl) => gl.resize(w, h),
        }
    }

    /// Advance the saver and put the result on the canvas.
    fn draw(&mut self, now: f64) {
        match &mut self.engine {
            Engine::Fb(fb) => {
                if !self.paused {
                    fb.runner.tick(now);
                }
                self.blit();
            }
            // Nothing to blit: GL wrote to the canvas itself. A paused one
            // simply is not drawn, and the last frame stays up.
            Engine::Gl3d(gl) => {
                if !self.paused {
                    gl.draw(now);
                }
            }
            Engine::Gl(gl) => {
                if !self.paused {
                    gl.draw(now);
                }
            }
        }
    }

    fn event(&mut self, event: XEvent) {
        match &mut self.engine {
            Engine::Fb(fb) => {
                fb.runner.event(event);
            }
            Engine::Gl3d(gl) => {
                gl.event(&event);
            }
            Engine::Gl(gl) => {
                gl.event(&event);
            }
        }
    }

    /// Has the hack asked for a picture? Only the 2D ones ever do; a Shadertoy
    /// program is self-contained by definition.
    fn wants_image(&mut self) -> bool {
        match &mut self.engine {
            Engine::Fb(fb) => fb.runner.dpy.take_image_request(),
            Engine::Gl3d(gl) => gl.take_image_request(),
            Engine::Gl(_) => false,
        }
    }

    fn deliver_image(&mut self, image: XImage, title: Option<String>) {
        match &mut self.engine {
            Engine::Fb(fb) => fb.runner.dpy.deliver_image(image, title),
            Engine::Gl3d(gl) => gl.deliver_image(image, title),
            Engine::Gl(_) => {}
        }
    }

    fn image_title(&self) -> Option<String> {
        match &self.engine {
            Engine::Fb(fb) => fb.runner.dpy.image_title().map(str::to_string),
            Engine::Gl3d(gl) => gl.image_title(),
            Engine::Gl(_) => None,
        }
    }

    fn blit(&self) {
        let Engine::Fb(fb) = &self.engine else {
            return;
        };
        let w = fb.runner.dpy.width() as u32;
        let h = fb.runner.dpy.height() as u32;
        let bytes = fb.runner.frame_bytes();
        // The copying constructor, not the zero-copy
        // `Uint8ClampedArray::view` one: constructing an `ImageData` from a
        // view into the wasm heap throws here, and throwing is indistinguishable
        // from a black canvas if you ignore the result (which is how this was
        // broken the first time). The copy is one memcpy of the framebuffer per
        // frame, far cheaper than rasterising it.
        let img = match ImageData::new_with_u8_clamped_array_and_sh(Clamped(bytes), w, h) {
            Ok(img) => img,
            Err(e) => {
                log::error!("screensaver: could not build ImageData {w}x{h}: {e:?}");
                return;
            }
        };
        if let Err(e) = fb.ctx.put_image_data(&img, 0.0, 0.0) {
            log::error!("screensaver: putImageData failed: {e:?}");
        }
    }

    /// Map a pointer position in CSS pixels to framebuffer coordinates.
    fn to_buffer(&self, client_x: f64, client_y: f64) -> (i32, i32) {
        let rect = self.canvas.get_bounding_client_rect();
        (
            ((client_x - rect.left()) / self.scale as f64) as i32,
            ((client_y - rect.top()) / self.scale as f64) as i32,
        )
    }
}

fn seed() -> u32 {
    (js_sys::Math::random() * u32::MAX as f64) as u32
}

/// The local time of day in seconds since midnight, for the savers that are
/// clocks.
fn wall_clock_seconds() -> f64 {
    let now = js_sys::Date::new_0();
    now.get_hours() as f64 * 3600.0
        + now.get_minutes() as f64 * 60.0
        + now.get_seconds() as f64
        + now.get_milliseconds() as f64 / 1000.0
}

fn now_seconds() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now() / 1000.0)
        .unwrap_or(0.0)
}

type AnimationClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// requestAnimationFrame loop; stops once the canvas leaves the document.
fn start_animation_loop(host: Rc<RefCell<Host>>) {
    let f: AnimationClosure = Rc::new(RefCell::new(None));
    let g = Rc::clone(&f);

    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        let mut fetch: Option<(Source, i32, i32)> = None;
        let mut fetch_text: Option<TextSource> = None;
        let connected = {
            let mut h = host.borrow_mut();
            if h.canvas.is_connected() {
                h.sync_size();
                let now = now_seconds();
                h.draw(now);

                if h.wants_image() {
                    h.image_wanted = true;
                    h.image_retry_at = 0.0;
                }
                if h.image_wanted && !h.image_fetching && now >= h.image_retry_at {
                    h.image_fetching = true;
                    fetch = Some((h.source.clone(), h.buf_w, h.buf_h));
                }

                if h.wants_text() {
                    h.text_wanted = true;
                    h.reads_text = true;
                }
                if h.text_wanted && !h.text_fetching && now >= h.text_retry_at {
                    h.text_fetching = true;
                    fetch_text = Some(h.text_source.clone());
                }
                true
            } else {
                false
            }
        };

        // Started outside the borrow: the fetch outlives this frame, and the
        // next one must be able to draw while it is in flight.
        if let Some((source, w, h_px)) = fetch {
            let host = Rc::clone(&host);
            wasm_bindgen_futures::spawn_local(async move {
                let picture = images::next_picture(&source, w, h_px).await;
                let mut h = host.borrow_mut();
                h.image_fetching = false;
                match picture {
                    Some(p) => {
                        h.image_wanted = false;
                        h.deliver_image(p.image, p.title);
                    }
                    // Nothing to show yet: a hashtag nobody has posted under
                    // since we started listening. Ask again shortly.
                    None => h.image_retry_at = now_seconds() + IMAGE_RETRY_SECONDS,
                }
            });
        }
        if let Some(source) = fetch_text {
            let host = Rc::clone(&host);
            wasm_bindgen_futures::spawn_local(async move {
                let words = text::next_text(&source).await;
                let mut h = host.borrow_mut();
                h.text_fetching = false;
                match words {
                    Some(w) => {
                        h.text_wanted = false;
                        // Upstream's pipe yields a paragraph at a time with a
                        // blank line after it, and the hacks are laid out for
                        // that.
                        h.deliver_text(&format!("{}\n\n", w.trim_end()));
                    }
                    // A hashtag nobody has posted under yet, or a source that
                    // did not answer. Either way, ask again shortly; the
                    // runtime falls back to its own passage meanwhile.
                    None => h.text_retry_at = now_seconds() + IMAGE_RETRY_SECONDS,
                }
            });
        }
        if connected && let Some(window) = web_sys::window() {
            let _ = window
                .request_animation_frame(f.borrow().as_ref().unwrap().as_ref().unchecked_ref());
        }
    }) as Box<dyn FnMut()>));

    if let Some(window) = web_sys::window() {
        let _ =
            window.request_animation_frame(g.borrow().as_ref().unwrap().as_ref().unchecked_ref());
    }
}

/// Turn the panel's overrides into a query string, in a stable order.
fn to_query(settings: &BTreeMap<String, String>) -> String {
    settings
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// The address-bar query: the panel's settings plus the picture source, which
/// the panel does not own but which has to survive being written back or a
/// reload would lose it.
fn shareable_query(
    settings: &BTreeMap<String, String>,
    source: &Source,
    words: &TextSource,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(images) = source.as_param() {
        parts.push(format!("images={images}"));
    }
    if let Some(text) = words.as_param() {
        parts.push(format!("text={text}"));
    }
    let settings = to_query(settings);
    if !settings.is_empty() {
        parts.push(settings);
    }
    parts.join("&")
}

/// `/screensaver`: pick one at random and redirect to it.
///
/// A redirect rather than rendering in place, so the URL always says what you
/// are looking at and the back button behaves. Reloading `/screensaver` gives
/// you a different one; reloading `/screensaver/munch` gives you munch.
#[component]
pub fn ScreensaverRandom() -> Element {
    let nav = use_navigator();
    use_effect(move || {
        nav.replace(Route::Screensaver {
            name: savers::random().slug.to_string(),
        });
    });
    rsx! {
        Stage { message: "Picking a screensaver" }
    }
}

/// `/screensaver/:name`.
#[component]
pub fn Screensaver(name: String) -> Element {
    if savers::find(&name).is_none() {
        return rsx! {
            Stage {
                message: "No screensaver called \"{name}\"",
                Link {
                    to: Route::ScreensaverRandom {},
                    style: "color:#ff8fb8;",
                    "Try a random one"
                }
            }
        };
    }
    // Keyed so that navigating between savers remounts rather than trying to
    // reuse the canvas and the running hack.
    rsx! {
        SaverStage { key: "{name}", slug: name }
    }
}

/// A full-bleed black page with a line of text, for the states where there is
/// no saver running yet.
#[component]
fn Stage(message: String, children: Element) -> Element {
    rsx! {
        div {
            style: "position:fixed;inset:0;background:#000;color:#777;display:flex;\
                    flex-direction:column;align-items:center;justify-content:center;gap:10px;\
                    font-family:'Space Grotesk',system-ui,sans-serif;font-size:14px;",
            div { "{message}" }
            {children}
        }
    }
}

#[component]
fn SaverStage(slug: String) -> Element {
    // The route component already rejected unknown slugs; handle it again here
    // rather than assert, so a future caller cannot turn a typo into a panic.
    let Some(entry) = savers::find(&slug) else {
        return rsx! { Stage { message: "No screensaver called \"{slug}\"" } };
    };

    // Only the knobs the user actually moved, so a shared link stays short and
    // an unset option keeps following the hack's own default.
    let mut settings = use_signal(|| initial_settings(&captured_query()));
    let mut panel_open = use_signal(|| false);
    let mut paused = use_signal(|| false);
    let host: Signal<Option<Rc<RefCell<Host>>>> = use_signal(|| None);
    let mut failed = use_signal(|| false);

    // Restart the hack whenever a setting changes, and mirror the settings into
    // the URL. `replaceState` keeps the canvas alive across a slider drag.
    //
    // Hacks read their resources once, in `init`, exactly as the C does, so
    // changing an option means starting the hack again. A fresh seed each time
    // means "Reset" also re-rolls anything the hack randomises at startup.
    use_effect(move || {
        let query = to_query(&settings.read());
        let source = host
            .read()
            .as_ref()
            .map(|h| h.borrow().source.clone())
            .unwrap_or(Source::None);
        let words = host
            .read()
            .as_ref()
            .map(|h| h.borrow().text_source.clone())
            .unwrap_or_default();
        replace_query(&shareable_query(&settings.read(), &source, &words));
        let Some(h) = host.read().clone() else { return };
        // Every borrow of the host is scoped to a single statement on purpose:
        // the restart has to await the saver's chunk, and a `RefCell` borrow
        // still open at that point would panic the moment the animation frame
        // in flight touched the host.
        let unchanged = h.borrow().query == query;
        if unchanged {
            return;
        }
        spawn(async move {
            let args = h.borrow().start_args(&query);
            // Already resident by now, so this resolves without another fetch.
            match &entry.start {
                Start::Fb(start) => {
                    let Some(runner) = start(args).await else {
                        return;
                    };
                    let mut h = h.borrow_mut();
                    if let Engine::Fb(fb) = &mut h.engine {
                        fb.runner = runner;
                    }
                }
                Start::Gl3d(start) => {
                    let Some(runner) = start(args).await else {
                        return;
                    };
                    let mut h = h.borrow_mut();
                    if let Engine::Gl3d(gl) = &mut h.engine {
                        gl.restart(runner);
                    }
                }
                Start::Gl(start) => {
                    let Some(st) = start(args).await else { return };
                    let mut h = h.borrow_mut();
                    if let Engine::Gl(gl) = &mut h.engine {
                        gl.restart(st);
                    }
                }
            }
            let mut h = h.borrow_mut();
            h.query = query;
            h.announce_image_source();
        });
    });

    let onmounted = {
        let mut host = host;
        move |evt: MountedEvent| {
            spawn(async move {
                let element: web_sys::Element = evt.data().try_as_web_event().unwrap();
                let canvas: HtmlCanvasElement = element.dyn_into().unwrap();

                let css_w = canvas.client_width().max(1);
                let css_h = canvas.client_height().max(1);
                let (w, h, scale) = buffer_size(css_w, css_h);
                canvas.set_width(w as u32);
                canvas.set_height(h as u32);

                let source = Source::from_query(&captured_query());
                let text_source = TextSource::from_query(&captured_query());
                let query = to_query(&settings.peek());
                // Fetches the saver's own wasm chunk the first time.
                let args = StartArgs::new(w, h, &query, seed())
                    .with_image_host(source != Source::None)
                    .with_text_host(true)
                    .with_wall_clock(wall_clock_seconds());
                // The context has to match the saver: asking a canvas for "2d"
                // after it has given out a "webgl2" (or the other way round)
                // returns null for the rest of its life.
                let engine = match &entry.start {
                    Start::Fb(start) => {
                        let ctx = canvas
                            .get_context("2d")
                            .ok()
                            .flatten()
                            .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok());
                        match (ctx, start(args).await) {
                            (Some(ctx), Some(runner)) => {
                                Some(Engine::Fb(Box::new(FbEngine { ctx, runner })))
                            }
                            _ => None,
                        }
                    }
                    Start::Gl3d(start) => start(args)
                        .await
                        .and_then(|runner| Gl3dEngine::new(&canvas, runner))
                        .map(|gl| Engine::Gl3d(Box::new(gl))),
                    Start::Gl(start) => start(args)
                        .await
                        .and_then(|st| GlEngine::new(&canvas, st))
                        .map(|gl| Engine::Gl(Box::new(gl))),
                };
                let Some(engine) = engine else {
                    failed.set(true);
                    return;
                };
                let mut built = Host {
                    canvas,
                    engine,
                    paused: false,
                    scale,
                    css_w,
                    css_h,
                    buf_w: w,
                    buf_h: h,
                    query,
                    source,
                    image_wanted: false,
                    image_fetching: false,
                    image_retry_at: 0.0,
                    text_source,
                    text_wanted: false,
                    text_fetching: false,
                    text_retry_at: 0.0,
                    reads_text: false,
                };
                built.announce_image_source();
                let h = Rc::new(RefCell::new(built));
                host.set(Some(Rc::clone(&h)));
                start_animation_loop(h);
            });
        }
    };

    let send = move |event: XEvent| {
        if let Some(h) = host.read().clone() {
            h.borrow_mut().event(event);
        }
    };

    let (play_label, play_style) = if paused() {
        ("Play", "background:#2f7d32;border-color:#3faf43")
    } else {
        ("Pause", "background:#8a5a1a;border-color:#c98a2a")
    };

    // The saver's definition only exists once its chunk has loaded and the hack
    // has started, so the panel appears with it rather than before it.
    let def = host.read().as_ref().map(|h| h.borrow().def());
    // Where the pictures come from, and what the one on screen is called.
    let pictures = host.read().as_ref().and_then(|h| {
        let h = h.borrow();
        h.source.describe().map(|from| (from, h.image_title()))
    });
    // And where the words come from, for the hacks that read text. Only shown
    // when the saver actually asks for any.
    let words = host.read().as_ref().and_then(|h| {
        let h = h.borrow();
        h.reads_text.then(|| h.text_source.describe())
    });

    rsx! {
        div {
            style: "position:relative;width:100vw;height:100vh;overflow:hidden;background:#000;\
                    font-family:'Space Grotesk',system-ui,sans-serif;color:#eee;outline:none;",
            tabindex: 0,
            onkeydown: move |e| {
                let key = e.data().key().to_string();
                let ch = key.chars().next().filter(|_| key.chars().count() == 1).unwrap_or(' ');
                send(XEvent::KeyPress { key: ch });
            },

            canvas {
                id: "screensaver-canvas",
                // The framebuffer is at most MAX_BUFFER_EDGE across and gets
                // scaled up by the browser; nearest-neighbour keeps the pixels
                // crisp instead of smearing them.
                style: "position:absolute;inset:0;width:100%;height:100%;display:block;\
                        image-rendering:pixelated;touch-action:none;",
                onmounted,
                onpointerdown: move |e| {
                    let c = e.data().client_coordinates();
                    if let Some(h) = host.read().clone() {
                        let (x, y) = h.borrow().to_buffer(c.x, c.y);
                        h.borrow_mut().event(XEvent::ButtonPress { x, y, button: 1 });
                    }
                },
                onpointerup: move |e| {
                    let c = e.data().client_coordinates();
                    if let Some(h) = host.read().clone() {
                        let (x, y) = h.borrow().to_buffer(c.x, c.y);
                        h.borrow_mut().event(XEvent::ButtonRelease { x, y, button: 1 });
                    }
                },
                onpointermove: move |e| {
                    let c = e.data().client_coordinates();
                    if let Some(h) = host.read().clone() {
                        let (x, y) = h.borrow().to_buffer(c.x, c.y);
                        h.borrow_mut().event(XEvent::MotionNotify { x, y });
                    }
                },
            }

            if failed() {
                div {
                    style: "position:absolute;inset:0;display:flex;align-items:center;\
                            justify-content:center;color:#777;font-size:14px;",
                    "Could not load {entry.label}"
                }
            }

            {def.map(|def| rsx! {
            if !panel_open() {
                button {
                    id: "screensaver-panel-toggle",
                    // Top right, not top left: several savers print a label in
                    // the top left corner and this would sit on top of it.
                    style: "position:absolute;top:16px;right:16px;padding:8px 14px;border:1px solid #555;\
                            border-radius:10px;background:rgba(20,20,20,.6);backdrop-filter:blur(4px);\
                            box-shadow:0 4px 24px rgba(0,0,0,.5);color:#eee;cursor:pointer;\
                            font:inherit;font-size:14px;",
                    onclick: move |_| panel_open.set(true),
                    "\u{2699} Options"
                }
            }

            if panel_open() {
                div {
                    id: "screensaver-panel",
                    style: "position:absolute;top:16px;left:16px;max-height:calc(100vh - 32px);\
                            overflow:auto;padding:14px 16px;border-radius:10px;\
                            background:rgba(20,20,20,.85);backdrop-filter:blur(4px);\
                            box-shadow:0 4px 24px rgba(0,0,0,.5);user-select:none;\
                            width:min(300px,calc(100vw - 32px));",

                    div {
                        style: "display:flex;align-items:center;justify-content:space-between;margin-bottom:2px;",
                        div { style: "font-weight:700;font-size:18px;color:#ff4d8d;", "{def.label}" }
                        button {
                            style: "padding:2px 9px;border:1px solid #555;border-radius:6px;background:#333;\
                                    color:#bbb;cursor:pointer;font:inherit;font-size:13px;",
                            onclick: move |_| panel_open.set(false),
                            "\u{2715} Hide"
                        }
                    }
                    div {
                        style: "font-size:11px;color:#777;margin-bottom:10px;",
                        "{def.about.blurb}"
                    }

                    button {
                        style: "width:100%;margin-bottom:12px;padding:7px;border:1px solid;\
                                border-radius:6px;color:#fff;cursor:pointer;font:inherit;\
                                font-size:14px;font-weight:600;{play_style}",
                        onclick: move |_| {
                            let now = !paused();
                            paused.set(now);
                            if let Some(h) = host.read().clone() {
                                h.borrow_mut().paused = now;
                            }
                        },
                        "{play_label}"
                    }

                    OptionControls { def, settings }

                    div {
                        style: "display:flex;gap:8px;margin-top:12px;",
                        Link {
                            to: Route::ScreensaverRandom {},
                            style: "flex:1;padding:6px;border:1px solid #7a4bbf;border-radius:6px;\
                                    background:#3a2a55;color:#e0c8ff;cursor:pointer;font:inherit;\
                                    font-size:13px;text-align:center;text-decoration:none;",
                            "\u{1F3B2} Random"
                        }
                        button {
                            style: "flex:1;padding:6px;border:1px solid #555;border-radius:6px;\
                                    background:#203a30;color:#9dffc0;cursor:pointer;font:inherit;font-size:13px;",
                            onclick: move |_| settings.write().clear(),
                            "Reset"
                        }
                    }

                    if let Some((from , caption)) = pictures.clone() {
                        Details { summary: "Pictures",
                            div {
                                style: "font-size:12px;color:#bbb;line-height:1.5;",
                                "From {from}"
                                if let Some(caption) = caption {
                                    br {}
                                    span { style: "color:#888;font-style:italic;", "{caption}" }
                                }
                            }
                        }
                    }

                    if let Some(from) = words.clone() {
                        Details { summary: "Words",
                            div {
                                style: "font-size:12px;color:#bbb;line-height:1.5;",
                                "From {from}"
                            }
                        }
                    }

                    Details { summary: "About",
                        div {
                            style: "font-size:12px;color:#bbb;line-height:1.5;",
                            "Written by {def.about.author}; {def.about.year}."
                            br {}
                            "Ported from "
                            a {
                                href: "https://www.jwz.org/xscreensaver/",
                                target: "_blank",
                                style: "color:#8ab4ff;",
                                "XScreenSaver"
                            }
                            "."
                            if let Some(video) = def.about.video {
                                br {}
                                a {
                                    href: "{video}",
                                    target: "_blank",
                                    style: "color:#8ab4ff;",
                                    "Watch the original"
                                }
                            }
                        }
                    }

                    a {
                        href: "/",
                        style: "display:inline-block;margin-top:12px;color:#888;font-size:13px;",
                        "\u{2190} home"
                    }
                }
            }
            })}
        }
    }
}

/// The panel body: one control per option the hack declares.
#[component]
fn OptionControls(
    def: ReadSignal<&'static SaverDef>,
    settings: Signal<BTreeMap<String, String>>,
) -> Element {
    let def = def();
    if def.opts.is_empty() {
        return rsx! {
            div { style: "font-size:12px;color:#777;", "This one has nothing to configure." }
        };
    }

    rsx! {
        for opt in def.opts {
            match opt.kind {
                OptKind::Slider { low, high, step, decimals, invert } => rsx! {
                    Slider {
                        key: "{opt.key}",
                        label: "{opt.label}",
                        min: "{low}",
                        max: "{high}",
                        step: "{step}",
                        decimals,
                        // An inverted slider runs the other way from the value
                        // it sets: bigger delay, lower frame rate.
                        value: {
                            let v = current_number(&settings, opt.key, opt.default);
                            if invert { low + high - v } else { v }
                        },
                        oninput: move |v: f64| {
                            let v = if invert { low + high - v } else { v };
                            settings.write().insert(
                                opt.key.to_string(),
                                format!("{:.1$}", v, decimals as usize),
                            );
                        },
                    }
                },
                OptKind::Spin { low, high } => rsx! {
                    Slider {
                        key: "{opt.key}",
                        label: "{opt.label}",
                        min: "{low}",
                        max: "{high}",
                        step: "1",
                        decimals: 0,
                        value: current_number(&settings, opt.key, opt.default),
                        oninput: move |v: f64| {
                            settings.write().insert(opt.key.to_string(), format!("{v:.0}"));
                        },
                    }
                },
                OptKind::Bool => rsx! {
                    div {
                        key: "{opt.key}",
                        style: "display:flex;margin-bottom:8px;",
                        Toggle {
                            label: "{opt.label}",
                            on: current_bool(&settings, opt.key, opt.default),
                            onclick: move |_| {
                                let now = current_bool(&settings, opt.key, opt.default);
                                settings.write().insert(
                                    opt.key.to_string(),
                                    if now { "false".into() } else { "true".into() },
                                );
                            },
                        }
                    }
                },
                OptKind::Select(items) => rsx! {
                    Choice {
                        key: "{opt.key}",
                        label: "{opt.label}",
                        value: current_string(&settings, opt.key, opt.default),
                        options: items.iter().map(|i| (i.value.to_string(), i.label.to_string())).collect::<Vec<_>>(),
                        onchange: move |v: String| {
                            settings.write().insert(opt.key.to_string(), v);
                        },
                    }
                },
            }
        }
    }
}

fn current_string(settings: &Signal<BTreeMap<String, String>>, key: &str, default: &str) -> String {
    settings
        .read()
        .get(key)
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

fn current_number(settings: &Signal<BTreeMap<String, String>>, key: &str, default: &str) -> f64 {
    current_string(settings, key, default)
        .parse()
        .unwrap_or(0.0)
}

fn current_bool(settings: &Signal<BTreeMap<String, String>>, key: &str, default: &str) -> bool {
    matches!(
        current_string(settings, key, default)
            .to_ascii_lowercase()
            .as_str(),
        "true" | "yes" | "on" | "1"
    )
}

/// Query parameters the host owns rather than the saver. The panel must not
/// adopt them as settings, or it would write them back a second time.
const RESERVED_PARAMS: &[&str] = &["images"];

/// Seed the panel from a shared link's query string.
fn initial_settings(query: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(params) = web_sys::UrlSearchParams::new_with_str(query) else {
        return out;
    };
    // `UrlSearchParams` has no iterator in web-sys, so walk the raw string and
    // use it only to decode each value.
    for pair in query.trim_start_matches('?').split('&') {
        let Some((k, _)) = pair.split_once('=') else {
            continue;
        };
        if RESERVED_PARAMS.contains(&k) {
            continue;
        }
        if let Some(v) = params.get(k) {
            out.insert(k.to_string(), v);
        }
    }
    out
}
