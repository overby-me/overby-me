//! The screensaver stage: `/screensaver` and `/screensaver/:name`.
//!
//! One canvas, one `requestAnimationFrame` loop, and one options panel behind a
//! button, driving whichever [`SaverDef`] the router asked for. None of this is
//! lazily loaded; only the savers themselves are (see [`crate::pages::savers`]),
//! so the shared runtime is downloaded once however many you look at.
//!
//! The saver draws into a software framebuffer, which this blits to the canvas
//! with `putImageData` once per animation frame.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use dioxus::prelude::*;
use dioxus::web::WebEventExt;
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};
use xscreensaver::SaverDef;
use xscreensaver::runtime::{OptKind, Runner, StartArgs, XEvent};

use crate::Route;
use crate::pages::savers;
use crate::pages::ui::{Choice, Details, Slider, Toggle};
use crate::url::{captured_query, replace_query};

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

struct Host {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    runner: Runner,
    paused: bool,
    /// CSS pixels per framebuffer pixel, for mapping pointer coordinates back.
    scale: i32,
    css_w: i32,
    css_h: i32,
    /// The query the running hack was started with, so that the settings effect
    /// firing for an unrelated reason does not restart it.
    query: String,
}

impl Host {
    fn def(&self) -> &'static SaverDef {
        self.runner.def()
    }

    fn start_args(&self, query: &str) -> StartArgs {
        StartArgs::new(
            self.runner.dpy.width(),
            self.runner.dpy.height(),
            query,
            seed(),
        )
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
        self.canvas.set_width(w as u32);
        self.canvas.set_height(h as u32);
        self.runner.resize(w, h);
    }

    fn blit(&self) {
        let w = self.runner.dpy.width() as u32;
        let h = self.runner.dpy.height() as u32;
        let bytes = self.runner.frame_bytes();
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
        if let Err(e) = self.ctx.put_image_data(&img, 0.0, 0.0) {
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
        let connected = {
            let mut h = host.borrow_mut();
            if h.canvas.is_connected() {
                h.sync_size();
                if !h.paused {
                    let now = now_seconds();
                    h.runner.tick(now);
                }
                h.blit();
                true
            } else {
                false
            }
        };
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
        replace_query(&query);
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
            if let Some(runner) = (entry.start)(args).await {
                let mut h = h.borrow_mut();
                h.runner = runner;
                h.query = query;
            }
        });
    });

    let onmounted = {
        let mut host = host;
        move |evt: MountedEvent| {
            spawn(async move {
                let element: web_sys::Element = evt.data().try_as_web_event().unwrap();
                let canvas: HtmlCanvasElement = element.dyn_into().unwrap();
                let Some(ctx) = canvas
                    .get_context("2d")
                    .ok()
                    .flatten()
                    .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
                else {
                    return;
                };

                let css_w = canvas.client_width().max(1);
                let css_h = canvas.client_height().max(1);
                let (w, h, scale) = buffer_size(css_w, css_h);
                canvas.set_width(w as u32);
                canvas.set_height(h as u32);

                let query = to_query(&settings.peek());
                // Fetches the saver's own wasm chunk the first time.
                let Some(runner) = (entry.start)(StartArgs::new(w, h, &query, seed())).await else {
                    failed.set(true);
                    return;
                };
                let h = Rc::new(RefCell::new(Host {
                    canvas,
                    ctx,
                    runner,
                    paused: false,
                    scale,
                    css_w,
                    css_h,
                    query,
                }));
                host.set(Some(Rc::clone(&h)));
                start_animation_loop(h);
            });
        }
    };

    let send = move |event: XEvent| {
        if let Some(h) = host.read().clone() {
            h.borrow_mut().runner.event(event);
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
                        h.borrow_mut().runner.event(XEvent::ButtonPress { x, y, button: 1 });
                    }
                },
                onpointerup: move |e| {
                    let c = e.data().client_coordinates();
                    if let Some(h) = host.read().clone() {
                        let (x, y) = h.borrow().to_buffer(c.x, c.y);
                        h.borrow_mut().runner.event(XEvent::ButtonRelease { x, y, button: 1 });
                    }
                },
                onpointermove: move |e| {
                    let c = e.data().client_coordinates();
                    if let Some(h) = host.read().clone() {
                        let (x, y) = h.borrow().to_buffer(c.x, c.y);
                        h.borrow_mut().runner.event(XEvent::MotionNotify { x, y });
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
                    style: "position:absolute;top:16px;left:16px;padding:8px 14px;border:1px solid #555;\
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
        if let Some(v) = params.get(k) {
            out.insert(k.to_string(), v);
        }
    }
    out
}
