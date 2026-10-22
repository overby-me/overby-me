//! Port of `hacks/glx/glmatrix.c`.
//!
//! ```text
//! glmatrix, Copyright (c) 2003-2018 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! GLMatrix -- simulate the text scrolls from the movie "The Matrix".
//!
//! This program does a 3D rendering of the dropping characters that
//! appeared in the title sequences of the movies.  See also `xmatrix'
//! for a simulation of what the computer monitors actually *in* the
//! movie did.
//! ```
//!
//! A strip is a column of glyphs with a brighter one falling down it, which
//! reveals the ones below as it passes and then, on a second pass, erases them
//! again. The strips drift toward the camera and are re-made when they reach
//! the glass.
//!
//! The glyphs are one texture of sixteen by thirteen characters, and upstream
//! rearranges it before uploading: the katakana row at the bottom is moved up
//! over two rows nobody uses, which brings the picture to a size that fits an
//! old-fashioned power-of-two texture. That rearrangement is kept, because the
//! glyph numbering assumes it. Its green channel becomes the alpha and the
//! green is set to full, so a glyph is a green shape with a transparent
//! background whatever colour it is drawn in.
//!
//! Everything is drawn additively, which Jeff Epler suggested to upstream: a
//! bright glyph with a darker one in front of it comes out a little brighter
//! than the bright one alone.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};

const CHAR_COLS: usize = 16;
const CHAR_ROWS: usize = 13;

/// Width and height of the arena, its depth, how often the brightness waves
/// repeat, and where in the depth a glyph hits the screen and vanishes.
const GRID_SIZE: usize = 70;
const GRID_DEPTH: f32 = 35.0;
const WAVE_SIZE: usize = 22;
const SPLASH_RATIO: f32 = 0.7;

/// The glyphs each mode draws from, as indexes into the texture.
const MATRIX_ENCODING: &[i32] = &[
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170,
    171, 172, 173, 174, 175,
];
const DECIMAL_ENCODING: &[i32] = &[16, 17, 18, 19, 20, 21, 22, 23, 24, 25];
const HEX_ENCODING: &[i32] = &[
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 33, 34, 35, 36, 37, 38,
];
const BINARY_ENCODING: &[i32] = &[16, 17];
const DNA_ENCODING: &[i32] = &[33, 35, 39, 52];

/// A list of viewer rotations that look nice. Every now and then it switches
/// to a new one, but it only uses the first at startup, and the last few are
/// repeats so that straight ahead comes up more often.
const NICE_VIEWS: [[f32; 2]; 16] = [
    [0.0, 0.0],
    [0.0, -20.0],
    [0.0, 20.0],
    [25.0, 0.0],
    [-25.0, 0.0],
    [25.0, 20.0],
    [-25.0, 20.0],
    [25.0, -20.0],
    [-25.0, -20.0],
    [10.0, 0.0],
    [-10.0, 0.0],
    [0.0, 0.0],
    [0.0, 0.0],
    [0.0, 0.0],
    [0.0, 0.0],
    [0.0, 0.0],
];

/// Three uniform numbers averaged: the middle of the range far more often
/// than either end.
fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

struct Strip {
    x: f32,
    y: f32,
    z: f32,
    dz: f32,
    /// Whether this strip is on its way out.
    erasing: bool,
    /// The bottommost glyph, which feeds the others, and where it has got to.
    spinner_glyph: i32,
    spinner_y: f32,
    spinner_speed: f32,
    /// The glyphs the spinner reveals as it passes. Zero is none and a
    /// negative one is still spinning.
    glyphs: [i32; GRID_SIZE],
    highlight: [bool; GRID_SIZE],
    /// Rotate every spinner every this many frames.
    spin_speed: i32,
    spin_tick: i32,
    /// Waves of brightness wash down a strip every this many frames.
    wave_position: usize,
    wave_speed: i32,
    wave_tick: i32,
}

impl Default for Strip {
    fn default() -> Self {
        Strip {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            dz: 0.0,
            erasing: false,
            spinner_glyph: 0,
            spinner_y: 0.0,
            spinner_speed: 0.0,
            glyphs: [0; GRID_SIZE],
            highlight: [false; GRID_SIZE],
            spin_speed: 0,
            spin_tick: 0,
            wave_position: 0,
            wave_speed: 0,
            wave_tick: 0,
        }
    }
}

struct GlMatrix {
    strips: Vec<Strip>,
    glyph_map: &'static [i32],
    texture: u32,
    tex_char_width: f32,
    tex_char_height: f32,
    brightness_ramp: [f32; WAVE_SIZE],
    /// Where the camera is looking and where it is heading.
    last_view: usize,
    target_view: usize,
    view_x: f32,
    view_y: f32,
    view_steps: i32,
    view_tick: i32,
    auto_tracking: bool,
    track_tick: i32,
    aspect: f32,
    speed: f32,
    fog: bool,
    waves: bool,
    rotate: bool,
    texture_p: bool,
    wire: bool,
}

impl GlMatrix {
    /// Re-randomize one strip: a new place in the arena, a new speed, and a
    /// new set of glyphs, most cells empty and a few of them spinning.
    fn reset_strip(&self, s: &mut Strip) {
        *s = Strip::default();
        s.x = frand(GRID_SIZE as f64) as f32 - (GRID_SIZE / 2) as f32;
        // Shift the top slightly, so they do not all start level.
        s.y = (GRID_SIZE / 2) as f32 + bellrand(0.5);
        s.z = GRID_DEPTH * 0.2 - frand((GRID_DEPTH * 0.7) as f64) as f32;
        s.dz = bellrand(0.02) * self.speed;
        s.spinner_speed = bellrand(0.3) * self.speed;
        s.spin_speed = bellrand(2.0 / self.speed as f64) as i32 + 1;
        s.wave_speed = bellrand(3.0 / self.speed as f64) as i32 + 1;

        for i in 0..GRID_SIZE {
            let draw = random() % 7;
            let spin = draw != 0 && random().is_multiple_of(20);
            let mut g = if draw != 0 {
                self.glyph_map[random() as usize % self.glyph_map.len()] + 1
            } else {
                0
            };
            if spin {
                g = -g;
            }
            s.glyphs[i] = g;
            s.highlight[i] = false;
        }
        s.spinner_glyph = -(self.glyph_map[random() as usize % self.glyph_map.len()] + 1);
    }

    /// One step of a strip. When the falling glyph reaches the bottom it goes
    /// back to the top and erases instead, more slowly than it drew.
    fn tick_strip(&self, s: &mut Strip) {
        s.z += s.dz;
        if s.z > GRID_DEPTH * SPLASH_RATIO {
            // Splashed into the screen.
            self.reset_strip(s);
            return;
        }

        s.spinner_y += s.spinner_speed;
        if s.spinner_y >= GRID_SIZE as f32 {
            if s.erasing {
                self.reset_strip(s);
                return;
            }
            s.erasing = true;
            s.spinner_y = 0.0;
            s.spinner_speed /= 2.0;
        }

        s.spin_tick += 1;
        if s.spin_tick > s.spin_speed {
            s.spin_tick = 0;
            s.spinner_glyph = -(self.glyph_map[random() as usize % self.glyph_map.len()] + 1);
            for i in 0..GRID_SIZE {
                if s.glyphs[i] < 0 {
                    s.glyphs[i] = -(self.glyph_map[random() as usize % self.glyph_map.len()] + 1);
                    if random().is_multiple_of(800) {
                        // Sometimes they stop spinning.
                        s.glyphs[i] = -s.glyphs[i];
                    }
                }
            }
        }

        s.wave_tick += 1;
        if s.wave_tick > s.wave_speed {
            s.wave_tick = 0;
            s.wave_position += 1;
            if s.wave_position >= WAVE_SIZE {
                s.wave_position = 0;
            }
        }
    }

    /// One character, at a place and a brightness.
    #[allow(clippy::too_many_arguments)]
    fn draw_glyph(
        &self,
        g: &mut crate::runtime::gl::Glx,
        glyph: i32,
        highlight: bool,
        x: f32,
        y: f32,
        z: f32,
        brightness: f32,
    ) {
        let spinner = glyph < 0;
        let glyph = glyph.abs();
        let mut brightness = brightness;
        let w = self.tex_char_width;
        let h = self.tex_char_height;
        let (mut cx, mut cy) = (0.0, 0.0);
        let mut s = 1.0;

        if spinner {
            brightness *= 1.5;
        }

        if !self.texture_p {
            s = 0.8;
        } else {
            let ccx = ((glyph - 1) as usize) % CHAR_COLS;
            let ccy = ((glyph - 1) as usize) / CHAR_COLS;
            cx = ccx as f32 * w;
            // The rows of the picture run down from the top here, where
            // upstream's texture has them running up from the bottom.
            cy = ccy as f32 * h;

            if self.fog {
                // How far back it is, scaled so that no row goes all black.
                let depth = (z / GRID_DEPTH) + 0.5;
                brightness *= 0.2 + (depth * 0.8);
            }
        }
        let (x, y) = if self.texture_p {
            (x, y)
        } else {
            (x + 0.1, y + 0.1)
        };

        if highlight {
            brightness *= 2.0;
        }
        let (r, gg, b) = if !self.texture_p && !spinner {
            (0.0, 1.0, 0.0)
        } else {
            (1.0, 1.0, 1.0)
        };
        let mut a = brightness;

        // A glyph very close to the glass is about to splash into it and
        // vanish, so fade it out as it comes.
        if z > GRID_DEPTH / 2.0 {
            let ratio = (z - GRID_DEPTH / 2.0) / ((GRID_DEPTH * SPLASH_RATIO) - GRID_DEPTH / 2.0);
            let i = ((ratio * WAVE_SIZE as f32) as usize).min(WAVE_SIZE - 1);
            a *= self.brightness_ramp[i];
        }
        g.color4f(r, gg, b, a);

        g.begin(if self.wire {
            Shape::LineLoop
        } else {
            Shape::Quads
        });
        g.normal3f(0.0, 0.0, 1.0);
        g.tex_coord2f(cx, cy + h);
        g.vertex3f(x, y, z);
        g.tex_coord2f(cx + w, cy + h);
        g.vertex3f(x + s, y, z);
        g.tex_coord2f(cx + w, cy);
        g.vertex3f(x + s, y + s, z);
        g.tex_coord2f(cx, cy);
        g.vertex3f(x, y + s, z);
        g.end();

        if self.wire && spinner {
            g.begin(Shape::Lines);
            g.vertex3f(x, y, z);
            g.vertex3f(x + s, y + s, z);
            g.vertex3f(x, y + s, z);
            g.vertex3f(x + s, y, z);
            g.end();
        }
    }

    /// Every glyph of a strip that the spinner has already passed, and the
    /// spinner itself.
    fn draw_strip(&self, g: &mut crate::runtime::gl::Glx, s: &Strip) {
        for i in 0..GRID_SIZE {
            let glyph = s.glyphs[i];
            let mut below = s.spinner_y >= i as f32;
            if s.erasing {
                below = !below;
            }
            if glyph != 0 && below {
                let brightness = if !self.waves {
                    1.0
                } else {
                    let j = WAVE_SIZE - ((i + (GRID_SIZE - s.wave_position)) % WAVE_SIZE);
                    self.brightness_ramp[j % WAVE_SIZE]
                };
                self.draw_glyph(
                    g,
                    glyph,
                    s.highlight[i],
                    s.x,
                    s.y - i as f32,
                    s.z,
                    brightness,
                );
            }
        }
        if !s.erasing {
            self.draw_glyph(g, s.spinner_glyph, false, s.x, s.y - s.spinner_y, s.z, 1.0);
        }
    }

    /// Every now and then, glide the camera to another of the nice views.
    fn auto_track(&mut self) {
        if !self.rotate {
            return;
        }
        if !self.auto_tracking {
            self.track_tick += 1;
            if (self.track_tick as f32) < 20.0 / self.speed {
                return;
            }
            self.track_tick = 0;
            if random().is_multiple_of(20) {
                self.auto_tracking = true;
            } else {
                return;
            }
        }

        let [ox, oy] = NICE_VIEWS[self.last_view];
        let [tx, ty] = NICE_VIEWS[self.target_view];
        // Sinusoidal steps, so that it does not jerk to a stop.
        let th =
            ((std::f32::consts::PI / 2.0) * self.view_tick as f32 / self.view_steps as f32).sin();
        self.view_x = ox + ((tx - ox) * th);
        self.view_y = oy + ((ty - oy) * th);
        self.view_tick += 1;

        if self.view_tick >= self.view_steps {
            self.view_tick = 0;
            self.view_steps = (350.0 / self.speed) as i32;
            self.last_view = self.target_view;
            self.target_view = random() as usize % (NICE_VIEWS.len() - 1) + 1;
            self.auto_tracking = false;
        }
    }
}

/// Upstream's `spank_image`: the glyph sheet is sixteen by thirteen
/// characters, which is 512 by 598 pixels and does not fit a power-of-two
/// texture. Two rows nobody uses are dropped and the katakana row at the
/// bottom is moved up into their place, which brings it to 512 by 506, and
/// then it is padded to 512 square. The numbering of the glyphs assumes all
/// of this.
fn spank_image(px: &[u8], w: usize, h: usize) -> (usize, usize, Vec<u8>) {
    let ch = h / CHAR_ROWS;
    let rows: Vec<usize> = (0..10).chain(std::iter::once(12)).collect();
    let mut out = vec![0u8; w * 512 * 4];
    for (to, from) in rows.iter().enumerate() {
        let src = from * ch * w * 4;
        let dst = to * ch * w * 4;
        out[dst..dst + ch * w * 4].copy_from_slice(&px[src..src + ch * w * 4]);
    }
    (w, 512, out)
}

/// Mirror every character in its own cell. Upstream does it to the bits
/// rather than to the texture coordinates, because on some machines that was
/// much faster.
fn flip_chars(px: &mut [u8], w: usize, h: usize) {
    let cw = w / CHAR_COLS;
    for y in 0..h {
        for col in 0..CHAR_COLS {
            let xx = col * cw;
            for x in 0..cw / 2 {
                let a = (y * w + xx + x) * 4;
                let b = (y * w + xx + cw - x - 1) * 4;
                for k in 0..4 {
                    px.swap(a + k, b + k);
                }
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let texture_p = g.res.bool("texture") && !wire;
    let speed = (g.res.float("speed") as f32).max(0.01);

    let mode = g.res.string("mode").to_ascii_lowercase();
    let (glyph_map, flip): (&'static [i32], bool) = match mode.as_str() {
        "dna" => (DNA_ENCODING, false),
        "bin" | "binary" => (BINARY_ENCODING, false),
        "hex" | "hexadecimal" => (HEX_ENCODING, false),
        "dec" | "decimal" => (DECIMAL_ENCODING, false),
        _ => (MATRIX_ENCODING, true),
    };

    let mut texture = 0;
    let (mut tex_char_width, mut tex_char_height) = (1.0, 1.0);
    if texture_p {
        texture = g.glx.gen_texture();
        g.glx.bind_texture(texture);
        match crate::runtime::png::decode_rgba(crate::images::MATRIX3) {
            Some((w, h, mut px)) => {
                let (w, h) = (w as usize, h as usize);
                if flip {
                    flip_chars(&mut px, w, h);
                }
                // The green channel becomes the alpha and the green goes to
                // full, so a glyph is a shape rather than a picture.
                for p in px.chunks_exact_mut(4) {
                    p[3] = p[1];
                    p[1] = 0xFF;
                }
                let (tw, th, out) = spank_image(&px, w, h);
                tex_char_width = (w / CHAR_COLS) as f32 / tw as f32;
                tex_char_height = (h / CHAR_ROWS) as f32 / th as f32;
                g.glx.tex_image_2d(tw as i32, th as i32, out);
            }
            None => g.glx.tex_image_2d(1, 1, vec![255, 255, 255, 255]),
        }
        // Upstream: I'd expect clamping to be the thing to do here, but oddly
        // we get a faint solid green border around the texture if it is not
        // repeated.
        g.glx.tex_clamp(false);
    }

    // Scaling coverage-percent to strips: this number looks about right.
    let nstrips = ((g.res.float("density") * 2.2) as usize).clamp(1, 2000);

    let mut ramp = [0.0f32; WAVE_SIZE];
    for (i, r) in ramp.iter_mut().enumerate() {
        let j = (WAVE_SIZE - i) as f32 / (WAVE_SIZE - 1) as f32;
        *r = 0.2 + (j * std::f32::consts::FRAC_PI_2).sin() * 0.8;
    }

    let mut this = GlMatrix {
        strips: Vec::new(),
        glyph_map,
        texture,
        tex_char_width,
        tex_char_height,
        brightness_ramp: ramp,
        last_view: 0,
        target_view: 0,
        view_x: NICE_VIEWS[0][0],
        view_y: NICE_VIEWS[0][1],
        view_steps: 100,
        view_tick: 0,
        auto_tracking: false,
        track_tick: 0,
        aspect: 1.0,
        speed,
        fog: g.res.bool("fog"),
        waves: g.res.bool("waves"),
        rotate: g.res.bool("rotate"),
        texture_p,
        wire,
    };

    for _ in 0..nstrips {
        let mut s = Strip::default();
        this.reset_strip(&mut s);
        // Starting every strip at once makes the first few seconds much
        // denser than normal, so they all start in erase mode at random
        // heights with nothing on them yet. As they die off and are re-made
        // the density settles.
        s.erasing = true;
        s.spinner_y = frand(GRID_SIZE as f64) as f32;
        s.glyphs = [0; GRID_SIZE];
        this.strips.push(s);
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for GlMatrix {
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
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(80.0, self.aspect, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 25.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.clear();
        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.lighting(false);
        if self.texture_p {
            g.glx.texturing(true);
            g.glx.bind_texture(self.texture);
            // Jeff Epler's suggestion: adding rather than blending means a
            // bright glyph with a darker one in front of it comes out a
            // little brighter than the bright glyph alone.
            g.glx.blend(Blend::AlphaAdd);
        }

        g.glx.push_matrix();
        if self.rotate {
            g.glx.rotate(self.view_x, 1.0, 0.0, 0.0);
            g.glx.rotate(self.view_y, 0.0, 1.0, 0.0);
        }

        // Back to front, so that the transparency comes out right.
        let mut order: Vec<usize> = (0..self.strips.len()).collect();
        order.sort_by(|&a, &b| {
            let (a, b) = (self.strips[a].z, self.strips[b].z);
            a.total_cmp(&b)
        });

        for i in order {
            let mut s = std::mem::take(&mut self.strips[i]);
            self.tick_strip(&mut s);
            let glx = &mut g.glx;
            self.draw_strip(glx, &s);
            self.strips[i] = s;
        }

        self.auto_track();

        g.glx.pop_matrix();
        g.glx.texturing(false);
        g.glx.blend(Blend::Off);
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:      30000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*speed:      1.0",
    "*density:    20",
    "*mode:       matrix",
    "*fog:        True",
    "*waves:      True",
    "*rotate:     True",
    "*texture:    True",
];

const MODES: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "matrix",
        label: "Matrix encoding",
    },
    crate::runtime::opts::SelectItem {
        value: "binary",
        label: "Binary encoding",
    },
    crate::runtime::opts::SelectItem {
        value: "hex",
        label: "Hexadecimal encoding",
    },
    crate::runtime::opts::SelectItem {
        value: "dna",
        label: "Genetic encoding",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("density", "Glyph density", 0.0, 100.0, 1.0, 0, "20"),
    Opt::slider("speed", "Glyph speed", 0.1, 8.0, 0.1, 1, "1.0"),
    Opt::select("mode", "Encoding", MODES, "matrix"),
    Opt::boolean("fog", "Fog", "true"),
    Opt::boolean("waves", "Waves", "true"),
    Opt::boolean("rotate", "Panning", "true"),
    Opt::boolean("texture", "Textured", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glmatrix",
    label: "GL Matrix",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=_dktSpsaCPg"),
        blurb: "The 3D digital rain effect, as seen in the title sequence of \
                The Matrix. See also xmatrix for the 2D version that appeared \
                on the computer monitors in the film.",
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

    /// The glyph sheet is rearranged before it is uploaded: eleven rows of a
    /// thirteen row picture, with the katakana row moved up to the end, padded
    /// to a square texture.
    #[test]
    fn the_glyph_sheet_is_rearranged() {
        let (w, h, px) = crate::runtime::png::decode_rgba(crate::images::MATRIX3)
            .expect("the glyph sheet does not decode");
        assert_eq!((w, h), (512, 598));
        let (tw, th, out) = spank_image(&px, w as usize, h as usize);
        assert_eq!((tw, th), (512, 512));
        let ch = 598 / CHAR_ROWS;
        // Row ten of the new picture is row twelve of the old one.
        let a = 10 * ch * 512 * 4;
        let b = 12 * ch * 512 * 4;
        assert_eq!(
            out[a..a + 4096],
            px[b..b + 4096],
            "the katakana moved wrong"
        );
        // And the six rows past the picture are empty.
        assert!(out[506 * 512 * 4..].iter().all(|&b| b == 0));
    }

    /// A strip's spinner falls down it, revealing the glyphs behind it, and
    /// then falls again erasing them.
    #[test]
    fn the_spinner_reveals_and_then_erases() {
        let mut r = start(StartArgs::new(640, 480, "density=100", 20260811));
        let mut seen = 0;
        for _ in 0..300 {
            r.step();
            seen = seen.max(r.frame().vertices.len());
        }
        assert!(seen > 400, "only {seen} vertices: nothing is falling");
        let f = r.frame();
        assert!(
            f.batches.iter().all(|b| !b.depth_test),
            "the glyphs are depth tested"
        );
        assert!(
            f.batches
                .iter()
                .all(|b| b.blend == crate::runtime::gl::Blend::AlphaAdd),
            "the glyphs are not drawn additively"
        );
    }

    /// Every glyph a mode draws is inside the sheet, and the matrix mode's
    /// katakana land on the row the rearrangement moved.
    #[test]
    fn every_glyph_is_on_the_sheet() {
        for map in [
            MATRIX_ENCODING,
            DECIMAL_ENCODING,
            HEX_ENCODING,
            BINARY_ENCODING,
            DNA_ENCODING,
        ] {
            for &g in map {
                let row = g as usize / CHAR_COLS;
                assert!(row < CHAR_ROWS - 2, "glyph {g} is on a row that was cut");
            }
        }
        assert!(MATRIX_ENCODING[10..].iter().all(|&g| g / 16 == 10));
    }

    /// The camera glides between the nice views rather than jumping, and
    /// stays among them.
    #[test]
    fn the_camera_glides_between_views() {
        let mut r = start(StartArgs::new(640, 480, "density=1", 20260811));
        let mut xs: Vec<f32> = Vec::new();
        for _ in 0..2000 {
            r.step();
            let f = r.frame();
            if let Some(b) = f.batches.first() {
                xs.push(b.modelview.0[0]);
            }
        }
        let lo = xs.iter().copied().fold(f32::MAX, f32::min);
        let hi = xs.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 0.0001, "the camera never moved");
        // And every step of it is small.
        let step = xs
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(step < 0.05, "the camera jumped by {step}");
    }
}
