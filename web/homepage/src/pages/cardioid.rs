//! Cardioid / epicyclic curve tracer.
//!
//! A Dioxus + WASM art toy built around a sum of rotating arms (a truncated
//! Fourier series): the pen at parameter `t` is
//!
//! ```text
//! P(t) = center + Σ dampⁿ · rₙ · (cos(wₙ·t + φₙ), sin(wₙ·t + φₙ))
//! ```
//!
//! Points are sampled at a step `Δt` and connected with lines onto a persistent
//! trace buffer (the analogue of pygame's `dsurface`). A large `Δt` samples the
//! curve stroboscopically, which is where the star-polygon / moiré patterns come
//! from. A separate "times-table" mode draws the classic modular chords
//! (connect `i` to `k·i mod N` on a circle).
//!
//! The panel is simple by default (a two-arm cardioid); every advanced knob
//! lives in a collapsed `<details>` section.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;
use dioxus::web::WebEventExt;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::pages::ui::{Details, Slider, Toggle};
use crate::url::captured_query;

const TAU: f64 = std::f64::consts::TAU;
/// Radius (px) of the little construction dots.
const DOT_R: f64 = 5.0;
/// Fixed stroke width for the trace and construction lines.
const LINE_WIDTH: f64 = 1.0;
/// Upper bound on the number of rotating arms.
const MAX_ARMS: usize = 6;

/// A tiny xorshift PRNG seeded from `js_sys::Math::random`, so "Randomize" and
/// per-segment colors vary without pulling in a rng crate.
fn rand() -> f64 {
    js_sys::Math::random()
}

/// What drives the geometry.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Sum of rotating arms.
    Epicycle,
    /// Modular "times-table" chords on a circle.
    TimesTable,
}

impl Mode {
    fn tag(self) -> &'static str {
        match self {
            Mode::Epicycle => "epi",
            Mode::TimesTable => "tt",
        }
    }
    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "epi" => Some(Mode::Epicycle),
            "tt" => Some(Mode::TimesTable),
            _ => None,
        }
    }
}

/// How each segment is colored.
#[derive(Clone, Copy, PartialEq)]
enum Fill {
    /// The fixed `color`.
    Solid,
    /// Hue cycles with the parameter `t`.
    HueTime,
    /// Hue mapped from the pen's speed.
    HueSpeed,
    /// A fresh random color per segment.
    Random,
}

impl Fill {
    fn tag(self) -> &'static str {
        match self {
            Fill::Solid => "solid",
            Fill::HueTime => "huet",
            Fill::HueSpeed => "hues",
            Fill::Random => "rand",
        }
    }
    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "solid" => Some(Fill::Solid),
            "huet" => Some(Fill::HueTime),
            "hues" => Some(Fill::HueSpeed),
            "rand" => Some(Fill::Random),
            _ => None,
        }
    }
}

/// One rotating arm: radius, angular velocity, phase offset.
#[derive(Clone, PartialEq)]
struct Arm {
    r: f64,
    w: f64,
    phase: f64,
}

#[derive(Clone, PartialEq)]
struct Settings {
    mode: Mode,
    arms: Vec<Arm>,
    /// Parameter step between sampled points (small = smooth, large = strobed).
    step: f64,
    /// Samples drawn per animation frame.
    speed: f64,
    /// Exponential radius decay `e^(-damping·t)` (0 = none). Harmonograph look.
    damping: f64,
    /// k-fold rotational symmetry (1 = none).
    symmetry: u32,
    // Times-table mode.
    tt_points: f64,
    tt_mult: f64,
    tt_radius: f64,
    // Aesthetics.
    color: String,
    fill: Fill,
    glow: bool,
    /// Per-frame fade of the trace (0 = accumulate forever, 1 = wipe each frame).
    fade: f64,
    // Overlay toggles.
    draw: bool,
    drawdot: bool,
    circles: bool,
    antialiasing: bool,
}

impl Default for Settings {
    fn default() -> Self {
        // The simple default: a live two-arm cardioid, drawn smoothly.
        Self {
            mode: Mode::Epicycle,
            arms: vec![
                Arm {
                    r: 140.0,
                    w: 1.0,
                    phase: 0.0,
                },
                Arm {
                    r: 140.0,
                    w: 2.0,
                    phase: 0.0,
                },
            ],
            step: 0.02,
            speed: 3.0,
            damping: 0.0,
            symmetry: 1,
            tt_points: 200.0,
            tt_mult: 2.0,
            tt_radius: 300.0,
            color: "#ff4d8d".to_string(),
            fill: Fill::Solid,
            glow: false,
            fade: 0.0,
            draw: true,
            drawdot: true,
            circles: true,
            antialiasing: true,
        }
    }
}

impl Settings {
    /// The parameters that change the *drawn figure* (as opposed to purely
    /// cosmetic ones). When any of these change, the accumulated trace is
    /// cleared and redrawn.
    #[allow(clippy::type_complexity)]
    fn figure(&self) -> (Mode, Vec<Arm>, u64, u64, u32, u64, u64, u64) {
        (
            self.mode,
            self.arms.clone(),
            self.step.to_bits(),
            self.damping.to_bits(),
            self.symmetry,
            self.tt_points.to_bits(),
            self.tt_mult.to_bits(),
            self.tt_radius.to_bits(),
        )
    }

    /// Serialize to a shareable query string (no leading `?`).
    fn to_query(&self) -> String {
        let b = |v: bool| if v { "1" } else { "0" };
        let arms = self
            .arms
            .iter()
            .map(|a| format!("{},{},{}", a.r, a.w, a.phase))
            .collect::<Vec<_>>()
            .join(";");
        format!(
            "mode={}&arms={}&step={}&speed={}&damp={}&sym={}&ttn={}&ttk={}&ttr={}&fill={}&glow={}&fade={}&draw={}&dot={}&circ={}&aa={}&color={}",
            self.mode.tag(),
            arms,
            self.step,
            self.speed,
            self.damping,
            self.symmetry,
            self.tt_points,
            self.tt_mult,
            self.tt_radius,
            self.fill.tag(),
            b(self.glow),
            self.fade,
            b(self.draw),
            b(self.drawdot),
            b(self.circles),
            b(self.antialiasing),
            self.color.trim_start_matches('#'),
        )
    }

    /// Parse settings from a URL query string, falling back to defaults for any
    /// parameter that is missing or malformed.
    fn from_query(query: &str) -> Self {
        let mut s = Self::default();
        let Ok(p) = web_sys::UrlSearchParams::new_with_str(query) else {
            return s;
        };
        let num = |k: &str| p.get(k).and_then(|v| v.parse::<f64>().ok());
        let flag = |k: &str| p.get(k).map(|v| v == "1" || v == "true");

        if let Some(m) = p.get("mode").and_then(|v| Mode::from_tag(&v)) {
            s.mode = m;
        }
        if let Some(arms) = p.get("arms") {
            let parsed: Vec<Arm> = arms
                .split(';')
                .filter_map(|a| {
                    let mut it = a.split(',').map(|v| v.parse::<f64>().ok());
                    Some(Arm {
                        r: it.next()??,
                        w: it.next()??,
                        phase: it.next().flatten().unwrap_or(0.0),
                    })
                })
                .take(MAX_ARMS)
                .collect();
            if !parsed.is_empty() {
                s.arms = parsed;
            }
        }
        if let Some(v) = num("step") {
            s.step = v;
        }
        if let Some(v) = num("speed") {
            s.speed = v;
        }
        if let Some(v) = num("damp") {
            s.damping = v;
        }
        if let Some(v) = num("sym") {
            s.symmetry = (v as u32).max(1);
        }
        if let Some(v) = num("ttn") {
            s.tt_points = v;
        }
        if let Some(v) = num("ttk") {
            s.tt_mult = v;
        }
        if let Some(v) = num("ttr") {
            s.tt_radius = v;
        }
        if let Some(f) = p.get("fill").and_then(|v| Fill::from_tag(&v)) {
            s.fill = f;
        }
        if let Some(v) = flag("glow") {
            s.glow = v;
        }
        if let Some(v) = num("fade") {
            s.fade = v;
        }
        if let Some(v) = flag("draw") {
            s.draw = v;
        }
        if let Some(v) = flag("dot") {
            s.drawdot = v;
        }
        if let Some(v) = flag("circ") {
            s.circles = v;
        }
        if let Some(v) = flag("aa") {
            s.antialiasing = v;
        }
        if let Some(c) = p.get("color") {
            let c = c.trim_start_matches('#');
            if c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()) {
                s.color = format!("#{c}");
            }
        }
        s
    }
}

/// A named preset (label, settings).
fn presets() -> Vec<(&'static str, Settings)> {
    let arm = |r: f64, w: f64, phase: f64| Arm { r, w, phase };
    let base = Settings::default;
    vec![
        ("Cardioid", Settings::default()),
        (
            // The old "turbo" feel: high, incommensurate frequencies sampled
            // coarsely (heavy aliasing) and drawn fast, with a light fade so it
            // shimmers erratically rather than settling. Nudge `fade` toward 0
            // to accumulate a dense web, or toward 1 for a live scribble.
            "Turbo",
            Settings {
                arms: vec![arm(190.0, 13.3, 0.0), arm(190.0, 198.5, 0.0)],
                step: 0.069,
                speed: 700.0,
                fade: 0.07,
                fill: Fill::HueSpeed,
                circles: false,
                drawdot: false,
                ..base()
            },
        ),
        (
            "Rose",
            Settings {
                arms: vec![arm(150.0, 1.0, 0.0), arm(150.0, 6.0, 0.0)],
                ..base()
            },
        ),
        (
            "Spirograph",
            Settings {
                arms: vec![arm(150.0, 1.0, 0.0), arm(60.0, 7.0, 0.0)],
                ..base()
            },
        ),
        (
            "Star (strobe)",
            Settings {
                arms: vec![arm(180.0, 1.0, 0.0), arm(60.0, 2.0, 0.0)],
                step: 0.74,
                speed: 40.0,
                glow: true,
                fill: Fill::HueTime,
                ..base()
            },
        ),
        (
            "Harmonograph",
            Settings {
                arms: vec![
                    arm(150.0, 2.0, 0.0),
                    arm(120.0, 3.01, 1.2),
                    arm(60.0, 5.0, 0.5),
                ],
                damping: 0.06,
                fade: 0.02,
                circles: false,
                fill: Fill::HueSpeed,
                ..base()
            },
        ),
        (
            "Kaleidoscope",
            Settings {
                arms: vec![arm(120.0, 3.0, 0.0), arm(80.0, -5.0, 0.6)],
                symmetry: 6,
                glow: true,
                fill: Fill::HueTime,
                circles: false,
                ..base()
            },
        ),
        (
            "Times-table",
            Settings {
                mode: Mode::TimesTable,
                tt_points: 220.0,
                tt_mult: 2.0,
                tt_radius: 320.0,
                glow: true,
                fill: Fill::HueTime,
                ..base()
            },
        ),
    ]
}

/// A tasteful random configuration.
fn randomized() -> Settings {
    let mut s = Settings::default();
    if rand() < 0.25 {
        // A times-table figure.
        s.mode = Mode::TimesTable;
        s.tt_points = (60.0 + rand() * 260.0).round();
        s.tt_mult = (2.0 + rand() * 20.0 * rand()).round().max(2.0);
        s.tt_radius = 260.0 + rand() * 120.0;
    } else {
        let n = 2 + (rand() * 3.0) as usize; // 2..=4 arms
        s.arms = (0..n)
            .map(|_| {
                let mut w = (rand() * 16.0 - 8.0).round();
                if w == 0.0 {
                    w = 1.0;
                }
                Arm {
                    r: 30.0 + rand() * 150.0,
                    w,
                    phase: rand() * TAU,
                }
            })
            .collect();
        // Half the time, strobe; otherwise a smooth curve.
        s.step = if rand() < 0.5 {
            0.01 + rand() * 0.08
        } else {
            0.25 + rand() * 1.2
        };
        s.speed = 4.0 + rand() * 60.0;
        if rand() < 0.4 {
            s.symmetry = 2 + (rand() * 7.0) as u32;
        }
        if rand() < 0.3 {
            s.damping = 0.02 + rand() * 0.08;
            s.fade = 0.02;
        }
        s.circles = false;
    }
    s.glow = rand() < 0.6;
    s.fill = match (rand() * 4.0) as u32 {
        0 => Fill::Solid,
        1 => Fill::HueTime,
        2 => Fill::HueSpeed,
        _ => Fill::Random,
    };
    s.color = format!(
        "#{:02x}{:02x}{:02x}",
        50 + (rand() * 205.0) as u32,
        50 + (rand() * 205.0) as u32,
        50 + (rand() * 205.0) as u32,
    );
    s
}

struct Sim {
    settings: Settings,
    /// Curve parameter, advanced every sample.
    t: f64,
    /// Previous traced point, or None right after a reset.
    last: Option<(f64, f64)>,
    /// When paused, time stops advancing (the frame still renders the overlay).
    paused: bool,
    /// Logical (CSS-pixel) canvas size; drawing happens in this space.
    w: f64,
    h: f64,
    /// Device pixel ratio (the backing store is `w*dpr` by `h*dpr`).
    dpr: f64,
    /// View transform: magnification and screen-space pan, applied only when
    /// compositing onto the visible canvas (the trace itself is unscaled).
    zoom: f64,
    pan: (f64, f64),
    /// Left-drag panning state.
    dragging: bool,
    drag_last: (f64, f64),
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    trace_canvas: HtmlCanvasElement,
    trace: CanvasRenderingContext2d,
}

impl Sim {
    fn center(&self) -> (f64, f64) {
        (self.w / 2.0, self.h / 2.0)
    }

    /// Clear the accumulated trace and restart.
    fn reset_trace(&mut self) {
        self.trace
            .set_global_composite_operation("source-over")
            .ok();
        self.trace.clear_rect(0.0, 0.0, self.w, self.h);
        self.last = None;
        self.t = 0.0;
    }

    /// Visible-context transform with dpr only (no zoom) — for the background.
    fn base_transform(&self) {
        let _ = self
            .ctx
            .set_transform(self.dpr, 0.0, 0.0, self.dpr, 0.0, 0.0);
    }

    /// Visible-context transform including zoom + pan about the center.
    fn view_transform(&self) {
        let (cx, cy) = self.center();
        let a = self.dpr * self.zoom;
        let e = self.dpr * (cx * (1.0 - self.zoom) + self.pan.0);
        let f = self.dpr * (cy * (1.0 - self.zoom) + self.pan.1);
        let _ = self.ctx.set_transform(a, 0.0, 0.0, a, e, f);
    }

    /// Zoom about a screen point (CSS px), keeping that world point fixed.
    fn zoom_at(&mut self, mx: f64, my: f64, factor: f64) {
        let (cx, cy) = self.center();
        let z = self.zoom;
        let wx = (mx - (cx * (1.0 - z) + self.pan.0)) / z;
        let wy = (my - (cy * (1.0 - z) + self.pan.1)) / z;
        let z2 = (z * factor).clamp(0.1, 60.0);
        self.pan.0 = mx - z2 * wx - cx * (1.0 - z2);
        self.pan.1 = my - z2 * wy - cy * (1.0 - z2);
        self.zoom = z2;
    }

    /// The pen position and (optional) speed at parameter `t`.
    fn pen(&self, t: f64) -> ((f64, f64), f64) {
        let (cx, cy) = self.center();
        let damp = if self.settings.damping > 0.0 {
            (-self.settings.damping * t).exp()
        } else {
            1.0
        };
        let (mut x, mut y) = (cx, cy);
        let (mut vx, mut vy) = (0.0, 0.0);
        for a in &self.settings.arms {
            let ang = a.w * t + a.phase;
            let rr = a.r * damp;
            x += rr * ang.cos();
            y += rr * ang.sin();
            vx -= rr * a.w * ang.sin();
            vy += rr * a.w * ang.cos();
        }
        ((x, y), vx.hypot(vy))
    }

    /// Advance the simulation and render one frame.
    fn frame(&mut self) {
        let s = self.settings.clone();
        let (cx, cy) = self.center();

        if let Mode::TimesTable = s.mode {
            self.render_times_table(&s);
            return;
        }

        let steps = if self.paused {
            0
        } else {
            s.speed.max(0.0) as u32
        };

        // Fade or accumulate on the trace buffer. Skip while paused so a frozen
        // view doesn't keep fading away.
        self.trace
            .set_global_composite_operation("source-over")
            .ok();
        if s.fade > 0.0 && !self.paused {
            self.trace
                .set_fill_style_str(&format!("rgba(0,0,0,{})", s.fade.clamp(0.0, 1.0)));
            self.trace.fill_rect(0.0, 0.0, self.w, self.h);
        }
        if s.glow {
            self.trace.set_global_composite_operation("lighter").ok();
        }
        self.trace.set_line_width(LINE_WIDTH);

        for _ in 0..steps {
            self.t += s.step;
            // Harmonograph: once the amplitude has decayed away, loop.
            if s.damping > 0.0 && (-s.damping * self.t).exp() < 0.02 {
                self.t = 0.0;
                self.last = None;
                self.trace
                    .set_global_composite_operation("source-over")
                    .ok();
                self.trace.clear_rect(0.0, 0.0, self.w, self.h);
                if s.glow {
                    self.trace.set_global_composite_operation("lighter").ok();
                }
            }
            let ((ox, oy), speed) = self.pen(self.t);
            let (px, py) = if s.antialiasing {
                (ox, oy)
            } else {
                (ox.round(), oy.round())
            };
            if let Some((lx, ly)) = self.last {
                self.trace
                    .set_stroke_style_str(&segment_color(&s, self.t, speed));
                draw_symmetric(&self.trace, s.symmetry, cx, cy, lx, ly, px, py);
            }
            self.last = Some((px, py));
        }
        self.trace
            .set_global_composite_operation("source-over")
            .ok();

        // Composite the trace and draw the live construction on the visible
        // canvas. The background is filled at 1:1; the trace and overlay are
        // drawn under the view transform (zoom + pan).
        self.base_transform();
        let ctx = &self.ctx;
        ctx.set_global_composite_operation("source-over").ok();
        ctx.set_fill_style_str("#000000");
        ctx.fill_rect(0.0, 0.0, self.w, self.h);
        self.view_transform();
        if s.draw {
            if s.glow {
                ctx.set_global_composite_operation("lighter").ok();
            }
            let _ = ctx.draw_image_with_html_canvas_element_and_dw_and_dh(
                &self.trace_canvas,
                0.0,
                0.0,
                self.w,
                self.h,
            );
            ctx.set_global_composite_operation("source-over").ok();
        }

        if s.circles {
            let damp = if s.damping > 0.0 {
                (-s.damping * self.t).exp()
            } else {
                1.0
            };
            let (mut px, mut py) = (cx, cy);
            ctx.set_line_width(1.0);
            for a in &s.arms {
                let rr = a.r * damp;
                ctx.set_stroke_style_str("rgba(120,200,255,0.45)");
                stroke_circle(ctx, px, py, rr);
                let ang = a.w * self.t + a.phase;
                let nx = px + rr * ang.cos();
                let ny = py + rr * ang.sin();
                ctx.set_stroke_style_str("rgba(255,120,200,0.55)");
                draw_line(ctx, px, py, nx, ny);
                px = nx;
                py = ny;
            }
        }

        if s.drawdot
            && let Some((px, py)) = self.last
        {
            ctx.set_fill_style_str("#ffe14d");
            fill_circle(ctx, px, py, DOT_R);
        }
    }

    /// Times-table mode: chords `i -> (k·i mod N)` on a circle, redrawn each
    /// frame straight onto the visible canvas.
    fn render_times_table(&self, s: &Settings) {
        let (cx, cy) = self.center();
        self.base_transform();
        let ctx = &self.ctx;
        ctx.set_global_composite_operation("source-over").ok();
        ctx.set_fill_style_str("#000000");
        ctx.fill_rect(0.0, 0.0, self.w, self.h);
        self.view_transform();
        if s.glow {
            ctx.set_global_composite_operation("lighter").ok();
        }
        ctx.set_line_width(LINE_WIDTH);
        let n = s.tt_points.max(2.0) as u32;
        let r = s.tt_radius;
        let point = |i: u32| {
            let a = TAU * i as f64 / n as f64;
            (cx + r * a.cos(), cy + r * a.sin())
        };
        for i in 0..n {
            let j = ((s.tt_mult * i as f64).rem_euclid(n as f64)) as u32 % n;
            let (x0, y0) = point(i);
            let (x1, y1) = point(j);
            ctx.set_stroke_style_str(&segment_color(s, i as f64 / n as f64 * TAU, 0.0));
            draw_symmetric(ctx, s.symmetry, cx, cy, x0, y0, x1, y1);
        }
        ctx.set_global_composite_operation("source-over").ok();
    }
}

/// Color for one segment given the fill mode.
fn segment_color(s: &Settings, t: f64, speed: f64) -> String {
    match s.fill {
        Fill::Solid => s.color.clone(),
        Fill::HueTime => format!("hsl({:.0}, 95%, 60%)", (t * 40.0).rem_euclid(360.0)),
        Fill::HueSpeed => format!("hsl({:.0}, 95%, 60%)", (speed * 0.4).rem_euclid(360.0)),
        Fill::Random => format!(
            "rgb({},{},{})",
            50 + (rand() * 205.0) as u32,
            50 + (rand() * 205.0) as u32,
            50 + (rand() * 205.0) as u32
        ),
    }
}

/// Draw a segment plus its `sym`-fold rotations about `(cx, cy)`.
#[allow(clippy::too_many_arguments)]
fn draw_symmetric(
    ctx: &CanvasRenderingContext2d,
    sym: u32,
    cx: f64,
    cy: f64,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
) {
    let k = sym.max(1);
    for i in 0..k {
        let a = TAU * i as f64 / k as f64;
        let (sn, cs) = (a.sin(), a.cos());
        let rot = |x: f64, y: f64| {
            let (dx, dy) = (x - cx, y - cy);
            (cx + dx * cs - dy * sn, cy + dx * sn + dy * cs)
        };
        let (ax, ay) = rot(x1, y1);
        let (bx, by) = rot(x2, y2);
        ctx.begin_path();
        ctx.move_to(ax, ay);
        ctx.line_to(bx, by);
        ctx.stroke();
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

fn context_2d(canvas: &HtmlCanvasElement) -> CanvasRenderingContext2d {
    canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into::<CanvasRenderingContext2d>()
        .unwrap()
}

/// Trigger a PNG download of the current canvas.
fn save_png(canvas: &HtmlCanvasElement) {
    let Ok(url) = canvas.to_data_url_with_type("image/png") else {
        return;
    };
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(a) = document
        .create_element("a")
        .ok()
        .and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok())
    else {
        return;
    };
    a.set_href(&url);
    a.set_download("cardioid.png");
    a.click();
}

type AnimationClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// requestAnimationFrame loop; stops once the canvas leaves the document.
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
    // Seed from the query captured in `main()` before the router ran.
    let mut settings = use_signal(|| Settings::from_query(&captured_query()));
    let state: Signal<Option<Rc<RefCell<Sim>>>> = use_signal(|| None);
    let mut paused = use_signal(|| false);
    // The options panel covers most of a phone screen, so it starts hidden on
    // narrow viewports and can be toggled from a floating button.
    let mut panel_open = use_signal(|| {
        web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|v| v.as_f64())
            .is_none_or(|w| w >= 640.0)
    });

    // Push setting changes into the running sim, clearing the trace when the
    // drawn figure (not just a cosmetic property) changed.
    use_effect(move || {
        let s = settings.read().clone();
        if let Some(sim) = state.read().clone() {
            let mut sim = sim.borrow_mut();
            let changed = sim.settings.figure() != s.figure();
            sim.settings = s;
            if changed {
                sim.reset_trace();
            }
        }
    });

    // Mirror settings into the URL (replaceState: no remount, canvas stays live).
    use_effect(move || {
        let query = settings.read().to_query();
        if let Some(history) = web_sys::window().and_then(|w| w.history().ok()) {
            let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&format!("?{query}")));
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
                    paused: false,
                    w: css_w,
                    h: css_h,
                    dpr,
                    zoom: 1.0,
                    pan: (0.0, 0.0),
                    dragging: false,
                    drag_last: (0.0, 0.0),
                    canvas,
                    ctx,
                    trace_canvas,
                    trace,
                }));
                state.set(Some(Rc::clone(&sim)));

                // Mouse-wheel zoom, centered on the cursor. A non-passive
                // listener so the graph zooms instead of the browser page.
                {
                    let sim_wheel = Rc::clone(&sim);
                    let canvas = sim.borrow().canvas.clone();
                    let on_wheel = Closure::<dyn FnMut(web_sys::WheelEvent)>::new(
                        move |evt: web_sys::WheelEvent| {
                            evt.prevent_default();
                            let factor = (-evt.delta_y() * 0.0015).exp();
                            let rect = sim_wheel.borrow().canvas.get_bounding_client_rect();
                            let mx = evt.client_x() as f64 - rect.left();
                            let my = evt.client_y() as f64 - rect.top();
                            sim_wheel.borrow_mut().zoom_at(mx, my, factor);
                        },
                    );
                    let opts = web_sys::AddEventListenerOptions::new();
                    opts.set_passive(false);
                    let _ = canvas.add_event_listener_with_callback_and_add_event_listener_options(
                        "wheel",
                        on_wheel.as_ref().unchecked_ref(),
                        &opts,
                    );
                    on_wheel.forget();
                }

                start_animation_loop(sim);
            });
        }
    };

    let s = settings.read();
    let (play_label, play_style) = if *paused.read() {
        ("▶ Play", "background:#2f7d32;border-color:#3faf43")
    } else {
        ("⏸ Pause", "background:#8a5a1a;border-color:#c98a2a")
    };
    let is_tt = matches!(s.mode, Mode::TimesTable);
    let panel_display = if panel_open() { "block" } else { "none" };

    rsx! {
        div {
            style: "position:relative;width:100vw;height:100vh;overflow:hidden;background:#000;\
                    font-family:'Space Grotesk',system-ui,sans-serif;color:#eee;",
            canvas {
                id: "cardioid-canvas",
                style: "position:absolute;inset:0;width:100%;height:100%;display:block;\
                        cursor:grab;touch-action:none;",
                onmounted: onmounted,
                // Left-drag to pan the view.
                onpointerdown: move |e| {
                    let c = e.data().client_coordinates();
                    if let Some(sim) = state.read().clone() {
                        let mut sim = sim.borrow_mut();
                        sim.dragging = true;
                        sim.drag_last = (c.x, c.y);
                    }
                },
                onpointermove: move |e| {
                    let c = e.data().client_coordinates();
                    if let Some(sim) = state.read().clone() {
                        let mut sim = sim.borrow_mut();
                        if sim.dragging {
                            sim.pan.0 += c.x - sim.drag_last.0;
                            sim.pan.1 += c.y - sim.drag_last.1;
                            sim.drag_last = (c.x, c.y);
                        }
                    }
                },
                onpointerup: move |_| {
                    if let Some(sim) = state.read().clone() {
                        sim.borrow_mut().dragging = false;
                    }
                },
                onpointerleave: move |_| {
                    if let Some(sim) = state.read().clone() {
                        sim.borrow_mut().dragging = false;
                    }
                },
                // Double-click resets the view to 1:1.
                ondoubleclick: move |_| {
                    if let Some(sim) = state.read().clone() {
                        let mut sim = sim.borrow_mut();
                        sim.zoom = 1.0;
                        sim.pan = (0.0, 0.0);
                    }
                },
            }

            if !panel_open() {
                button {
                    id: "cardioid-panel-toggle",
                    style: "position:absolute;top:16px;left:16px;padding:8px 14px;border:1px solid #555;\
                            border-radius:10px;background:rgba(20,20,20,.85);backdrop-filter:blur(4px);\
                            box-shadow:0 4px 24px rgba(0,0,0,.5);color:#eee;cursor:pointer;\
                            font:inherit;font-size:14px;",
                    onclick: move |_| panel_open.set(true),
                    "⚙ Options"
                }
            }

            div {
                id: "cardioid-panel",
                // Hidden via display:none (not unmounted) so slider positions and
                // open <details> sections survive a hide/show round-trip.
                style: "position:absolute;top:16px;left:16px;max-height:calc(100vh - 32px);\
                        overflow:auto;padding:14px 16px;border-radius:10px;\
                        background:rgba(20,20,20,.85);backdrop-filter:blur(4px);\
                        box-shadow:0 4px 24px rgba(0,0,0,.5);user-select:none;\
                        width:min(300px,calc(100vw - 32px));\
                        display:{panel_display};",

                div {
                    style: "display:flex;align-items:center;justify-content:space-between;margin-bottom:2px;",
                    div {
                        style: "font-weight:700;font-size:18px;color:#ff4d8d;",
                        "Cardioid"
                    }
                    button {
                        style: "padding:2px 9px;border:1px solid #555;border-radius:6px;background:#333;\
                                color:#bbb;cursor:pointer;font:inherit;font-size:13px;",
                        onclick: move |_| panel_open.set(false),
                        "✕ Hide"
                    }
                }
                div {
                    style: "font-size:11px;color:#777;margin-bottom:10px;",
                    "scroll = zoom · drag = pan · dbl-click = reset view"
                }

                button {
                    style: "width:100%;margin-bottom:12px;padding:7px;border:1px solid;\
                            border-radius:6px;color:#fff;cursor:pointer;font:inherit;\
                            font-size:14px;font-weight:600;{play_style}",
                    onclick: move |_| {
                        let now = !paused();
                        paused.set(now);
                        if let Some(sim) = state.read().clone() {
                            sim.borrow_mut().paused = now;
                        }
                    },
                    "{play_label}"
                }

                // ── Basic: the arms and the trace color ──────────────────────
                if !is_tt {
                    for (i , arm) in s.arms.iter().enumerate() {
                        Slider {
                            label: format!("Arm {} radius", i + 1),
                            min: "1", max: "300", step: "1", value: arm.r, decimals: 0,
                            oninput: move |v| settings.write().arms[i].r = v,
                        }
                        Slider {
                            label: format!("Arm {} speed", i + 1),
                            min: "-200", max: "200", step: "0.1", value: arm.w, decimals: 1,
                            oninput: move |v| settings.write().arms[i].w = v,
                        }
                    }
                    div {
                        style: "display:flex;gap:8px;margin:6px 0 10px;",
                        button {
                            style: "flex:1;padding:5px;border:1px solid #555;border-radius:6px;\
                                    background:#333;color:#eee;cursor:pointer;font:inherit;font-size:13px;",
                            onclick: move |_| {
                                let mut w = settings.write();
                                if w.arms.len() < MAX_ARMS {
                                    let a = w.arms.last().cloned().unwrap_or(Arm { r: 80.0, w: 3.0, phase: 0.0 });
                                    w.arms.push(a);
                                }
                            },
                            "+ arm"
                        }
                        button {
                            style: "flex:1;padding:5px;border:1px solid #555;border-radius:6px;\
                                    background:#333;color:#eee;cursor:pointer;font:inherit;font-size:13px;",
                            onclick: move |_| {
                                let mut w = settings.write();
                                if w.arms.len() > 1 {
                                    w.arms.pop();
                                }
                            },
                            "– arm"
                        }
                    }
                }

                div {
                    style: "display:flex;align-items:center;justify-content:space-between;gap:8px;margin:0 0 4px;font-size:12px;color:#bbb;",
                    span { "Trace color" }
                    input {
                        r#type: "color", value: "{s.color}",
                        style: "width:46px;height:26px;border:none;background:none;cursor:pointer;padding:0;",
                        oninput: move |e| settings.write().color = e.value(),
                    }
                }

                // ── Advanced sections (collapsed by default) ─────────────────
                Details { summary: "Phase & sampling",
                    if !is_tt {
                        for (i , arm) in s.arms.iter().enumerate() {
                            Slider {
                                label: format!("Arm {} phase", i + 1),
                                min: "0", max: "6.283", step: "0.01", value: arm.phase, decimals: 2,
                                oninput: move |v| settings.write().arms[i].phase = v,
                            }
                        }
                    }
                    Slider { label: "Sample step (Δt)", min: "0.001", max: "1.6", step: "0.001",
                        value: s.step, decimals: 3, oninput: move |v| settings.write().step = v }
                    Slider { label: "Draw speed (samples/frame)", min: "1", max: "1200", step: "1",
                        value: s.speed, decimals: 0, oninput: move |v| settings.write().speed = v }
                }

                Details { summary: "Look",
                    div {
                        style: "font-size:12px;color:#bbb;margin:4px 0 2px;",
                        "Color mode"
                    }
                    select {
                        style: "width:100%;margin-bottom:8px;padding:4px;background:#222;color:#eee;border:1px solid #555;border-radius:6px;font:inherit;font-size:13px;",
                        onchange: move |e| {
                            if let Some(f) = Fill::from_tag(&e.value()) {
                                settings.write().fill = f;
                            }
                        },
                        option { value: "solid", selected: matches!(s.fill, Fill::Solid), "Solid" }
                        option { value: "huet", selected: matches!(s.fill, Fill::HueTime), "Hue by time" }
                        option { value: "hues", selected: matches!(s.fill, Fill::HueSpeed), "Hue by speed" }
                        option { value: "rand", selected: matches!(s.fill, Fill::Random), "Random" }
                    }
                    Slider { label: "Trail fade (0 = keep all)", min: "0", max: "0.3", step: "0.005",
                        value: s.fade, decimals: 3, oninput: move |v| settings.write().fade = v }
                    div {
                        style: "display:flex;flex-wrap:wrap;gap:6px;margin-top:8px;",
                        Toggle { label: "Glow", on: s.glow, onclick: move |_| { let v = settings.read().glow; settings.write().glow = !v; } }
                        Toggle { label: "Draw", on: s.draw, onclick: move |_| { let v = settings.read().draw; settings.write().draw = !v; } }
                        Toggle { label: "Dot", on: s.drawdot, onclick: move |_| { let v = settings.read().drawdot; settings.write().drawdot = !v; } }
                        Toggle { label: "Circles", on: s.circles, onclick: move |_| { let v = settings.read().circles; settings.write().circles = !v; } }
                        Toggle { label: "Antialias", on: s.antialiasing, onclick: move |_| { let v = settings.read().antialiasing; settings.write().antialiasing = !v; } }
                    }
                }

                Details { summary: "Symmetry & damping",
                    Slider { label: "Symmetry (k-fold)", min: "1", max: "16", step: "1",
                        value: s.symmetry as f64, decimals: 0, oninput: move |v| settings.write().symmetry = (v as u32).max(1) }
                    Slider { label: "Damping (harmonograph)", min: "0", max: "0.2", step: "0.002",
                        value: s.damping, decimals: 3, oninput: move |v| settings.write().damping = v }
                }

                Details { summary: "Mode",
                    div {
                        style: "display:flex;gap:8px;margin:4px 0 8px;",
                        Toggle { label: "Epicycle", on: !is_tt, onclick: move |_| settings.write().mode = Mode::Epicycle }
                        Toggle { label: "Times-table", on: is_tt, onclick: move |_| settings.write().mode = Mode::TimesTable }
                    }
                    if is_tt {
                        Slider { label: "Points (N)", min: "3", max: "500", step: "1",
                            value: s.tt_points, decimals: 0, oninput: move |v| settings.write().tt_points = v }
                        Slider { label: "Multiplier (k)", min: "2", max: "100", step: "0.05",
                            value: s.tt_mult, decimals: 2, oninput: move |v| settings.write().tt_mult = v }
                        Slider { label: "Radius", min: "50", max: "400", step: "1",
                            value: s.tt_radius, decimals: 0, oninput: move |v| settings.write().tt_radius = v }
                    }
                }

                Details { summary: "Presets & tools",
                    div {
                        style: "display:flex;flex-wrap:wrap;gap:6px;margin:4px 0 8px;",
                        for (name , preset) in presets() {
                            button {
                                style: "flex:0 0 auto;padding:5px 9px;border:1px solid #555;border-radius:6px;\
                                        background:#26304a;color:#cfe0ff;cursor:pointer;font:inherit;font-size:12px;",
                                onclick: move |_| *settings.write() = preset.clone(),
                                "{name}"
                            }
                        }
                    }
                    div {
                        style: "display:flex;gap:8px;",
                        button {
                            style: "flex:1;padding:6px;border:1px solid #7a4bbf;border-radius:6px;\
                                    background:#3a2a55;color:#e0c8ff;cursor:pointer;font:inherit;font-size:13px;",
                            onclick: move |_| *settings.write() = randomized(),
                            "🎲 Randomize"
                        }
                        button {
                            style: "flex:1;padding:6px;border:1px solid #555;border-radius:6px;\
                                    background:#333;color:#eee;cursor:pointer;font:inherit;font-size:13px;",
                            onclick: move |_| {
                                if let Some(sim) = state.read().clone() {
                                    save_png(&sim.borrow().canvas);
                                }
                            },
                            "💾 PNG"
                        }
                    }
                }

                // ── Transport: clear / reset ─────────────────────────────────
                div {
                    style: "display:flex;gap:8px;margin-top:12px;",
                    button {
                        style: "flex:1;padding:6px;border:1px solid #555;border-radius:6px;\
                                background:#3a2030;color:#ff9dc0;cursor:pointer;font:inherit;font-size:13px;",
                        onclick: move |_| {
                            if let Some(sim) = state.read().clone() {
                                sim.borrow_mut().reset_trace();
                            }
                        },
                        "Clear trace"
                    }
                    button {
                        style: "flex:1;padding:6px;border:1px solid #555;border-radius:6px;\
                                background:#203a30;color:#9dffc0;cursor:pointer;font:inherit;font-size:13px;",
                        onclick: move |_| *settings.write() = Settings::default(),
                        "Reset"
                    }
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
