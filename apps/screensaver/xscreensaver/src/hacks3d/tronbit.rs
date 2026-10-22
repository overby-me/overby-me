//! Port of `hacks/glx/tronbit.c`.
//!
//! ```text
//! tronbit, Copyright © 2011-2025 Jamie Zawinski <jwz@jwz.org>
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
//! The character "Bit" from the film "Tron": a polyhedron that answers yes or
//! no and otherwise idles.
//!
//! Four shapes, none of them computed. They are polyhedra someone modelled and
//! upstream ships as vertex tables: a tetrahedron for yes, the second
//! stellation of an icosahedron for no, and for idling a small triambic
//! icosahedron and the compound of an icosahedron and a dodecahedron. Each is
//! turned into place by a fixed rotation that nobody derived, they were nudged
//! until the four lined up.
//!
//! Nothing ever cuts from one shape to the next. Every frame draws *two* of
//! them, the one it is leaving and the one it is arriving at, scaled by
//! `sin(ratio * pi/2)` and its complement, so one swells out of the middle of
//! the other. Between the two idle shapes they only shrink to nine tenths,
//! which reads as a twitch; going to or from an answer they shrink to a half,
//! which reads as a transformation.
//!
//! It speaks about three times a second and is almost always idling: an
//! utterance is an answer only if `frand` beats a confidence of 0.06, and an
//! answer is never followed by another, so roughly one in fifteen is a yes or a
//! no. Answers hold for twice as long as an idle twitch.
//!
//! Across the bottom is a waveform that is not a readout of anything. It is a
//! ring buffer filled with 128 plus noise, pulled towards 240 while the Bit is
//! saying yes and 17 while it is saying no, with a random kick applied at each
//! crossing to round off what would otherwise be a square wave. It is drawn
//! five times over in rising brightness, each pass jittering its own vertical
//! noise, and that is where the glow comes from.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

/// Which shape the Bit is wearing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BitState {
    Idle1,
    Idle2,
    No,
    Yes,
}

const MODELS: usize = 4;
const HISTORY_LENGTH: usize = 512;

/// The colour each shape is made of. The two idle ones share theirs.
const COLORS: [[f32; 4]; MODELS] = [
    [0.66, 0.85, 1.00, 1.00],
    [0.66, 0.85, 1.00, 1.00],
    [1.00, 0.12, 0.12, 1.00],
    [0.98, 0.85, 0.30, 1.00],
];

struct TronBit {
    rot: Rotator,
    trackball: Trackball,

    models: [GlList; MODELS],

    /// Seconds between utterances.
    frequency: f64,
    /// How often an utterance is an answer rather than a twitch.
    confidence: f64,

    last_time: f64,
    /// What it has said, most recently at `history_fp`. Only the last two
    /// entries are ever drawn; the rest is upstream's buffer, kept because the
    /// indexing wraps through it.
    history: [BitState; HISTORY_LENGTH],
    history_fp: usize,
    /// The waveform along the bottom.
    histogram: [u8; HISTORY_LENGTH],
    histogram_fp: usize,

    speed: f32,
    wireframe: bool,
    /// A key the viewer pressed, which forces the next utterance.
    kbd: Option<char>,
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let speed = g.res.float("speed") as f32;
    let do_spin = g.res.bool("spin");
    let do_wander = g.res.bool("wander");

    let spin_speed = 3.0;
    let wander_speed = 0.03 * speed as f64;
    let spin_accel = 4.0;

    let mut this = TronBit {
        rot: Rotator::new(
            if do_spin { spin_speed } else { 0.0 },
            if do_spin { spin_speed } else { 0.0 },
            if do_spin { spin_speed } else { 0.0 },
            spin_accel,
            if do_wander { wander_speed } else { 0.0 },
            false,
        ),
        trackball: Trackball::new(),
        models: [
            GlList::parse(crate::models::TRONBIT_IDLE1),
            GlList::parse(crate::models::TRONBIT_IDLE2),
            GlList::parse(crate::models::TRONBIT_NO),
            GlList::parse(crate::models::TRONBIT_YES),
        ],
        frequency: 0.30 / speed as f64, // parity around 3x/second
        confidence: 0.06,               // provide answer 1/15 or so
        last_time: 0.0,
        history: [BitState::Idle1; HISTORY_LENGTH],
        history_fp: 0,
        histogram: [128; HISTORY_LENGTH],
        histogram_fp: 0,
        speed,
        wireframe: g.res.bool("wireframe"),
        kbd: None,
    };

    for h in &mut this.histogram {
        *h = (128 + (random() % 16) as i32 - 8) as u8;
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl TronBit {
    /// `make_bit`. Sets the shape's material and draws it in its own alignment.
    ///
    /// Upstream compiles this into a display list once. The material calls are
    /// state rather than geometry and the recorder does not keep those in a
    /// list, so it runs afresh every frame instead.
    fn draw_model(&self, g: &mut Gl, which: BitState) {
        let i = which as usize;
        let color = COLORS[i];

        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);
        g.glx.material_ambient_diffuse(color);
        g.glx.color4f(color[0], color[1], color[2], color[3]);

        g.glx.push_matrix();
        // Line up the shapes with each other. Hand-tuned upstream.
        let s = match which {
            BitState::Idle1 => {
                g.glx.rotate(-44.0, 0.0, 1.0, 0.0);
                g.glx.rotate(-11.0, 1.0, 0.0, 0.0);
                g.glx.rotate(8.0, 0.0, 0.0, 1.0);
                1.0
            }
            BitState::Idle2 => {
                g.glx.rotate(16.0, 0.0, 0.0, 1.0);
                g.glx.rotate(-28.0, 1.0, 0.0, 0.0);
                1.0
            }
            BitState::No => {
                g.glx.rotate(16.0, 0.0, 0.0, 1.0);
                g.glx.rotate(-28.0, 1.0, 0.0, 0.0);
                1.6
            }
            BitState::Yes => {
                g.glx.rotate(-44.0, 0.0, 1.0, 0.0);
                g.glx.rotate(-32.0, 1.0, 0.0, 0.0);
                1.53
            }
        };
        g.glx.scale(s, s, s);
        self.models[i].render(&mut g.glx, self.wireframe);
        g.glx.pop_matrix();
    }

    /// `tick_bit`. Advances the waveform every frame, and the Bit itself only
    /// when its clock says so.
    fn tick(&mut self, now: f64) {
        let n = self.history[self.history_fp];
        let mut freq = self.frequency;
        if n == BitState::Yes || n == BitState::No {
            freq *= 2.0;
        }

        if self.trackball.button_down() {
            return;
        }

        let histogram_speed = ((3.0 * self.speed) as i32).max(1);
        for _ in 0..histogram_speed {
            let mut nn: i32 = match n {
                BitState::Yes => 240,
                BitState::No => 17,
                _ => 128,
            };
            let previous = if self.histogram_fp == 0 {
                // `(fp - 1) % 512` in C, which at zero is zero rather than the
                // last slot: the index is signed and -1 % 512 is -1, which the
                // array subscript then reads as itself. Kept because the noise
                // it shapes is visible.
                0
            } else {
                self.histogram_fp - 1
            };
            let on = self.histogram[previous] as i32;

            // Smooth out the square wave a little bit.
            let mid = |v: i32| v > 100 && v < 200;
            if mid(nn) != mid(on) {
                nn += ((random() % 48) as i32 - 32) * if mid(on) { 1 } else { -1 };
            }
            nn += (random() % 16) as i32 - 8;

            self.histogram_fp += 1;
            if self.histogram_fp >= HISTORY_LENGTH {
                self.histogram_fp = 0;
            }
            self.histogram[self.histogram_fp] = nn.clamp(0, 255) as u8;
        }

        if self.last_time + freq > now && self.kbd.is_none() {
            return;
        }
        self.last_time = now;

        self.history_fp += 1;
        if self.history_fp >= HISTORY_LENGTH {
            self.history_fp = 0;
        }

        let next = if let Some(c) = self.kbd.take() {
            match c {
                '1' => BitState::Yes,
                '0' => BitState::No,
                _ if random() & 1 != 0 => BitState::Yes,
                _ => BitState::No,
            }
        } else if n == BitState::Yes || n == BitState::No || frand(1.0) >= self.confidence {
            if n == BitState::Idle1 {
                BitState::Idle2
            } else {
                BitState::Idle1
            }
        } else if random() & 1 != 0 {
            BitState::Yes
        } else {
            BitState::No
        };

        self.history[self.history_fp] = next;
    }

    /// `animate_bits`. Both shapes at once, the old one shrinking and the new
    /// one growing through the same middle.
    fn animate(&self, g: &mut Gl, omodel: BitState, nmodel: BitState, ratio: f32) {
        let scale = (ratio * std::f32::consts::PI / 2.0).sin();

        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wireframe {
            g.glx.lighting(true);
        }

        let idling = |m: BitState| m == BitState::Idle1 || m == BitState::Idle2;
        let small: f32 = if idling(omodel) && idling(nmodel) {
            0.9
        } else {
            0.5
        };

        let nsize = small + (1.0 - small) * scale;
        let osize = small + (1.0 - small) * (1.0 - scale);

        g.glx.push_matrix();
        g.glx.scale(osize, osize, osize);
        self.draw_model(g, omodel);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.scale(nsize, nsize, nsize);
        self.draw_model(g, nmodel);
        g.glx.pop_matrix();
    }

    /// `draw_histogram`. Five passes of the same buffer in rising brightness,
    /// each with its own vertical jitter, drawn in screen coordinates
    /// underneath everything else.
    fn draw_histogram(&self, g: &mut Gl) {
        let samples = HISTORY_LENGTH;
        let scalex = g.width() as f32 / samples as f32;
        let scaley = g.height() as f32 / 255.0 / 4.0; // about 1/4th of screen
        let overlays = 5;

        g.glx.texturing(false);
        g.glx.lighting(false);
        g.glx.blend(Blend::Off);
        g.glx.depth_test(false);
        g.glx.cull_face(false);

        g.glx.matrix_mode_projection();
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx
            .ortho(0.0, g.width() as f32, 0.0, g.height() as f32, -1.0, 1.0);

        for k in 0..overlays {
            let a = k as f32 / overlays as f32;
            g.glx.color3f(0.3 * a, 0.7 * a, 1.0 * a);
            g.glx.begin(Shape::LineStrip);

            let mut j = self.histogram_fp + 1;
            for i in 0..samples {
                if j >= samples {
                    j = 0;
                }
                let x = i as f32 * scalex;
                let mut y = self.histogram[j] as f32;
                y += ((random() % 16) as i32 - 8) as f32;
                y += 16.0; // margin at bottom of screen
                g.glx.vertex3f(x, y * scaley, 0.0);
                j += 1;
            }
            g.glx.end();
        }

        g.glx.pop_matrix();
        g.glx.matrix_mode_projection();
        g.glx.pop_matrix();
        g.glx.matrix_mode_modelview();
    }
}

impl Hack3d for TronBit {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width as f32;
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event {
            // Upstream also takes the arrow and page keys, which the host does
            // not deliver; the digits and the space it does.
            let c = match *key {
                '1' => Some('1'),
                '0' => Some('0'),
                ' ' | '\t' | '\n' => Some(*key),
                _ => None,
            };
            if let Some(c) = c {
                self.kbd = Some(c);
                return true;
            }
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
        }

        g.glx.push_matrix();
        g.glx.scale(1.1, 1.1, 1.1);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 11.0,
            (y as f32 - 0.5) * 5.0,
            (z as f32 - 0.5) * 3.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(6.0, 6.0, 6.0);

        let nmodel = self.history[self.history_fp];
        let omodel = self.history[if self.history_fp > 0 {
            self.history_fp - 1
        } else {
            HISTORY_LENGTH - 1
        }];
        let now = g.time;
        let ratio =
            (1.0 - ((self.last_time + self.frequency) - now) / self.frequency).min(1.0) as f32;

        self.draw_histogram(g);
        self.animate(g, omodel, nmodel, ratio);
        self.tick(now);

        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        30",
    "*showFPS:      False",
    "*wireframe:    False",
    "*spin:         True",
    "*wander:       True",
    "*speed:        1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.05, 20.0, 0.05, 2, "1.0"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "tronbit",
    label: "TronBit",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2011",
        video: Some("https://www.youtube.com/watch?v=dIF4fodt-L8"),
        blurb: "The character \"Bit\" from the film \"Tron\". The \"yes\" state is a \
                tetrahedron; the \"no\" state is the second stellation of an \
                icosahedron; and the idle state oscillates between a small triambic \
                icosahedron and the compound of an icosahedron and a dodecahedron.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    /// The two idle shapes are 240 and 168 vertices, so a frame that draws
    /// both is 408. One alone would be under 250.
    const IDLE_PAIR: usize = 240 + 168;

    #[test]
    fn two_shapes_are_always_on_screen_at_once() {
        // The Bit never cuts. Even at rest it is drawing the shape it is
        // leaving as well as the one it is arriving at, which is why a still of
        // it usually shows one polyhedron growing out of another.
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let lit: usize = f
            .batches
            .iter()
            .filter(|b| b.lighting)
            .map(|b| b.count)
            .sum();
        assert!(lit >= IDLE_PAIR, "expected two shapes, got {lit} vertices");
    }

    #[test]
    fn the_waveform_is_drawn_five_times_over_in_rising_brightness() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let mut blues: Vec<f32> = f
            .batches
            .iter()
            .filter(|b| !b.lighting)
            .flat_map(|b| f.vertices[b.first..b.first + b.count].iter().take(1))
            .map(|v| v.color[2])
            .collect();
        blues.sort_by(f32::total_cmp);
        blues.dedup();
        // 0.0, 0.2, 0.4, 0.6, 0.8: the darkest pass is black and invisible.
        assert_eq!(blues.len(), 5, "got {blues:?}");
        assert!((blues[4] - 0.8).abs() < 1e-5, "brightest is {}", blues[4]);
    }

    #[test]
    fn the_waveform_sits_in_the_bottom_quarter_of_the_screen() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let top = f
            .batches
            .iter()
            .filter(|b| !b.lighting)
            .flat_map(|b| f.vertices[b.first..b.first + b.count].iter())
            .map(|v| v.pos[1])
            .fold(0.0_f32, f32::max);
        // The band is a quarter of the screen and the trace idles at 128 of the
        // 255 it is scaled against, so it sits around the middle of that band
        // and only reaches the top of it while the Bit is saying yes.
        let band = 480.0 / 4.0;
        assert!(top < band, "waveform leaves its band at {top}");
        assert!(top > band / 2.0, "waveform is flat at the bottom, {top}");
    }

    #[test]
    fn it_idles_far_more_often_than_it_answers() {
        // Confidence is 0.06 and an answer is never followed by another, so
        // about one utterance in fifteen should be a yes or a no. Read off the
        // rendered frame, which is the only place the state shows.
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let (mut utterances, mut answers, mut last) = (0, 0, None);
        for _ in 0..2000 {
            r.step();
            let f = r.frame();
            // Red and yellow both have a diffuse red near one; the idle blue is
            // 0.66, and both shapes carry the same colour while idling.
            let answering = f
                .batches
                .iter()
                .any(|b| b.lighting && b.material.ambient_diffuse[0] > 0.9);
            if last != Some(answering) {
                utterances += 1;
                if answering {
                    answers += 1;
                }
                last = Some(answering);
            }
        }
        assert!(utterances > 10, "only {utterances} changes of shape");
        assert!(answers > 0, "never answered at all");
        let ratio = answers as f32 / utterances as f32;
        assert!(ratio < 0.5, "answered on {ratio} of the changes");
    }

    #[test]
    fn a_keypress_makes_it_answer_at_once() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        r.event(XEvent::KeyPress { key: '1' });
        // The forced answer lands on the next tick, whatever the clock says.
        r.step();
        r.step();
        let f = r.frame();
        let yellow = f.batches.iter().any(|b| {
            b.lighting && b.material.ambient_diffuse[0] > 0.9 && b.material.ambient_diffuse[1] > 0.8
        });
        assert!(yellow, "pressing 1 did not make it say yes");
    }
}
