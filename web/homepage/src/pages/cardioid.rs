//! Cardioid / epicyclic curve tracer.
//!
//! A Dioxus + WASM port of pysim's `main3.py`: the traced point is the sum of
//! two rotating arms,
//!
//! ```text
//! OP = (r1·cos(w1·t) + r2·cos(w2·t) + cx,  r1·sin(w1·t) + r2·sin(w2·t) + cy)
//! ```
//!
//! Successive points are stroked onto a persistent trace buffer (an off-screen
//! canvas, the analogue of pygame's `dsurface`); each frame the visible canvas
//! is cleared, the trace composited on top when "Draw" is on, and the two
//! construction circles / dot / scale bar / speed drawn as a live overlay.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;
use dioxus::web::WebEventExt;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

const TAU: f64 = std::f64::consts::TAU;
/// Radius (px) of the little construction dots, fixed as in pysim (`r3`).
const DOT_R: f64 = 5.0;
/// pysim's `pixelpermeter`, used only for the speed readout / scale bar.
const PIXELS_PER_METER: f64 = 6200.0;

#[derive(Clone, PartialEq)]
struct Settings {
    r1: f64,
    w1: f64,
    r2: f64,
    w2: f64,
    time: f64,
    calrate: f64,
    linewidth: f64,
    draw: bool,
    clean: bool,
    drawdot: bool,
    circles: bool,
    showspeed: bool,
    colors: bool,
    antialiasing: bool,
    sandbox: bool,
}

impl Default for Settings {
    fn default() -> Self {
        // pysim starts everything at rest (time = 0), which shows a blank
        // screen. Seed a live cardioid instead: two equal arms at a 1:2
        // frequency ratio, animating, with the construction circles shown.
        Self {
            r1: 140.0,
            r2: 140.0,
            w1: 1.0,
            w2: 2.0,
            time: 30.0,
            calrate: 2.0,
            linewidth: 2.0,
            draw: true,
            clean: false,
            drawdot: true,
            circles: true,
            showspeed: false,
            colors: false,
            antialiasing: true,
            sandbox: false,
        }
    }
}

impl Settings {
    /// The geometry-defining parameters. When these change the traced curve is
    /// different, so the accumulated trace is cleared (pysim clears `dsurface`
    /// on any slider drag; we keep the drawing when only speed/width change).
    fn geometry(&self) -> (f64, f64, f64, f64) {
        (self.r1, self.w1, self.r2, self.w2)
    }
}

struct Sim {
    settings: Settings,
    /// Curve parameter, advanced every sub-step.
    t: f64,
    /// Previous traced point, or None right after a reset.
    last: Option<(f64, f64)>,
    /// Most recent speed magnitude (px/param-unit) for the readout.
    speed: f64,
    /// Logical (CSS-pixel) canvas size; drawing happens in this space.
    w: f64,
    h: f64,
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    trace_canvas: HtmlCanvasElement,
    trace: CanvasRenderingContext2d,
}

impl Sim {
    fn center(&self) -> (f64, f64) {
        (self.w / 2.0, self.h / 2.0)
    }

    /// Clear the accumulated trace and restart from the current parameters.
    fn reset_trace(&mut self) {
        self.trace.clear_rect(0.0, 0.0, self.w, self.h);
        self.last = None;
    }

    /// Advance the simulation and render one frame.
    fn frame(&mut self) {
        let s = self.settings.clone();
        let (cx, cy) = self.center();
        let steps = (s.calrate.max(1.0)) as u32;
        // Sandbox uses the raw time step; otherwise it is scaled down so the
        // slider's 0..100 range maps to a gentle speed.
        let dt = if s.sandbox {
            s.time / s.calrate
        } else {
            s.time / 1000.0 / s.calrate
        };

        // One color for the whole frame lets the common (non-random) case batch
        // every sub-step into a single stroked path.
        let batched = !s.colors;
        if batched {
            self.trace.set_stroke_style_str("red");
            self.trace.set_line_width(s.linewidth);
            self.trace.begin_path();
            if let Some((lx, ly)) = self.last {
                self.trace.move_to(lx, ly);
            }
        }

        for _ in 0..steps {
            self.t += dt;
            let opx = s.r1 * (s.w1 * self.t).cos() + s.r2 * (s.w2 * self.t).cos() + cx;
            let opy = s.r1 * (s.w1 * self.t).sin() + s.r2 * (s.w2 * self.t).sin() + cy;
            let (px, py) = if s.antialiasing {
                (opx, opy)
            } else {
                (opx.round(), opy.round())
            };

            if batched {
                if self.last.is_none() {
                    self.trace.move_to(px, py);
                } else {
                    self.trace.line_to(px, py);
                }
            } else if let Some((lx, ly)) = self.last {
                self.trace.set_stroke_style_str(&random_color());
                self.trace.set_line_width(s.linewidth);
                self.trace.begin_path();
                self.trace.move_to(lx, ly);
                self.trace.line_to(px, py);
                self.trace.stroke();
            }
            self.last = Some((px, py));

            if s.showspeed {
                let vx = -s.r1 * s.w1 * (self.t * s.w1).sin() - s.r2 * s.w2 * (self.t * s.w2).sin();
                let vy = s.r1 * s.w1 * (self.t * s.w1).cos() + s.r2 * s.w2 * (self.t * s.w2).cos();
                self.speed = vx.hypot(vy);
            }
        }
        if batched {
            self.trace.stroke();
        }

        // Overlay: clear, composite the trace, then draw the live construction.
        let ctx = &self.ctx;
        ctx.set_fill_style_str("#000000");
        ctx.fill_rect(0.0, 0.0, self.w, self.h);
        if s.draw {
            let _ = ctx.draw_image_with_html_canvas_element_and_dw_and_dh(
                &self.trace_canvas,
                0.0,
                0.0,
                self.w,
                self.h,
            );
        }

        if s.circles {
            let c2x = s.r1 * (s.w1 * self.t).cos() + cx;
            let c2y = s.r1 * (s.w1 * self.t).sin() + cy;
            ctx.set_line_width(1.0);
            ctx.set_stroke_style_str("green");
            ctx.set_fill_style_str("green");
            stroke_circle(ctx, cx, cy, s.r1);
            fill_circle(ctx, cx, cy, DOT_R);
            ctx.set_stroke_style_str("purple");
            ctx.set_fill_style_str("purple");
            stroke_circle(ctx, c2x, c2y, s.r2);
            fill_circle(ctx, c2x, c2y, DOT_R);
            ctx.set_stroke_style_str("red");
            ctx.set_line_width(s.linewidth);
            draw_line(ctx, cx, cy, c2x, c2y);
        }

        if s.drawdot
            && let Some((px, py)) = self.last
        {
            ctx.set_fill_style_str("yellow");
            fill_circle(ctx, px, py, DOT_R);
        }

        // Scale bar: a 100px reference labelled in millimetres.
        ctx.set_fill_style_str("#ffffff");
        ctx.set_stroke_style_str("#ffffff");
        ctx.set_line_width(1.0);
        ctx.set_font("20px 'Space Grotesk', system-ui, sans-serif");
        ctx.set_text_baseline("top");
        ctx.set_text_align("center");
        let _ = ctx.fill_text(
            &format!("{:.1}mm", 100.0 / PIXELS_PER_METER * 1000.0),
            70.0,
            self.h - 62.0,
        );
        draw_line(ctx, 20.0, self.h - 40.0, 120.0, self.h - 40.0);
        draw_line(ctx, 20.0, self.h - 44.0, 20.0, self.h - 36.0);
        draw_line(ctx, 120.0, self.h - 44.0, 120.0, self.h - 36.0);

        if s.showspeed {
            ctx.set_text_align("right");
            let mm_s = self.speed / PIXELS_PER_METER * 1000.0;
            let _ = ctx.fill_text(
                &format!("Speed: {mm_s:.0} mm/s"),
                self.w - 20.0,
                self.h - 62.0,
            );
        }

        // pysim wipes `dsurface` at the end of the frame while "Clean" is on, so
        // only the current frame's fresh segments are ever shown.
        if s.clean {
            self.trace.clear_rect(0.0, 0.0, self.w, self.h);
        }
    }
}

fn stroke_circle(ctx: &CanvasRenderingContext2d, x: f64, y: f64, r: f64) {
    ctx.begin_path();
    let _ = ctx.arc(x, y, r.max(0.5), 0.0, TAU);
    ctx.stroke();
}

fn fill_circle(ctx: &CanvasRenderingContext2d, x: f64, y: f64, r: f64) {
    ctx.begin_path();
    let _ = ctx.arc(x, y, r.max(0.5), 0.0, TAU);
    ctx.fill();
}

fn draw_line(ctx: &CanvasRenderingContext2d, x1: f64, y1: f64, x2: f64, y2: f64) {
    ctx.begin_path();
    ctx.move_to(x1, y1);
    ctx.line_to(x2, y2);
    ctx.stroke();
}

/// A bright random color, matching pysim's `randint(50, 255)` per channel.
fn random_color() -> String {
    let ch = || 50 + (js_sys::Math::random() * 206.0) as u32;
    format!("rgb({},{},{})", ch(), ch(), ch())
}

fn context_2d(canvas: &HtmlCanvasElement) -> CanvasRenderingContext2d {
    canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into::<CanvasRenderingContext2d>()
        .unwrap()
}

type AnimationClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// requestAnimationFrame loop; stops itself once the canvas leaves the document
/// (e.g. the user navigates away from `/cardioid`).
fn start_animation_loop(sim: Rc<RefCell<Sim>>) {
    let f: AnimationClosure = Rc::new(RefCell::new(None));
    let g = Rc::clone(&f);

    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        let connected = {
            let mut s = sim.borrow_mut();
            if s.canvas.is_connected() {
                s.frame();
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

#[component]
pub fn Cardioid() -> Element {
    let mut settings = use_signal(Settings::default);
    let state: Signal<Option<Rc<RefCell<Sim>>>> = use_signal(|| None);

    // Push setting changes into the running simulation, clearing the trace when
    // the curve geometry (not just speed/width) changed.
    use_effect(move || {
        let s = settings.read().clone();
        if let Some(sim) = state.read().clone() {
            let mut sim = sim.borrow_mut();
            let shape_changed = sim.settings.geometry() != s.geometry();
            sim.settings = s;
            if shape_changed {
                sim.reset_trace();
            }
        }
    });

    let onmounted = {
        let mut state = state;
        move |evt: MountedEvent| {
            spawn(async move {
                let elem: web_sys::Element = evt.data().try_as_web_event().unwrap();
                let canvas: HtmlCanvasElement = elem.dyn_into().unwrap();

                let dpr = web_sys::window()
                    .map(|w| w.device_pixel_ratio())
                    .unwrap_or(1.0);
                let css_w = canvas.client_width() as f64;
                let css_h = canvas.client_height() as f64;
                canvas.set_width((css_w * dpr) as u32);
                canvas.set_height((css_h * dpr) as u32);

                // Off-screen trace buffer, same physical size as the canvas.
                let Some(document) = web_sys::window().and_then(|w| w.document()) else {
                    return;
                };
                let Some(trace_canvas) = document
                    .create_element("canvas")
                    .ok()
                    .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
                else {
                    return;
                };
                trace_canvas.set_width((css_w * dpr) as u32);
                trace_canvas.set_height((css_h * dpr) as u32);

                let ctx = context_2d(&canvas);
                let trace = context_2d(&trace_canvas);
                // Draw in CSS pixels; the dpr scale keeps it crisp on HiDPI. The
                // trace composites 1:1 because it shares the physical size.
                let _ = ctx.scale(dpr, dpr);
                let _ = trace.scale(dpr, dpr);
                ctx.set_line_cap("round");
                ctx.set_line_join("round");
                trace.set_line_cap("round");
                trace.set_line_join("round");

                let sim = Rc::new(RefCell::new(Sim {
                    settings: settings.peek().clone(),
                    t: 0.0,
                    last: None,
                    speed: 0.0,
                    w: css_w,
                    h: css_h,
                    canvas,
                    ctx,
                    trace_canvas,
                    trace,
                }));
                state.set(Some(Rc::clone(&sim)));
                start_animation_loop(sim);
            });
        }
    };

    let s = settings.read();
    let toggle_bg = |on: bool| {
        if on {
            "background:#2f7d32;border-color:#3faf43"
        } else {
            "background:#333;border-color:#555"
        }
    };

    rsx! {
        div {
            style: "position:relative;width:100vw;height:100vh;overflow:hidden;background:#000;\
                    font-family:'Space Grotesk',system-ui,sans-serif;color:#eee;",
            canvas {
                id: "cardioid-canvas",
                style: "position:absolute;inset:0;width:100%;height:100%;display:block;",
                onmounted: onmounted,
            }

            // Control panel.
            div {
                style: "position:absolute;top:16px;left:16px;max-height:calc(100vh - 32px);\
                        overflow:auto;padding:14px 16px;border-radius:10px;\
                        background:rgba(20,20,20,.82);backdrop-filter:blur(4px);\
                        box-shadow:0 4px 24px rgba(0,0,0,.5);width:290px;user-select:none;",
                div {
                    style: "font-weight:700;font-size:18px;margin-bottom:10px;color:#ff4d8d;",
                    "Cardioid"
                }

                Slider { label: "Center circle radius (r₁)", min: "1", max: "300", step: "1",
                    value: s.r1, decimals: 0, oninput: move |v| settings.write().r1 = v }
                Slider { label: "Center circle speed (ω₁)", min: "-200", max: "200", step: "0.1",
                    value: s.w1, decimals: 1, oninput: move |v| settings.write().w1 = v }
                Slider { label: "Rotating circle radius (r₂)", min: "1", max: "300", step: "1",
                    value: s.r2, decimals: 0, oninput: move |v| settings.write().r2 = v }
                Slider { label: "Rotating circle speed (ω₂)", min: "-200", max: "200", step: "0.1",
                    value: s.w2, decimals: 1, oninput: move |v| settings.write().w2 = v }
                Slider { label: "Time", min: "0", max: "100", step: "0.5",
                    value: s.time, decimals: 1, oninput: move |v| settings.write().time = v }
                Slider { label: "Calc / frame", min: "1", max: "1000", step: "1",
                    value: s.calrate, decimals: 0, oninput: move |v| settings.write().calrate = v }
                Slider { label: "Line width", min: "1", max: "10", step: "1",
                    value: s.linewidth, decimals: 0, oninput: move |v| settings.write().linewidth = v }

                div {
                    style: "display:flex;flex-wrap:wrap;gap:6px;margin-top:12px;",
                    for (name , on) in [
                        ("Draw", s.draw),
                        ("Clean", s.clean),
                        ("Dot", s.drawdot),
                        ("Circles", s.circles),
                        ("Speed", s.showspeed),
                        ("Colors", s.colors),
                        ("Antialias", s.antialiasing),
                        ("Sandbox", s.sandbox),
                    ] {
                        button {
                            style: "flex:0 0 auto;padding:5px 10px;border:1px solid;border-radius:6px;\
                                    color:#eee;cursor:pointer;font:inherit;font-size:13px;{toggle_bg(on)}",
                            onclick: move |_| {
                                let mut w = settings.write();
                                match name {
                                    "Draw" => w.draw = !w.draw,
                                    "Clean" => w.clean = !w.clean,
                                    "Dot" => w.drawdot = !w.drawdot,
                                    "Circles" => w.circles = !w.circles,
                                    "Speed" => w.showspeed = !w.showspeed,
                                    "Colors" => w.colors = !w.colors,
                                    "Antialias" => w.antialiasing = !w.antialiasing,
                                    "Sandbox" => w.sandbox = !w.sandbox,
                                    _ => {}
                                }
                            },
                            "{name}"
                        }
                    }
                }

                button {
                    style: "margin-top:12px;width:100%;padding:6px;border:1px solid #555;\
                            border-radius:6px;background:#3a2030;color:#ff9dc0;cursor:pointer;\
                            font:inherit;font-size:13px;",
                    // Nudging r1 by nothing still triggers the sync effect's
                    // shape check to false, so clear the trace directly.
                    onclick: move |_| {
                        if let Some(sim) = state.read().clone() {
                            sim.borrow_mut().reset_trace();
                        }
                    },
                    "Clear trace"
                }

                a {
                    href: "/",
                    style: "display:inline-block;margin-top:12px;color:#888;font-size:13px;",
                    "← home"
                }
            }
        }
    }
}

#[component]
fn Slider(
    label: String,
    min: String,
    max: String,
    step: String,
    value: f64,
    decimals: u8,
    oninput: EventHandler<f64>,
) -> Element {
    let shown = format!("{:.1$}", value, decimals as usize);
    rsx! {
        div {
            style: "margin-bottom:8px;",
            div {
                style: "display:flex;justify-content:space-between;font-size:12px;color:#bbb;margin-bottom:2px;",
                span { "{label}" }
                span { style: "color:#ff8fb8;font-variant-numeric:tabular-nums;", "{shown}" }
            }
            input {
                r#type: "range",
                min,
                max,
                step,
                value: "{value}",
                style: "width:100%;accent-color:#ff4d8d;",
                oninput: move |e| {
                    if let Ok(v) = e.value().parse::<f64>() {
                        oninput.call(v);
                    }
                },
            }
        }
    }
}
