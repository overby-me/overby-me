//! Port of `hacks/glx/gltext.c`.
//!
//! ```text
//! gltext, Copyright (c) 2001-2021 Jamie Zawinski <jwz@jwz.org>
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
//! Text in three dimensions, turning.
//!
//! The letters are not solid: they are the polylines of a vector font with a
//! tube drawn along every segment and a sphere at every joint. The spheres are
//! wasteful on a curve like a nought and necessary on a corner like a four,
//! which is upstream's reason for putting one at every point rather than
//! working out which corners need them.
//!
//! Upstream reads up to eight lines from a program that only ever prints
//! three; the channel here never runs out, so the line count is what stops it
//! and three is what upstream actually shows. Each line is some three and a
//! half thousand vertices, so this matters.
//!
//! Facing front is not a rotation that is undone: the tilt is a wander over a
//! ninety degree range, so the text sways about the camera rather than turning
//! away from it, and the swaying axes are the ones the spin would have used.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::glutstroke::{MONO_ROMAN, ROMAN, StrokeFont};
use crate::runtime::rotator::Rotator;
use crate::runtime::shapes::unit_sphere;
use crate::runtime::tube::TubeMesh;
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent};

/// How densely to render the tubes. Upstream draws them smooth.
const TUBE_FACES: i32 = 12;
const TUBE_WIDTH: f32 = 10.0;

struct GlText {
    trackball: Trackball,
    rot: Rotator,
    rot2: Rotator,
    spin: [bool; 3],
    colors: Vec<XColor>,
    ccolor: usize,
    text: String,
    /// How many seconds until the text is read again, or none.
    reload: f64,
    last_update: f64,
    /// The tube along a stroke and the ball at its ends, both built once.
    tube: TubeMesh,
    sphere: u32,
    max_lines: usize,
    scale_factor: f32,
    aspect: f32,
    scale: f32,
    face_front: bool,
    mono: bool,
    wire: bool,
}

impl GlText {
    fn font(&self) -> &'static StrokeFont {
        if self.mono { &MONO_ROMAN } else { &ROMAN }
    }

    /// One character, as tubes along its strokes and balls at the joints.
    /// Returns how far along to move for the next one.
    fn fill_character(&self, g: &mut crate::runtime::gl::Glx, c: char) -> f32 {
        let font = self.font();
        let Some(ch) = font.char_at(c) else {
            return 0.0;
        };
        for stroke in ch.strokes {
            let mut last = [0.0f32, 0.0];
            for (j, p) in stroke.iter().enumerate() {
                if j > 0 {
                    self.tube.draw(
                        g,
                        [last[0], last[1], 0.0],
                        [p[0], p[1], 0.0],
                        TUBE_WIDTH,
                        TUBE_WIDTH * 0.15,
                    );
                }
                last = *p;

                // A ball at the end of every segment. Wasteful on a curve
                // like a nought but necessary on a corner like a four.
                if !self.wire {
                    g.push_matrix();
                    g.translate(p[0], p[1], 0.0);
                    g.scale(TUBE_WIDTH, TUBE_WIDTH, TUBE_WIDTH);
                    g.call_list(self.sphere);
                    g.pop_matrix();
                }
            }
        }
        ch.right + TUBE_WIDTH
    }

    /// How wide and how tall the text is, and how many lines it has.
    fn text_extents(&self, s: &str) -> (f32, f32, usize) {
        let font = self.font();
        let line_height = font.line_height();
        let mut w: f32 = 0.0;
        let mut h = 0.0;
        let mut lines = 0;
        for line in s.split('\n') {
            w = w.max(font.length(line));
            h += line_height;
            lines += 1;
        }
        (w, h, lines)
    }

    /// The whole text, centred, a line at a time.
    fn fill_string(&self, g: &mut crate::runtime::gl::Glx, s: &str) {
        let font = self.font();
        let line_height = font.line_height();
        let (ow, oh, _) = self.text_extents(s);
        let mut y = oh / 2.0 - line_height;

        for line in s.split('\n') {
            // Centring, so the whitespace at either end is not wanted.
            let line = line.trim_matches(|c: char| c.is_whitespace());
            let line_w = font.length(line);
            let mut x = (-ow / 2.0) + ((ow - line_w) / 2.0);
            for c in line.chars() {
                g.push_matrix();
                g.translate(x, y, 0.0);
                x += self.fill_character(g, c);
                g.pop_matrix();
            }
            y -= line_height;
        }
    }

    /// Read the text again. With no host this is the compiled-in passage,
    /// cut to the line count.
    fn parse_text(&mut self, g: &mut Gl) {
        let mut buf = String::new();
        let mut lines = 0;
        while buf.len() < 4096 && lines < self.max_lines {
            match g.text_getc() {
                Some(b'\n') => {
                    lines += 1;
                    buf.push('\n');
                }
                // The vector font has no characters past ASCII.
                Some(c) if c.is_ascii() && c != b'\r' => buf.push(c as char),
                Some(_) => {}
                None => break,
            }
        }
        // Hold on to the old text rather than flickering if none arrived.
        if !buf.is_empty() {
            self.text = buf;
        }
        self.reload = if self.text.is_empty() { 1.0 } else { 7.0 };
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let spin_str = g.res.string("spin").to_string();
    let axis = |c: char, d: char| spin_str.contains(c) || spin_str.contains(d);
    let spin = [axis('x', 'X'), axis('y', 'Y'), axis('z', 'Z')];
    let face_front = g.res.bool("faceFront");
    let wander = g.res.bool("wander");

    let sphere = g.glx.gen_lists(1);
    g.glx.new_list(sphere);
    unit_sphere(&mut g.glx, TUBE_FACES, TUBE_FACES, wire);
    g.glx.end_list();

    // Brighter colours, please.
    let mut colors = make_smooth_colormap(255);
    for c in &mut colors {
        c.red = c.red / 2 + 32767;
        c.green = c.green / 2 + 32767;
        c.blue = c.blue / 2 + 32767;
    }

    let mut this = GlText {
        trackball: Trackball::new(),
        rot: Rotator::new(
            if spin[0] { 0.5 } else { 0.0 },
            if spin[1] { 0.5 } else { 0.0 },
            if spin[2] { 0.5 } else { 0.0 },
            0.5,
            if wander { 0.02 } else { 0.0 },
            false,
        ),
        rot2: Rotator::new(0.0, 0.0, 0.0, 0.0, 0.03, true),
        spin,
        colors,
        ccolor: 0,
        text: String::new(),
        reload: 0.0,
        last_update: 0.0,
        tube: TubeMesh::tube(TUBE_FACES, true, false, wire),
        sphere,
        max_lines: g.res.int("maxLines").max(1) as usize,
        scale_factor: g.res.float("scaleFactor") as f32,
        aspect: 1.0,
        scale: 1.0,
        face_front,
        mono: g.res.bool("useMonoSpace"),
        wire,
    };
    // Upstream's default program is `xscreensaver-text --date --cols 20
    // --lines 3`; the same request goes to the channel here.
    g.text_reshape(20, 3);
    this.parse_text(g);
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for GlText {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 9 / 16;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = width as f32 / height as f32;
        self.scale = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        if self.reload > 0.0 && g.time >= self.last_update + self.reload {
            self.parse_text(g);
            self.last_update = g.time;
        }

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, self.aspect, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(!self.wire);
        if !self.wire {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 5.0, 5.0, 10.0, 1.0);
        } else {
            g.glx.lighting(false);
        }

        g.glx.push_matrix();

        let turning = !self.trackball.button_down();
        let (x, y, z) = self.rot.position(turning);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 8.0,
            (z as f32 - 0.5) * 8.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        if self.face_front {
            // Not a turn but a sway: ninety degrees either side of facing the
            // camera, on whichever axes the spin would have used.
            let max = 90.0;
            let (x, y, z) = self.rot2.position(turning);
            if self.spin[0] {
                g.glx.rotate(max / 2.0 - x as f32 * max, 1.0, 0.0, 0.0);
            }
            if self.spin[1] {
                g.glx.rotate(max / 2.0 - y as f32 * max, 0.0, 1.0, 0.0);
            }
            if self.spin[2] {
                g.glx.rotate(max / 2.0 - z as f32 * max, 0.0, 0.0, 1.0);
            }
        } else {
            let (x, y, z) = self.rot.rotation(turning);
            g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
            g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        }

        let c = self.colors[self.ccolor];
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        g.glx.material_ambient_diffuse([
            c.red as f32 / 65536.0,
            c.green as f32 / 65536.0,
            c.blue as f32 / 65536.0,
            1.0,
        ]);
        self.ccolor = (self.ccolor + 1) % self.colors.len();

        let s = self.scale_factor;
        g.glx.scale(s, s, s);
        let text = std::mem::take(&mut self.text);
        let glx = &mut g.glx;
        self.fill_string(glx, &text);
        self.text = text;

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:         20000",
    "*showFPS:       False",
    "*wireframe:     False",
    "*usePty:        False",
    "*text:          (default)",
    "*scaleFactor:   0.01",
    "*wanderSpeed:   0.02",
    "*maxLines:      3",
    "*spin:          XYZ",
    "*wander:        True",
    "*faceFront:     True",
    "*useMonoSpace:  False",
];

const SPINS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "XYZ",
        label: "Rotate around all three axes",
    },
    crate::runtime::opts::SelectItem {
        value: "X",
        label: "Rotate around X",
    },
    crate::runtime::opts::SelectItem {
        value: "Y",
        label: "Rotate around Y",
    },
    crate::runtime::opts::SelectItem {
        value: "Z",
        label: "Rotate around Z",
    },
    crate::runtime::opts::SelectItem {
        value: "XY",
        label: "Rotate around X and Y",
    },
    crate::runtime::opts::SelectItem {
        value: "0",
        label: "Do not rotate",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::select("spin", "Rotation", SPINS, "XYZ"),
    Opt::slider("scaleFactor", "Text size", 0.002, 0.05, 0.001, 3, "0.01"),
    Opt::slider("maxLines", "Lines of text", 1.0, 20.0, 1.0, 0, "3"),
    Opt::boolean("faceFront", "Always face front", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("useMonoSpace", "Monospaced", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "gltext",
    label: "GL Text",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=jrXa-QtY6MU"),
        blurb: "Displays a few lines of text spinning around in 3D.",
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

    /// A letter is tubes along the strokes of a vector font with a ball at
    /// every joint, so it is round rather than flat.
    #[test]
    fn a_letter_is_tubes_and_balls() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "no text was drawn");
        // Nothing is flat: every letter has depth.
        let zs: Vec<f32> = f.vertices.iter().map(|v| v.pos[2]).collect();
        let lo = zs.iter().copied().fold(f32::MAX, f32::min);
        let hi = zs.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 0.0, "the text is flat");
    }

    /// The text is centred: every line is placed by its own width, so the
    /// drawing is about as far left as it is right.
    #[test]
    fn the_text_is_centred() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..5 {
            r.step();
        }
        let f = r.frame();
        let xs: Vec<f32> = f.batches.iter().map(|b| b.modelview.0[12]).collect();
        let lo = xs.iter().copied().fold(f32::MAX, f32::min);
        let hi = xs.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            (lo + hi).abs() < (hi - lo) * 0.75 + 1.0,
            "the text is off to one side: {lo} to {hi}"
        );
    }

    /// The colour walks round a smooth map, a step a frame, and stays bright:
    /// upstream lifts every component halfway to white.
    #[test]
    fn the_colour_cycles_and_stays_bright() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            r.step();
            let f = r.frame();
            if let Some(b) = f.batches.first() {
                let c = b.material.ambient_diffuse;
                assert!(
                    c[0] >= 0.49 || c[1] >= 0.49 || c[2] >= 0.49,
                    "a dull colour: {c:?}"
                );
                seen.insert(format!("{c:?}"));
            }
        }
        assert!(seen.len() > 10, "the colour did not cycle");
    }

    /// The monospaced font puts every character the same distance apart, so
    /// the same text is a different width in it.
    #[test]
    fn the_monospaced_font_is_wider() {
        let vari = ROMAN.length("Hello, world.");
        let mono = MONO_ROMAN.length("Hello, world.");
        assert_ne!(vari, mono);
        let mut r = start(StartArgs::new(640, 480, "useMonoSpace=true", 20260811));
        r.step();
        assert!(!r.frame().vertices.is_empty(), "nothing was drawn");
    }
}
