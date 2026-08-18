//! Port of `hacks/glx/unknownpleasures.c`.
//!
//! ```text
//! unknownpleasures, Copyright © 2013-2025 Jamie Zawinski <jwz@jwz.org>
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
//! The Joy Division album cover: eighty stacked periods of the pulsar
//! PSR B1919+21, as printed in the Cambridge Encyclopedia of Astronomy.
//!
//! A waterfall chart, which is a picture made of two things. Each row is one
//! signal, drawn as a line; and behind that line, in the background colour, is
//! a filled strip running from the line down to the baseline. The strip is what
//! makes the plot read as a landscape rather than a tangle: it hides the rows
//! further back wherever this row rises over them, so a peak occludes what is
//! behind it and the whole stack acquires depth from nothing but drawing order.
//!
//! A signal is not a recording of anything. It is between six and twenty-one
//! cosine bumps of random amplitude, frequency and offset added together,
//! normalised, then multiplied by `cos(r^2 * pi * 14)` clipped to one period,
//! which is a bell that pins both ends of the row to the baseline and leaves
//! the activity in the middle. A little static is added per point.
//!
//! It scrolls by one row at a time and the two rows at either end fade out, so
//! nothing appears or vanishes abruptly: the height and the alpha of a row are
//! the same eased number, its distance from the nearest edge over a twentieth
//! of the stack.
//!
//! The default projection is barely a projection at all: a one-degree field of
//! view from seven hundred units away, which is orthographic to within a
//! rounding error and matches the flat original. The perspective alternative is
//! the usual thirty degrees from thirty units, and clicking swaps between them.
//!
//! Upstream can also mask the plot with an image, drawing the signal only where
//! the picture is bright. That knob takes a filename, which a browser has
//! nothing to do with, so it is not ported.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::unrgb;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Ease, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, ease, frand,
    random, screenhack_event_helper,
};

/// One row of the chart: its signal, once per buzz frame.
///
/// Buzzing redraws the same row from a different sample of the static every
/// frame, which upstream does by compiling fifteen display lists per row and
/// picking one at random. The variants are the same idea as data.
struct Row {
    /// `frames` takes of the same signal, each `resolution` heights long.
    takes: Vec<Vec<f32>>,
}

struct Unk {
    trackball: Trackball,
    orthop: bool,
    speed: f64,
    tick: f64,
    /// Points across.
    resolution: usize,
    /// Rows down. Set by `reshape`, not by the knob directly.
    count: usize,
    /// How tall a row is allowed to be.
    amplitude: f32,
    /// How many takes of each row to keep, for buzzing.
    frames: usize,
    /// Peaks in a row.
    noise: f32,
    /// Shape of the plot.
    aspect: f32,
    /// Oldest first, so the newest row is at the bottom of the stack.
    rows: Vec<Row>,
    /// Each row's eased elevation, which is also its alpha.
    heights: Vec<f32>,
    fg: [f32; 4],
    bg: [f32; 4],
    wireframe: bool,
    /// What the knob asked for, before `reshape` cuts it down for a small
    /// window. Kept because a resize has to recompute from it.
    want_count: usize,
    /// The extra scale `reshape` works out from the window size.
    scale: f32,
}

/// Like `cos` but constrained to `[-pi, +pi]`.
fn cos1(th: f32) -> f32 {
    th.clamp(-std::f32::consts::PI, std::f32::consts::PI).cos()
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let colour = |g: &Gl, key: &str| {
        let (r, gr, b) = unrgb(g.res.pixel(key));
        [r as f32 / 255.0, gr as f32 / 255.0, b as f32 / 255.0, 1.0]
    };

    let mut this = Unk {
        trackball: Trackball::new(),
        orthop: g.res.bool("ortho"),
        speed: g.res.float("speed").max(0.001),
        tick: 0.0,
        resolution: (g.res.int("resolution").clamp(1, 300)) as usize,
        count: 0,
        amplitude: (g.res.float("amplitude") as f32).clamp(0.01, 1.0),
        frames: if g.res.bool("buzz") { 15 } else { 1 },
        noise: (g.res.float("noise") as f32).max(0.01),
        aspect: g.res.float("aspect") as f32,
        rows: Vec::new(),
        heights: Vec::new(),
        fg: colour(g, "foreground"),
        bg: colour(g, "background"),
        wireframe: g.res.bool("wireframe"),
        want_count: g.res.int("count").max(1) as usize,
        scale: 1.0,
    };

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Unk {
    /// `generate_signal`. A sum of random cosine bumps, normalised, then
    /// pinched at both ends by a bell and roughened with static.
    fn generate_signal(&self) -> Vec<f32> {
        let n = self.resolution;
        let mut points = vec![0.0f32; n];
        let nspikes = ((6.0 + frand(15.0)) as f32 * self.noise) as i32;
        let step = 1.0 / n as f32;

        for _ in 0..nspikes {
            let off = frand(0.8) as f32 - 0.4; // leave a margin
            let amp = (0.1 + frand(0.9) as f32) * nspikes as f32;
            let freq = (7.0 + frand(11.0) as f32) * self.noise;
            let mut r = -0.5f32;
            for p in points.iter_mut() {
                *p += amp / 2.0 + amp / 2.0 * cos1((r + off) * std::f32::consts::PI * 2.0 * freq);
                r += step;
            }
        }

        // Avoid clipping.
        let mut max = (nspikes as f32).max(1.0);
        for p in &points {
            if max < *p {
                max = *p;
            }
        }

        // Multiply by baseline clipping curve, add static.
        let mut r = -0.5f32;
        for p in points.iter_mut() {
            *p = (*p / max)
                * (0.5
                    + 0.5 * cos1(r * r * std::f32::consts::PI * 14.0) * (1.0 - frand(0.2) as f32));
            r += step;
        }
        points
    }

    /// `tick_unk`. Roll the stack forward by one and generate the row that
    /// takes the vacated place at the bottom.
    fn add_row(&mut self) {
        let signal = self.generate_signal();
        let takes = (0..self.frames)
            .map(|_| {
                signal
                    .iter()
                    .map(|z| ((z + frand(0.05) as f32) * self.amplitude).clamp(0.0, self.amplitude))
                    .collect()
            })
            .collect();

        if self.rows.len() >= self.count && !self.rows.is_empty() {
            self.rows.remove(0);
        }
        self.rows.push(Row { takes });

        if self.heights.len() >= self.count && !self.heights.is_empty() {
            self.heights.remove(0);
        }
        self.heights.push(0.0);
    }

    /// The plinth the plot sits on: a slab and a border line.
    fn draw_base(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let h1 = 0.01;
        let h2 = 0.02;
        let h3 = (h1 + h2) / 2.0;
        let mut s = 0.505;

        let face = |g: &mut Gl, vs: [[f32; 3]; 4]| {
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });
            for v in vs {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();
        };

        face(g, [[-s, -s, -h1], [s, -s, -h1], [s, s, -h1], [-s, s, -h1]]);
        face(
            g,
            [[-s, -s, 0.0], [-s, -s, -h2], [s, -s, -h2], [s, -s, 0.0]],
        );
        face(
            g,
            [[-s, -s, 0.0], [-s, -s, -h2], [-s, s, -h2], [-s, s, 0.0]],
        );
        face(g, [[s, -s, 0.0], [s, -s, -h2], [s, s, -h2], [s, s, 0.0]]);
        face(g, [[-s, s, 0.0], [-s, s, -h2], [s, s, -h2], [s, s, 0.0]]);

        g.glx.color3f(self.fg[0], self.fg[1], self.fg[2]);
        s -= 0.01;
        g.glx.begin(Shape::LineLoop);
        for v in [[-s, -s, -h3], [s, -s, -h3], [s, s, -h3], [-s, s, -h3]] {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();
    }

    /// One row: the signal as a line, then the strip that hides what is behind
    /// it. The strip goes down in the background colour, which is the whole of
    /// how the plot occludes itself.
    fn draw_row(&self, g: &mut Gl, take: &[f32], alpha: f32) {
        let n = self.resolution as f32;

        g.glx.color4f(self.fg[0], self.fg[1], self.fg[2], alpha);
        g.glx.begin(Shape::LineStrip);
        for (i, z) in take.iter().enumerate() {
            g.glx.vertex3f(i as f32 / n, 0.0, *z);
        }
        g.glx.end();

        if self.wireframe {
            g.glx
                .color4f(0.5 * self.fg[0], 0.5 * self.fg[1], 0.5 * self.fg[2], 1.0);
        } else {
            g.glx.color4f(self.bg[0], self.bg[1], self.bg[2], 1.0);
        }
        g.glx.begin(if self.wireframe {
            Shape::Lines
        } else {
            Shape::QuadStrip
        });
        for (i, z) in take.iter().enumerate() {
            let x = i as f32 / n;
            g.glx.vertex3f(x, 0.0, *z);
            g.glx.vertex3f(x, 0.0, 0.0);
        }
        g.glx.end();
    }
}

impl Hack3d for Unk {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let (mut height, mut y) = (height, 0);
        let mut h = height as f32 / width as f32;
        if width > height * 5 {
            // Tiny window: show the middle.
            height = (width as f32 * 1.5) as i32;
            y = -height / 2;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        if self.orthop {
            // A one-degree field of view from seven hundred units away is
            // orthographic to within a rounding error. The near and far planes
            // are kept thirty apart because polygon offset stops working when
            // the depth range is large.
            let magic = 700.0;
            let range = 30.0;
            g.glx.matrix_mode_projection();
            g.glx
                .perspective(1.0, 1.0 / h, magic - range / 2.0, magic + range / 2.0);
            g.glx.matrix_mode_modelview();
            g.glx.load_identity();
            g.glx
                .look_at([0.0, 0.0, magic], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
            if width < height {
                g.glx.scale(1.0 / h, 1.0 / h, 1.0);
            }
            g.glx.translate(0.0, -0.5, 0.0);
        } else {
            g.glx.matrix_mode_projection();
            g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);
            g.glx.matrix_mode_modelview();
            g.glx.load_identity();
            g.glx
                .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
            if width < height {
                g.glx.scale(1.0 / h, 1.0 / h, 1.0);
            }
        }

        let mut new_count = self.want_count;
        if g.width() <= 480 || g.height() <= 480 {
            new_count /= 2;
        }
        if self.wireframe {
            new_count /= 2;
        }
        let new_count = new_count.max(1);

        if self.count != new_count {
            self.count = new_count;
            self.heights.truncate(new_count);
            self.rows.truncate(new_count);
            // Upstream drops the display lists here and lets the plot refill a
            // row at a time; filling them straight away costs nothing and means
            // a resize does not blank the picture.
            while self.rows.len() < new_count {
                self.add_row();
            }
        }

        // Make the image fill the screen a little more fully on a small one.
        let mut s: f32 = 1.0;
        if g.width() <= 640 || g.height() <= 640 {
            s = 1.2;
        }
        s /= 1.9 / self.aspect;
        self.scale = s;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) {
            self.orthop = !self.orthop;
            let (w, h) = (g.width(), g.height());
            self.reshape(g, w, h);
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let step = 1.0 / self.count as f32;
        let speed = (0.6 / self.speed) * (80.0 / self.count as f64);
        let now = g.time;
        let ratio = (((now - self.tick) / speed) as f32).clamp(0.0, 1.0);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(false);
        if !self.wireframe {
            g.glx.blend(Blend::Alpha);
        }

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.rotate(-45.0, 1.0, 0.0, 0.0);

        let i: f32 = if self.orthop { 15.0 } else { 17.0 };
        let z = i * self.scale;
        g.glx.scale(z / self.aspect, z, z / self.aspect);

        if self.wireframe {
            g.glx
                .color3f(0.5 * self.fg[0], 0.5 * self.fg[1], 0.5 * self.fg[2]);
        } else {
            g.glx
                .color4f(self.bg[0], self.bg[1], self.bg[2], self.bg[3]);
        }
        self.draw_base(g);

        g.glx.translate(-0.5, 0.55 + step * ratio, 0.0);

        let frame = (random() as usize) % self.frames;
        for i in 0..self.rows.len() {
            let s = ease(Ease::InOutSine, self.heights[i] as f64) as f32;
            // Upstream eases the alpha from one and a half times the height and
            // does not clamp, so it crosses full opacity while a row is still
            // rising and settles back to a half once the row is fully up. The
            // body of the plot is therefore half alpha and the row coming in is
            // briefly the brightest thing on screen.
            let s2 = ease(Ease::InOutSine, (self.heights[i] * 1.5) as f64) as f32;
            g.glx.push_matrix();
            g.glx.scale(1.0, 1.0, s);
            let take = &self.rows[i].takes[frame % self.rows[i].takes.len()];
            // Cloning would be tidier than this dance, but the row is the hot
            // path and a copy per frame per row is not free.
            let take: Vec<f32> = take.clone();
            self.draw_row(g, &take, s2);
            g.glx.pop_matrix();
            g.glx.translate(0.0, -step, 0.0);
        }

        g.glx.pop_matrix();

        if !self.trackball.button_down() {
            // Set height and fade from the distance to the nearer edge.
            let dist = self.count as f32 * 0.05;
            for i in 0..self.heights.len() {
                let i2 = i as f32 - ratio;
                let half = (self.count / 2) as f32;
                let h = if i2 < half {
                    i2
                } else {
                    self.count as f32 - 1.0 - i2
                } / dist;
                self.heights[i] = h.clamp(0.0, 1.0);
            }

            if self.tick + speed <= now {
                self.add_row();
                self.tick = now;
            }
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        80",
    "*showFPS:      False",
    "*wireframe:    False",
    ".foreground:   white",
    ".background:   black",
    "*ortho:        True",
    "*speed:        1.0",
    "*resolution:   100",
    "*amplitude:    0.13",
    "*noise:        1.0",
    "*aspect:       1.9",
    "*buzz:         False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("count", "Scanlines", 3.0, 200.0, 1.0, 0, "80"),
    Opt::slider("speed", "Speed", 0.1, 20.0, 0.1, 1, "1.0"),
    Opt::slider("resolution", "Resolution", 5.0, 300.0, 1.0, 0, "100"),
    Opt::slider("amplitude", "Amplitude", 0.01, 0.25, 0.01, 2, "0.13"),
    Opt::slider("noise", "Noise", 0.0, 3.0, 0.1, 1, "1.0"),
    Opt::boolean("ortho", "Orthographic projection", "true"),
    Opt::boolean("buzz", "Buzz", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "unknownpleasures",
    label: "Unknown Pleasures",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2013",
        video: Some("https://www.youtube.com/watch?v=DEWPiUbwnt0"),
        blurb: "PSR B1919+21 was the first pulsar ever discovered. An illustration \
                of its signal was published in 1971, and later seen by the drummer \
                of Joy Division, and consequently appropriated for the cover of \
                the band's album \"Unknown Pleasures\".",
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

    #[test]
    fn every_row_is_a_line_and_a_strip_that_hides_what_is_behind_it() {
        let mut r = start(StartArgs::new(800, 800, "count=20&resolution=50", 20260811));
        r.step();
        let f = r.frame();
        let lines = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::LineStrip)
            .count();
        // A quad strip is cut into triangles by the recorder.
        let strips = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
            .count();
        // Twenty rows, each a line and a strip, and the strip is what makes it
        // a landscape rather than a tangle.
        assert_eq!(lines, 20, "got {lines} signal lines");
        // Twenty strips plus the base, whose five faces share their state and
        // fold into one batch.
        assert_eq!(strips, 21, "got {strips} occluding strips");
    }

    #[test]
    fn the_ends_of_a_row_are_pinned_to_the_baseline() {
        // The bell that multiplies every signal is what gives the picture its
        // shape: activity in the middle, flat at both edges.
        let mut r = start(StartArgs::new(
            800,
            800,
            "count=10&resolution=100",
            20260811,
        ));
        r.step();
        let f = r.frame();
        let line = f
            .batches
            .iter()
            .find(|b| b.primitive == crate::runtime::gl::Primitive::LineStrip)
            .expect("no signal was drawn");
        let zs: Vec<f32> = f.vertices[line.first..line.first + line.count]
            .iter()
            .map(|v| v.pos[2])
            .collect();
        let edge = zs[0].max(zs[zs.len() - 1]);
        let peak = zs.iter().copied().fold(0.0_f32, f32::max);
        assert!(peak > 0.0, "the row is flat");
        assert!(edge < peak * 0.25, "edge {edge} against peak {peak}");
    }

    #[test]
    fn a_row_never_grows_past_the_amplitude_it_was_given() {
        let mut r = start(StartArgs::new(
            800,
            800,
            "count=10&resolution=60&amplitude=0.05",
            20260811,
        ));
        for _ in 0..40 {
            r.step();
        }
        let f = r.frame();
        let top = f.vertices.iter().map(|v| v.pos[2]).fold(0.0_f32, f32::max);
        assert!(top <= 0.05 + 1e-6, "a row reached {top}");
        assert!(top > 0.02, "nothing reached anywhere near the ceiling");
    }

    #[test]
    fn the_stack_scrolls_and_the_far_ends_fade() {
        let mut r = start(StartArgs::new(800, 800, "count=20&resolution=40", 20260811));
        let alphas = |r: &Runner3d| -> Vec<f32> {
            let f = r.frame();
            f.batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::LineStrip)
                .map(|b| f.vertices[b.first].color[3])
                .collect()
        };

        // The heights are worked out at the end of a frame, so the very first
        // one draws nothing at all. Upstream does the same.
        r.step();

        // Run long enough for several rows to have rolled through.
        let mut bottom_seen: Vec<f32> = Vec::new();
        for _ in 0..200 {
            r.step();
            let a = alphas(&r);
            assert_eq!(a.len(), 20);
            // The oldest row is always on its way out, and the middle of the
            // stack is always solid.
            assert_eq!(a[0], 0.0, "top row is at {}", a[0]);
            // Half, not one: see the note on the alpha in `draw`.
            assert!((a[10] - 0.5).abs() < 1e-5, "the middle is at {}", a[10]);
            bottom_seen.push(a[19]);
        }
        // The newest row fades in over one scroll step rather than appearing.
        let lo = bottom_seen.iter().copied().fold(f32::MAX, f32::min);
        let hi = bottom_seen.iter().copied().fold(0.0_f32, f32::max);
        assert!(lo < 0.2 && hi > 0.8, "bottom row ran {lo} to {hi}");
        // And on its way in it passes through full opacity, which the settled
        // rows behind it never reach.
        assert!(hi > bottom_seen[bottom_seen.len() - 1]);
    }

    #[test]
    fn clicking_swaps_the_projection() {
        let mut r = start(StartArgs::new(800, 800, "", 20260811));
        r.step();
        let before = r.frame().batches[0].mvp;
        r.event(XEvent::KeyPress { key: ' ' });
        r.step();
        let after = r.frame().batches[0].mvp;
        assert_ne!(before, after, "the projection did not change");
    }

    #[test]
    fn buzzing_keeps_more_than_one_take_of_each_row() {
        let mut r = start(StartArgs::new(
            800,
            800,
            "count=8&resolution=30&buzz=true",
            20260811,
        ));
        let mut seen = std::collections::HashSet::new();
        for _ in 0..60 {
            r.step();
            let f = r.frame();
            if let Some(b) = f
                .batches
                .iter()
                .find(|b| b.primitive == crate::runtime::gl::Primitive::LineStrip)
            {
                let bits: Vec<u32> = f.vertices[b.first..b.first + b.count]
                    .iter()
                    .map(|v| v.pos[2].to_bits())
                    .collect();
                seen.insert(bits);
            }
        }
        // Without buzz the top row would be identical until it scrolled off.
        assert!(seen.len() > 5, "only {} distinct takes", seen.len());
    }
}
