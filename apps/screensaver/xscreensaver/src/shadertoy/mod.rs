//! The Shadertoy savers: upstream's `hacks/glx/xshadertoy.c`.
//!
//! ```text
//! xshadertoy, Copyright © 2025-2026 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Run arbitrary GLSL programs that use the shadertoy.com API.
//! ```
//!
//! Thirty savers that are all the same program. Upstream builds each one as a
//! shell script that runs `xshadertoy` with a `.glsl` file on stdin; here each
//! is a [`ShadertoyDef`] naming its sources, and this module is the part they
//! share.
//!
//! Nothing here draws. A Shadertoy program is a fragment shader run over every
//! pixel, so unlike the 2D tier there is no software path: the host owns a
//! WebGL2 context and this side works out, for each frame, which program to
//! run and what the uniforms should be. That keeps the interesting half
//! testable without a browser, which is the same bargain [`crate::runtime`]
//! makes for the 2D savers.
//!
//! ## What a Shadertoy program is
//!
//! A function `mainImage (out vec4 fragColor, in vec2 fragCoord)`, called once
//! per pixel, with a fixed set of uniforms in scope. Up to five of them run in
//! a chain per frame: `BufferA` to `BufferD` render into textures and `Image`
//! renders the picture, with each pass able to read any pass's texture through
//! `iChannel0` to `iChannel3`. A sixth source, `Common`, is not a pass; it is
//! text prepended to all of them.
//!
//! Of the collection, four programs read a channel and one has more than one
//! pass, so the chain is mostly a formality. It is here because it is what
//! makes the two that need it work.
//!
//! ## Divergences
//!
//! Upstream binds a pass's own output texture to its own `iChannel0` while
//! drawing into it. That is a feedback loop, undefined in OpenGL, and what it
//! does in practice is read the previous frame, which is what the programs
//! that do it want. WebGL2 does not leave it undefined: it refuses the draw
//! outright and nothing appears. So each pass here has two textures and they
//! swap every frame, which is the behaviour the undefined version has and is
//! also what shadertoy.com itself does.
//!
//! `iDate` gets a time of day but a zero date: the host tells a saver what time
//! it is, not what day it is, and no program in the collection reads it.
//! `iChannelResolution`, `iChannelTime` and `iSampleRate` are declared and
//! never set, exactly as upstream leaves them. Keyboard input is not supported
//! upstream either, and for the same reason: it arrives as a texture with one
//! bit per key.

use crate::runtime::{Opt, Resources, SaverDef, StartArgs, XEvent, random};

pub mod alienbeacon;
pub mod batteredplanet;
pub mod bestill;
pub mod bubblecolors;
pub mod darktransit;
pub mod downfall;
pub mod driftclouds;
pub mod elementalring;
pub mod fluxcore;
pub mod gimbalharmonics;
pub mod goldenapollian;
pub mod hexplasma;
pub mod logarithmiccircles;
pub mod neongravity;
pub mod neontriangulator;
pub mod noxfire;
pub mod prococean;
pub mod protophore;
pub mod rigrekt;
pub mod selfreflect;
pub mod skyline;
pub mod stardome;
pub mod starnest;
pub mod stripeytorus;
pub mod synthwavecity;
pub mod topologica;
pub mod trainmandala;
pub mod trizm;
pub mod truchetzoom;
pub mod universeball;

/// `BufferA`, `BufferB`, `BufferC`, `BufferD`, `Image`. Upstream's `BUFFERS` is
/// six and counts `Common`, which is not a pass.
pub const MAX_PASSES: usize = 5;

/// How many textures a program can read. The uniforms are named `iChannel0`
/// through `iChannel3`, so this is not a policy, it is the API.
pub const MAX_CHANNELS: usize = 4;

/// One whole program: the passes, and the source shared between them.
pub struct Variant {
    /// Prepended to every pass. Empty when the program has no `Common`.
    pub common: &'static str,
    /// `BufferA` first and `Image` last, however many there are.
    pub passes: &'static [&'static str],
}

/// A saver in this tier: its identity and knobs, plus the programs themselves.
///
/// The [`SaverDef`] is the same one the 2D savers use, so the options panel and
/// the credits work the same way for both tiers.
pub struct ShadertoyDef {
    pub def: SaverDef,
    /// Alternative programs, stepped through every `duration` seconds. Only
    /// `bestill` has more than one.
    pub variants: &'static [Variant],
}

/// Every Shadertoy saver.
///
/// Deliberately absent on wasm, for the reason [`crate::all`] is: naming every
/// saver's data in one table would pull all of it into the main module instead
/// of into the chunks.
#[cfg(not(target_arch = "wasm32"))]
pub static ALL: &[&ShadertoyDef] = &[
    &alienbeacon::DEF,
    &batteredplanet::DEF,
    &bestill::DEF,
    &bubblecolors::DEF,
    &darktransit::DEF,
    &downfall::DEF,
    &driftclouds::DEF,
    &elementalring::DEF,
    &fluxcore::DEF,
    &gimbalharmonics::DEF,
    &goldenapollian::DEF,
    &hexplasma::DEF,
    &logarithmiccircles::DEF,
    &neongravity::DEF,
    &neontriangulator::DEF,
    &noxfire::DEF,
    &prococean::DEF,
    &protophore::DEF,
    &rigrekt::DEF,
    &selfreflect::DEF,
    &skyline::DEF,
    &stardome::DEF,
    &starnest::DEF,
    &stripeytorus::DEF,
    &synthwavecity::DEF,
    &topologica::DEF,
    &trainmandala::DEF,
    &trizm::DEF,
    &truchetzoom::DEF,
    &universeball::DEF,
];

/// `xshadertoy`'s own defaults, which every saver in the tier inherits.
pub const DEFAULTS: &[&str] = &[
    "*delay:		20000",
    "*showFPS:		False",
    "*speed:		1.0",
    "*scale:		1.0",
    "*duration:		120",
];

/// The knobs every one of them shows, from the XML. `duration` is only in the
/// XML of the saver that has variants, so it is added there.
pub const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 10.0, 0.01, 2, "1.0"),
    Opt::slider("scale", "Resolution", 0.1, 1.0, 0.05, 2, "1.0"),
];

/// `#version`. WebGL2 is GLSL ES 3.00, which is also what shadertoy.com
/// compiles against, so the compatibility half of the preamble below never
/// applies. It is kept because it is upstream's.
const VERSION: &str = "#version 300 es\n";

/// Upstream's `vertex_shader`: two triangles over the whole viewport, which is
/// the only geometry any of this has.
const VERTEX_SHADER: &str = include_str!("vertex.glsl");

/// Upstream's `fragment_shader_head`: the Shadertoy API's uniforms, and a
/// compatibility layer for GLSL 1.20 that GLSL ES 3.00 skips.
const FRAGMENT_HEAD: &str = include_str!("head.glsl");

/// Upstream's `fragment_shader_tail`: the `main` that calls `mainImage`.
const FRAGMENT_TAIL: &str = include_str!("tail.glsl");

/// The vertex shader every pass uses.
pub fn vertex_source() -> String {
    format!("{VERSION}{VERTEX_SHADER}")
}

/// Assemble one pass: the version, the API preamble, the program's `Common`
/// source, and the program itself, wrapped in a `main`.
///
/// The `#line 0` before the body is upstream's, and it is what makes a
/// compiler error point at a line of the `.glsl` file rather than at a line of
/// the preamble.
pub fn fragment_source(variant: &Variant, pass: usize) -> String {
    let body = variant.passes.get(pass).copied().unwrap_or("");
    format!(
        "{VERSION}{FRAGMENT_HEAD}{}\n#line 0\n{body}{FRAGMENT_TAIL}",
        variant.common
    )
}

/// What the host needs to draw one frame.
pub struct Frame {
    /// Which variant to run. The host recompiles when this changes.
    pub variant: usize,
    /// `iResolution`: the size the passes render at, which is the window scaled
    /// by the resolution knob.
    pub resolution: [f32; 3],
    /// `iTime`, seconds since this variant started, warped by the speed knob.
    pub time: f32,
    /// `iTimeDelta`, since the previous frame.
    pub time_delta: f32,
    /// `iFrameRate`, averaged over the run rather than measured.
    pub frame_rate: f32,
    /// `iFrame`.
    pub frame: i32,
    /// `iMouse`. See [`Shadertoy::mouse`].
    pub mouse: [f32; 4],
    /// `iDate`: year, month, day, seconds since midnight. The first three are
    /// zero here; the host knows the time but not the date.
    pub date: [f32; 4],
}

/// A running Shadertoy saver: which program, how far through it, and where the
/// pointer has been.
pub struct Shadertoy {
    def: &'static ShadertoyDef,
    /// Microseconds the saver would like between frames.
    delay: u32,
    speed: f64,
    /// Fraction of the window the passes render at. Upstream scales down only.
    scale: f32,
    duration: f64,
    width: i32,
    height: i32,

    variant: usize,
    start_time: f64,
    last_time: f64,
    next_due: f64,
    started: bool,
    total_frames: u32,
    /// Set when the variant changed, cleared once the host has recompiled.
    reload: bool,

    /// Where the pointer is, in window pixels with the origin at the bottom.
    pointer: (i32, i32),
    mouse_clicked: (i32, i32),
    mouse_dragged: (i32, i32),
    button_down: bool,
    was_button_down: bool,

    wall_clock: f64,
}

impl Shadertoy {
    /// Start a saver. Called by the saver itself, from inside its own module,
    /// for the code-splitting reason [`crate::runtime::Runner::start`] explains.
    pub fn start(def: &'static ShadertoyDef, args: StartArgs) -> Self {
        crate::runtime::ya_rand_init(args.seed);
        let res = Resources::new(def.def.defaults, def.def.opts, &args.query);
        let scale = res.float("scale") as f32;
        Self {
            def,
            delay: res.int("delay").max(0) as u32,
            speed: res.float("speed"),
            /* Scale down only. */
            scale: if scale > 1.0 || scale <= 0.0 {
                1.0
            } else {
                scale
            },
            duration: res.float("duration").max(1.0),
            width: args.width.max(1),
            height: args.height.max(1),
            variant: (random() as usize) % def.variants.len().max(1),
            start_time: 0.0,
            last_time: 0.0,
            next_due: 0.0,
            started: false,
            total_frames: 0,
            reload: true,
            pointer: (0, 0),
            mouse_clicked: (0, 0),
            mouse_dragged: (0, 0),
            button_down: false,
            was_button_down: false,
            wall_clock: args.wall_clock,
        }
    }

    pub fn def(&self) -> &'static SaverDef {
        &self.def.def
    }

    pub fn variants(&self) -> &'static [Variant] {
        self.def.variants
    }

    /// Which variant is running. The host compiles this one.
    pub fn variant(&self) -> usize {
        self.variant
    }

    /// The size the passes render at, which is what the textures have to be.
    pub fn buffer_size(&self) -> (i32, i32) {
        (
            ((self.width as f32 * self.scale) as i32).max(1),
            ((self.height as f32 * self.scale) as i32).max(1),
        )
    }

    /// True until the host has compiled the current variant.
    pub fn take_reload(&mut self) -> bool {
        std::mem::take(&mut self.reload)
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }

    /// `NAME_handle_event`. Shadertoy's only input is the pointer.
    pub fn event(&mut self, event: &XEvent) -> bool {
        match event {
            XEvent::ButtonPress { x, y, .. } => {
                self.pointer = self.flip(*x, *y);
                self.button_down = true;
                true
            }
            XEvent::ButtonRelease { x, y, .. } => {
                self.pointer = self.flip(*x, *y);
                self.button_down = false;
                true
            }
            XEvent::MotionNotify { x, y } => {
                self.pointer = self.flip(*x, *y);
                true
            }
            _ => false,
        }
    }

    /// A window position with the origin at the top left, as X and the browser
    /// both give it, to one with the origin at the bottom left, as GLSL wants
    /// it. Zero means "no button", so neither coordinate is allowed to be zero.
    fn flip(&self, x: i32, y: i32) -> (i32, i32) {
        (
            x.clamp(1, (self.width - 1).max(1)),
            (self.height - 1 - y).clamp(1, (self.height - 1).max(1)),
        )
    }

    /// `iMouse`, which is a history rather than a position.
    ///
    /// `xy` is where the pointer is; `zw` is where the drag started, and goes
    /// negative when the button comes up. A program can therefore tell a click
    /// from a drag from a release from having never been touched at all, which
    /// is why the sign flipping looks like an accident and is not.
    fn mouse(&mut self) -> [f32; 4] {
        let (x, y) = self.pointer;
        let (mx, my, mz, mw) = if self.button_down && !self.was_button_down {
            /* Drag beginning */
            self.mouse_clicked = (x, y);
            (x, y, x, y)
        } else if self.button_down {
            /* Drag continuing */
            self.mouse_dragged = (x, y);
            (x, y, self.mouse_clicked.0, -self.mouse_clicked.1)
        } else if self.was_button_down {
            /* Drag released */
            self.mouse_dragged = (x, y);
            (x, y, -self.mouse_clicked.0, -self.mouse_clicked.1)
        } else {
            /* Not dragging */
            (
                self.mouse_dragged.0,
                self.mouse_dragged.1,
                -self.mouse_clicked.0,
                -self.mouse_clicked.1,
            )
        };
        self.was_button_down = self.button_down;
        [
            mx as f32 * self.scale,
            my as f32 * self.scale,
            mz as f32 * self.scale,
            mw as f32 * self.scale,
        ]
    }

    /// Advance to wall-clock `now` and say what to draw, or `None` if the
    /// saver's requested delay has not elapsed yet.
    pub fn tick(&mut self, now: f64) -> Option<Frame> {
        if !self.started {
            self.started = true;
            self.start_time = now;
            self.last_time = now;
            self.next_due = now;
        }
        if now < self.next_due {
            return None;
        }
        self.next_due = now + f64::from(self.delay) / 1_000_000.0;

        if self.def.variants.len() > 1 && now > self.start_time + self.duration {
            self.variant = (self.variant + 1) % self.def.variants.len();
            self.reload = true;
            self.total_frames = 0;
            self.start_time = now;
            self.last_time = now;
        }

        /* Time warp */
        let warped = self.start_time + (now - self.start_time) * self.speed;
        let elapsed = warped - self.start_time;
        let mouse = self.mouse();
        let (bw, bh) = self.buffer_size();

        let frame = Frame {
            variant: self.variant,
            resolution: [bw as f32, bh as f32, 1.0],
            time: elapsed as f32,
            time_delta: (warped - self.last_time) as f32,
            // Upstream divides by the elapsed time, which is zero on the first
            // frame. Nothing reads it, but a NaN uniform is a poor thing to
            // hand a driver.
            frame_rate: if elapsed > 0.0 {
                (f64::from(self.total_frames) / elapsed) as f32
            } else {
                0.0
            },
            frame: self.total_frames as i32,
            mouse,
            date: [0.0, 0.0, 0.0, (self.wall_clock + elapsed) as f32],
        };

        self.last_time = warped;
        self.total_frames += 1;
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The version line has to come first or nothing compiles, and the body has
    /// to come after the uniforms it uses.
    #[test]
    fn a_pass_is_assembled_in_order() {
        let v = Variant {
            common: "// COMMON\n",
            passes: &["// BODY\n"],
        };
        let src = fragment_source(&v, 0);
        assert!(src.starts_with("#version 300 es\n"), "version line first");
        let uniforms = src.find("uniform vec3  iResolution");
        let common = src.find("// COMMON");
        let body = src.find("// BODY");
        let main = src.find("void main");
        assert!(uniforms < common && common < body && body < main, "{src}");
    }

    /// Every saver has to name a program with an entry point in it, or it is a
    /// black screen with no error.
    #[test]
    fn every_saver_has_a_program() {
        for st in ALL {
            let def = &st.def;
            assert!(!st.variants.is_empty(), "{} has no variants", def.slug);
            for (i, v) in st.variants.iter().enumerate() {
                assert!(
                    !v.passes.is_empty() && v.passes.len() <= MAX_PASSES,
                    "{} variant {i} has {} passes",
                    def.slug,
                    v.passes.len()
                );
                for (j, p) in v.passes.iter().enumerate() {
                    assert!(
                        p.contains("mainImage"),
                        "{} variant {i} pass {j} has no mainImage",
                        def.slug
                    );
                }
            }
        }
    }

    /// A shader with its own `#version` line would end up with two, and the
    /// second is an error.
    #[test]
    fn no_program_declares_its_own_version() {
        for st in ALL {
            for v in st.variants {
                for p in v.passes.iter().chain(std::iter::once(&v.common)) {
                    assert!(
                        !p.contains("#version"),
                        "{} declares a version",
                        st.def.slug
                    );
                }
            }
        }
    }

    /// The slug is the URL, so it has to match the module it came from, and the
    /// panel needs the rest.
    #[test]
    fn every_saver_is_described() {
        for st in ALL {
            let d = &st.def;
            assert!(!d.slug.is_empty() && !d.label.is_empty());
            assert!(!d.about.blurb.is_empty(), "{} has no blurb", d.slug);
            assert!(!d.about.author.is_empty(), "{} has no author", d.slug);
            assert!(
                d.about.video.is_some_and(|v| v.starts_with("https://")),
                "{} has no video",
                d.slug
            );
        }
    }

    #[test]
    fn slugs_are_unique() {
        let mut seen: Vec<&str> = ALL.iter().map(|s| s.def.slug).collect();
        seen.sort_unstable();
        let n = seen.len();
        seen.dedup();
        assert_eq!(n, seen.len(), "duplicate slug");
    }

    /// The delay knob is a frame rate: a saver asked to run slowly must skip
    /// the frames in between rather than draw them all at once.
    #[test]
    fn the_delay_paces_the_frames() {
        let mut st = starnest::start(StartArgs::new(640, 480, "delay=100000", 20260811));
        assert!(st.tick(0.0).is_some(), "the first frame is due immediately");
        assert!(st.tick(0.05).is_none(), "not due for another 50ms");
        assert!(st.tick(0.10).is_some());
    }

    /// Time is multiplied by the speed knob, not added to.
    #[test]
    fn speed_warps_the_clock() {
        let mut fast = starnest::start(StartArgs::new(640, 480, "speed=2.0&delay=0", 20260811));
        fast.tick(0.0);
        let f = fast.tick(1.0).expect("due");
        assert!((f.time - 2.0).abs() < 1e-5, "{}", f.time);
    }

    /// The resolution knob shrinks what the passes render at, and upstream
    /// clamps it to a reduction.
    #[test]
    fn the_resolution_knob_only_scales_down() {
        let half = starnest::start(StartArgs::new(640, 480, "scale=0.5", 20260811));
        assert_eq!(half.buffer_size(), (320, 240));
        let over = starnest::start(StartArgs::new(640, 480, "scale=4.0", 20260811));
        assert_eq!(over.buffer_size(), (640, 480));
    }

    /// The four values of `iMouse` say what stage of a drag we are in, and a
    /// program that has never been touched has to be able to tell.
    #[test]
    fn the_mouse_records_the_whole_drag() {
        let mut st = starnest::start(StartArgs::new(640, 480, "delay=0", 20260811));
        let idle = st.tick(0.0).expect("due");
        assert_eq!(idle.mouse, [0.0, 0.0, 0.0, 0.0], "untouched");

        st.event(&XEvent::ButtonPress {
            x: 100,
            y: 380,
            button: 1,
        });
        let down = st.tick(0.1).expect("due");
        // y is flipped: 480 - 1 - 380.
        assert_eq!(down.mouse, [100.0, 99.0, 100.0, 99.0], "drag beginning");

        st.event(&XEvent::MotionNotify { x: 200, y: 280 });
        let drag = st.tick(0.2).expect("due");
        assert_eq!(drag.mouse, [200.0, 199.0, 100.0, -99.0], "dragging");

        st.event(&XEvent::ButtonRelease {
            x: 200,
            y: 280,
            button: 1,
        });
        let up = st.tick(0.3).expect("due");
        assert_eq!(up.mouse, [200.0, 199.0, -100.0, -99.0], "released");

        let after = st.tick(0.4).expect("due");
        assert_eq!(after.mouse, [200.0, 199.0, -100.0, -99.0], "let go");
    }

    /// The one saver with variants steps to the next one and starts its clock
    /// again, which is what tells the host to recompile.
    #[test]
    fn variants_change_over_and_reset_the_clock() {
        let mut st = bestill::start(StartArgs::new(640, 480, "duration=10&delay=0", 20260811));
        let first = st.tick(0.0).expect("due").variant;
        assert!(st.take_reload(), "the first frame needs compiling");
        assert_eq!(st.tick(5.0).expect("due").variant, first, "not yet");
        assert!(!st.take_reload());

        let f = st.tick(11.0).expect("due");
        assert_ne!(f.variant, first, "changed over");
        assert!(st.take_reload(), "the host has to recompile");
        assert!(f.time < 1.0, "the clock starts again: {}", f.time);
    }

    /// A saver with one variant never changes over, however long it runs.
    #[test]
    fn one_variant_stays_put() {
        let mut st = starnest::start(StartArgs::new(640, 480, "duration=1&delay=0", 20260811));
        st.tick(0.0);
        assert!(st.take_reload());
        for i in 1..10 {
            st.tick(f64::from(i) * 10.0);
        }
        assert!(!st.take_reload(), "nothing to recompile");
    }
}
