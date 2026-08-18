//! Port of `hacks/glx/splitflap.c`.
//!
//! ```text
//! splitflap, Copyright (c) 2015-2018 Jamie Zawinski <jwz@jwz.org>
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
//! A split-flap display, the kind that used to hang in railway stations. Each
//! cell is a spool of hinged fins; to show a letter the spool turns until that
//! letter is at the front, and every fin on the way flops over the top with
//! the top half of one character on its front and the bottom half of the next
//! on its back.
//!
//! One cell in ten is missing a fin, so it skips a character; one in twenty is
//! sticky and does not quite fall the whole way. Both are upstream's, and both
//! are what makes it look like a real board rather than an animation.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::rotator::Rotator;
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

const QUARTER_FRAME: usize = 0;
const DISC_QUARTER: usize = 1;
const FIN_EDGE_HALF: usize = 2;
const FIN_FACE_HALF: usize = 3;
const OUTER_FRAME: usize = 4;

/// How wide the gap that a clock's colon sits in is.
const COLON_WIDTH: f32 = 0.5;

/// The four modelled pieces. The case round the outside is drawn rather than
/// modelled, since its size depends on the grid.
const MODELS: [&str; 4] = [
    crate::models::SPLITFLAP_OBJ_BOX_QUARTER_FRAME,
    crate::models::SPLITFLAP_OBJ_DISC_QUARTER,
    crate::models::SPLITFLAP_OBJ_FIN_EDGE_HALF,
    crate::models::SPLITFLAP_OBJ_FIN_FACE_HALF,
];

/// Every character a text board can show. Upstream also carries a Latin-1
/// spool and does not use it: "If we include these, the flappers just take too
/// long. It's boring."
const ASCII_SPOOL: [&str; 69] = [
    " ", "!", "\"", "#", "$", "%", "&", "'", "(", ")", "*", "+", ",", "-", ".", "/", "0", "1", "2",
    "3", "4", "5", "6", "7", "8", "9", ":", ";", "<", "=", ">", "?", "@", "A", "B", "C", "D", "E",
    "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X",
    "Y", "Z", "[", "\\", "]", "^", "_", "`", "{", "|", "}", "~",
];

const DIGIT_S1_SPOOL: [&str; 2] = [" ", "1"];
const DIGIT_01_SPOOL: [&str; 2] = ["0", "1"];
const AP_SPOOL: [&str; 2] = ["A", "P"];
const M_SPOOL: [&str; 1] = ["M"];
const DIGIT_05_SPOOL: [&str; 6] = ["0", "1", "2", "3", "4", "5"];
const DIGIT_SPOOL: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// One cell of the board.
struct Flapper {
    /// The character wanted, and the one on show, which is fractional while
    /// the spool is turning.
    target_index: usize,
    current_index: f64,
    /// How far short of the bottom this cell's fin stops, if it sticks.
    sticky: f32,
    /// Which fin has snapped off, or none.
    missing: Option<usize>,
    spool: &'static [&'static str],
}

/// One character rendered into a texture of its own, and how big it came out.
struct TexInfo {
    text: &'static str,
    texid: u32,
    width: i32,
    ascent: i32,
    descent: i32,
    tex_width: i32,
    tex_height: i32,
}

struct Splitflap {
    rot: Rotator,
    rot2: Option<Rotator>,
    trackball: Trackball,
    spin: [bool; 3],
    face_front: bool,
    texinfo: Vec<TexInfo>,
    dlists: Vec<u32>,
    component_colors: Vec<[f32; 4]>,
    text_color: [f32; 4],
    flappers: Vec<Flapper>,
    font: TexFont,
    ascent: i32,
    descent: i32,
    /// Zero for a text board, or twelve or twenty-four for a clock.
    clock: i32,
    linger: i32,
    first_time: bool,
    grid_width: usize,
    grid_height: usize,
    speed: f32,
    wire: bool,
    aspect: f32,
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// Works with negatives, as upstream's `MOD` does.
fn modulo(m: i64, n: i64) -> usize {
    ((m + n) % n).max(0) as usize
}

impl Splitflap {
    fn draw_component(&self, g: &mut Gl, i: usize) {
        g.glx.material_ambient_diffuse(self.component_colors[i]);
        // A display list here replays geometry and not state, so the shine
        // every piece shares goes on where it is called.
        g.glx.material_specular([0.4, 0.4, 0.4, 1.0]);
        g.glx.material_shininess(80.0);
        g.glx.call_list(self.dlists[i]);
    }

    /// `draw_frame`: the box round one cell and the disc it turns on, drawn
    /// four times over as mirror images of a quarter.
    fn draw_frame(&self, g: &mut Gl) {
        g.glx.push_matrix();
        for (sx, sy, cw) in [
            (1.0, 1.0, false),
            (-1.0, 1.0, true),
            (1.0, -1.0, false),
            (-1.0, 1.0, true),
        ] {
            g.glx.scale(sx, sy, 1.0);
            g.glx.front_face_cw(cw);
            self.draw_component(g, QUARTER_FRAME);
            self.draw_component(g, DISC_QUARTER);
        }
        g.glx.pop_matrix();
    }

    /// `draw_fin_text_quad`: half a character on the face of one fin, cut off
    /// just short of the gutter.
    fn draw_fin_text_quad(&self, g: &mut Gl, f: &Flapper, index: usize, top: bool) {
        // Lifted off the surface, kept clear of the gutter, and scaled to
        // fill the panel.
        let z = 0.035;
        let mut bot = 0.013f32;
        let mut scale = 1.8f32;

        let lh = (self.ascent + self.descent) as f32;
        let Some(ti) = self
            .texinfo
            .iter()
            .find(|t| t.text == f.spool[index.min(f.spool.len() - 1)])
        else {
            return;
        };

        if self.ascent < ti.ascent {
            // "WTF! &Aacute; has a higher ascent than the font itself!"
            scale *= self.ascent as f32 / ti.ascent as f32 * 0.98;
        }

        g.glx.push_matrix();
        g.glx.normal3f(0.0, 0.0, 1.0);
        g.glx.front_face_cw(!top);

        if !self.wire {
            g.glx.bind_texture(ti.texid);
            g.glx.texturing(true);
            g.glx.blend(Blend::Alpha);
            g.glx.lighting(false);
        }

        g.glx.translate(0.0, 0.0, z);
        g.glx.scale(1.0 / lh, 1.0 / lh, 1.0);
        g.glx.scale(scale, scale, 1.0);
        if !top {
            g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        }

        // Upstream places the character by its left and right bearings; the
        // metrics here carry the advance, which for a single glyph is the
        // same span.
        let w = ti.width as f32;
        let mut qx0 = -w / 2.0;
        let mut qx1 = w / 2.0;
        let mut qy0 = -ti.descent as f32;
        let mut qy1 = ti.ascent as f32;
        qy0 += self.descent as f32;
        qy1 += self.descent as f32;
        qy0 -= lh / 2.0;
        qy1 -= lh / 2.0;

        // Move the descenders down a bit, if there is room.
        let off = (self.descent as f32 / 3.0)
            .min(self.descent as f32 - self.descent as f32 / 3.0 - ti.descent as f32)
            .max(0.0);
        qy0 -= off;
        qy1 -= off;

        let mut tx0 = 0.0f32;
        let mut tx1 = w / ti.tex_width as f32;
        let mut ty1 = 0.0f32;
        let mut ty0 = (ti.ascent + ti.descent) as f32 / ti.tex_height as f32;

        // The bottom panel shows the character the other way up.
        if !top {
            std::mem::swap(&mut tx0, &mut tx1);
        }

        // Cut the character in half, truncating just above the split line.
        let (oqy0, oqy1) = (qy0, qy1);
        bot *= lh * scale;
        if top {
            if qy0 < bot {
                qy0 = bot;
            }
        } else if qy1 > -bot {
            qy1 = -bot;
        }
        let r0 = (qy0 - oqy0) / (oqy1 - oqy0);
        let r1 = (qy1 - oqy1) / (oqy1 - oqy0);
        ty0 -= r0 * (ty0 - ty1);
        ty1 -= r1 * (ty0 - ty1);

        let c = self.text_color;
        g.glx.color4f(c[0], c[1], c[2], c[3]);
        g.glx.begin(if self.wire {
            Shape::LineLoop
        } else {
            Shape::Quads
        });
        for (u, v, x, y) in [
            (tx0, ty0, qx0, qy0),
            (tx1, ty0, qx1, qy0),
            (tx1, ty1, qx1, qy1),
            (tx0, ty1, qx0, qy1),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, 0.0);
        }
        g.glx.end();
        let _ = (&mut qx0, &mut qx1);
        g.glx.pop_matrix();

        if !self.wire {
            g.glx.blend(Blend::Off);
            g.glx.lighting(true);
            g.glx.texturing(false);
        }
    }

    /// `draw_fin`: one hinged flap, with a character on the front, the back,
    /// or neither.
    fn draw_fin(
        &self,
        g: &mut Gl,
        f: &Flapper,
        front: Option<usize>,
        back: Option<usize>,
        text: bool,
    ) {
        g.glx.push_matrix();
        g.glx.front_face_cw(false);

        if !text {
            self.draw_component(g, FIN_EDGE_HALF);
        }
        if let Some(front) = front {
            if text {
                self.draw_fin_text_quad(g, f, front, true);
            } else if !self.wire {
                self.draw_component(g, FIN_FACE_HALF);
            }
        }

        g.glx.scale(-1.0, 1.0, 1.0);
        if !text {
            g.glx.front_face_cw(true);
            self.draw_component(g, FIN_EDGE_HALF);
            if front.is_some() && !self.wire {
                self.draw_component(g, FIN_FACE_HALF);
            }
        }

        if let Some(back) = back {
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            if text {
                self.draw_fin_text_quad(g, f, back, false);
            } else if !self.wire {
                self.draw_component(g, FIN_FACE_HALF);
                g.glx.scale(-1.0, 1.0, 1.0);
                g.glx.front_face_cw(false);
                self.draw_component(g, FIN_FACE_HALF);
            }
        }
        g.glx.pop_matrix();
    }

    /// `draw_outer_frame`: the case the grid of cells sits in.
    fn draw_outer_frame(&self, g: &mut Gl) {
        if self.wire {
            return;
        }
        let mut w = self.grid_width as f32;
        let mut h = self.grid_height as f32;
        let d = 1.0f32;
        if self.clock == 12 {
            w += COLON_WIDTH * 3.0;
        } else if self.clock == 24 {
            w += COLON_WIDTH * 2.0;
        }
        w += 0.2;
        h += 0.2;
        if self.clock != 0 {
            w += 0.25;
        }
        if w > 3.0 {
            w += 0.5;
        }
        if h > 3.0 {
            h += 0.5;
        }

        g.glx.front_face_cw(false);
        g.glx.push_matrix();
        g.glx.translate(0.0, 1.03, 0.0);
        g.glx.begin(Shape::Quads);
        for (n, quad) in [
            (
                [0.0, 1.0, 0.0],
                [[-w, d, h], [w, d, h], [w, d, -h], [-w, d, -h]],
            ),
            (
                [0.0, -1.0, 0.0],
                [[-w, -d, -h], [w, -d, -h], [w, -d, h], [-w, -d, h]],
            ),
            (
                [0.0, 0.0, 1.0],
                [[-w, -d, h], [w, -d, h], [w, d, h], [-w, d, h]],
            ),
            (
                [0.0, 0.0, -1.0],
                [[-w, d, -h], [w, d, -h], [w, -d, -h], [-w, -d, -h]],
            ),
            (
                [1.0, 0.0, 0.0],
                [[w, -d, h], [w, -d, -h], [w, d, -h], [w, d, h]],
            ),
            (
                [-1.0, 0.0, 0.0],
                [[-w, -d, -h], [-w, -d, h], [-w, d, h], [-w, d, -h]],
            ),
        ] {
            g.glx.normal3f(n[0], n[1], n[2]);
            for v in quad {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
        }
        g.glx.end();
        g.glx.pop_matrix();
    }

    /// `tick_flapper`: turn the crank one step towards the wanted character.
    fn tick_flapper(&mut self, i: usize) {
        let speed = self.speed;
        let f = &mut self.flappers[i];
        let prev = f.current_index;
        if f.current_index == f.target_index as f64 {
            return;
        }
        f.current_index += f64::from(speed) * 0.35;
        let mut wrapped = false;
        while f.current_index > f.spool.len() as f64 {
            f.current_index -= f.spool.len() as f64;
            wrapped = true;
        }
        // Just overshot.
        if (prev < f.target_index as f64 || wrapped) && f.current_index > f.target_index as f64 {
            f.current_index = f.target_index as f64;
        }
    }

    /// `draw_flapper`: the cell, the flap resting at the top, the flap resting
    /// at the bottom, and the one in between if it is falling.
    fn draw_flapper(&self, g: &mut Gl, i: usize, text: bool) {
        let f = &self.flappers[i];
        let n = f.spool.len() as i64;
        let prev_index = f.current_index.floor() as i64;
        let mut next_index = modulo(prev_index + 1, n);
        let epsilon = 0.02;
        let mut r = (f.current_index - prev_index as f64) as f32;
        let mut moving = r > 0.0 && r < 1.0;
        let mut sticky = f.sticky;

        if f.missing.is_some() {
            sticky = 0.0;
        }
        if let Some(missing) = f.missing
            && modulo(prev_index, n) == missing
        {
            moving = false;
            sticky = 0.0;
        }
        if !moving {
            next_index = modulo(prev_index, n);
        }

        if !text {
            self.draw_frame(g);
        }

        // The flap lying flat at the top, showing the top half of the next
        // character.
        if !moving || !text || r > epsilon {
            let mut p2 = next_index;
            if Some(p2) == f.missing {
                p2 = modulo(p2 as i64 + 1, n);
            }
            self.draw_fin(g, f, Some(p2), None, text);
        }

        // And the one lying flat at the bottom, showing the bottom half of
        // the one before.
        if !moving || !text || r < 1.0 - epsilon {
            let mut p2 = modulo(prev_index, n);
            if !moving && sticky > 0.0 {
                p2 = modulo(p2 as i64 - 1, n);
            }
            if let Some(missing) = f.missing
                && p2 == modulo(missing as i64 + 1, n)
            {
                p2 = modulo(p2 as i64 - 1, n);
            }
            g.glx.push_matrix();
            g.glx.rotate(180.0, 1.0, 0.0, 0.0);
            self.draw_fin(g, f, None, Some(p2), text);
            g.glx.pop_matrix();
        }

        // And the one in the air, top half of the old on its front and bottom
        // half of the new on its back.
        if moving || sticky > 0.0 {
            if !moving {
                r = 1.0;
            }
            if sticky > 0.0 && r > 1.0 - sticky {
                r = 1.0 - sticky;
            }
            g.glx.push_matrix();
            g.glx.rotate(r * 180.0, 1.0, 0.0, 0.0);
            self.draw_fin(g, f, Some(modulo(prev_index, n)), Some(next_index), text);
            g.glx.pop_matrix();
        }
    }

    /// `draw_colon`: the colon between a clock's pairs of digits, drawn five
    /// times over to give it a border.
    fn draw_colon(&self, g: &mut Gl) {
        let m = self.font.metrics(":");
        let s = 2.0 / (self.ascent + self.descent) as f32;
        let z = 0.01;

        g.glx.push_matrix();
        g.glx.translate(-(1.0 + COLON_WIDTH), 0.0, 0.0);
        g.glx.scale(s, s, 1.0);
        g.glx.translate(
            -(m.width as f32) / 2.0,
            -((m.ascent + m.descent) as f32) / 2.0,
            0.0,
        );
        g.glx.blend(Blend::Alpha);
        g.glx.lighting(false);
        let n = 1.5;
        for (dx, dy) in [
            (-1.0, -1.0),
            (-1.0, 1.0),
            (1.0, 1.0),
            (1.0, -1.0),
            (0.0, 0.0),
        ] {
            g.glx.push_matrix();
            if dx == 0.0 && dy == 0.0 {
                let c = self.text_color;
                g.glx.color4f(c[0], c[1], c[2], c[3]);
                g.glx.translate(0.0, 0.0, z * 2.0);
            } else {
                g.glx.color4f(0.0, 0.0, 0.0, 1.0);
            }
            g.glx.translate(n * dx, n * dy, 0.0);
            self.font.print_string(&mut g.glx, ":");
            g.glx.pop_matrix();
        }
        g.glx.lighting(true);
        g.glx.blend(Blend::Off);
        g.glx.pop_matrix();
    }

    /// Where a character sits on this cell's spool, or the first entry.
    fn find_index(f: &Flapper, c: char) -> usize {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        f.spool.iter().position(|&t| t == s).unwrap_or(0)
    }

    /// `fill_targets`: read the next screenful of text, or the time.
    fn fill_targets(&mut self, g: &mut Gl) {
        if self.clock != 0 {
            let secs = g.wall_clock();
            let h24 = (secs / 3600.0) as i32 % 24;
            let mi = (secs / 60.0) as i32 % 60;
            let se = secs as i32 % 60;
            let text: String = if self.clock == 24 {
                format!("{h24:02}{mi:02}{se:02}")
            } else {
                let pm = h24 >= 12;
                let mut h = h24 % 12;
                if h == 0 {
                    h = 12;
                }
                // A leading zero on the hour reads as a blank.
                let hs = if h < 10 {
                    format!(" {h}")
                } else {
                    format!("{h}")
                };
                format!("{hs}{mi:02}{se:02}{}M", if pm { 'P' } else { 'A' })
            };
            let chars: Vec<char> = text.chars().collect();
            for i in 0..self.flappers.len() {
                let c = chars.get(i).copied().unwrap_or(' ');
                self.flappers[i].target_index = Self::find_index(&self.flappers[i], c);
            }
            return;
        }

        for y in 0..self.grid_height {
            let mut nl = false;
            for x in 0..self.grid_width {
                let i = y * self.grid_width + x;
                let mut c = if nl {
                    ' '
                } else {
                    char::from(g.text_getc().unwrap_or(b' '))
                };
                if c == '\r' || c == '\n' {
                    nl = true;
                    c = ' ';
                }
                // Anything outside ASCII has no fin to show it on: upstream
                // folds Latin-1 down to its nearest ASCII and this folds the
                // rest to a space.
                if !c.is_ascii() {
                    c = ' ';
                }
                // "Upcase ASCII. Upcasing Unicrud would be rocket surgery."
                let c = c.to_ascii_uppercase();
                self.flappers[i].target_index = Self::find_index(&self.flappers[i], c);
                self.flappers[i].sticky = if random().is_multiple_of(20) {
                    0.05 + frand(0.1) as f32 + frand(0.1) as f32
                } else {
                    0.0
                };
            }
        }
    }

    fn draw_flappers(&mut self, g: &mut Gl, text: bool) {
        let mut running = 0;
        for y in 0..self.grid_height {
            for x in 0..self.grid_width {
                let i = (self.grid_height - y - 1) * self.grid_width + x;
                let mut xx = x as f32;
                let yy = y as f32;
                if self.clock != 0 {
                    if x >= 2 {
                        xx += COLON_WIDTH;
                    }
                    if x >= 4 {
                        xx += COLON_WIDTH;
                    }
                    if x >= 6 {
                        xx += COLON_WIDTH;
                    }
                }
                g.glx.push_matrix();
                g.glx.translate(xx * 2.01, yy * 1.98, 0.0);
                self.draw_flapper(g, i, text);
                if text && self.clock != 0 && (x == 2 || x == 4) {
                    self.draw_colon(g);
                }
                g.glx.pop_matrix();

                if text {
                    self.tick_flapper(i);
                    if self.flappers[i].current_index != self.flappers[i].target_index as f64 {
                        running += 1;
                    }
                }
            }
        }

        if text && running == 0 {
            if self.clock != 0 {
                self.fill_targets(g);
            } else if self.linger > 0 {
                self.linger -= 1;
                if self.linger == 0 {
                    self.fill_targets(g);
                }
            } else {
                // A second, plus a second for every twenty-five characters.
                self.linger = 30;
                if !self.first_time {
                    self.linger += (self.grid_width * self.grid_height) as i32 * 12 / 10;
                }
                self.first_time = false;
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let mut speed = g.res.float("speed") as f32;
    let mut grid_width = (g.res.int("width") as usize).max(1);
    let mut grid_height = (g.res.int("height") as usize).max(1);

    let clock = match g.res.string("mode") {
        "clock" | "clock12" => 12,
        "clock24" => 24,
        _ => 0,
    };
    if clock == 12 {
        grid_width = 8;
        grid_height = 1;
    } else if clock == 24 {
        grid_width = 6;
        grid_height = 1;
    }
    if clock != 0 {
        speed /= 4.0;
    }

    let spin = g.res.string("spin").to_string();
    let axis = |c: char, d: char| spin.contains(c) || spin.contains(d);
    let spin = [axis('x', 'X'), axis('y', 'Y'), axis('z', 'Z')];
    let face_front = g.res.bool("faceFront");
    let spin_speed = 0.5;

    let font = TexFont::load(&mut g.glx, g.res.string("flapFont"));
    let m = font.metrics("");
    let (ascent, descent) = (m.ascent, m.descent);

    let mut this = Splitflap {
        rot: Rotator::new(
            if spin[0] { spin_speed } else { 0.0 },
            if spin[1] { spin_speed } else { 0.0 },
            if spin[2] { spin_speed } else { 0.0 },
            0.5,
            if g.res.bool("wander") { 0.005 } else { 0.0 },
            false,
        ),
        rot2: face_front.then(|| Rotator::new(0.0, 0.0, 0.0, 0.0, 0.001, true)),
        trackball: Trackball::new(),
        spin,
        face_front,
        texinfo: Vec::new(),
        dlists: Vec::new(),
        component_colors: Vec::new(),
        text_color: [1.0; 4],
        flappers: Vec::new(),
        font,
        ascent,
        descent,
        clock,
        linger: 0,
        first_time: true,
        grid_width,
        grid_height,
        speed,
        wire,
        aspect: 1.0,
    };
    this.text_color = resource_color(g, "textColor");

    // One texture per character the board can show. Upstream builds them for
    // its whole Latin-1 spool; every cell here draws from ASCII or a subset of
    // it, so that is what is needed.
    for &text in &ASCII_SPOOL {
        let (texid, tw, th, m) = this.font.string_to_texture(&mut g.glx, text);
        this.texinfo.push(TexInfo {
            text,
            texid,
            width: m.width,
            ascent: m.ascent,
            descent: m.descent,
            tex_width: tw,
            tex_height: th,
        });
    }

    // The four modelled pieces and then the case, which is drawn.
    for i in 0..=MODELS.len() {
        let key = match i {
            QUARTER_FRAME => "frameColor",
            OUTER_FRAME => "caseColor",
            DISC_QUARTER => {
                if wire {
                    "frameColor"
                } else {
                    "discColor"
                }
            }
            _ => "finColor",
        };
        let mut c = resource_color(g, key);
        if wire && i == FIN_EDGE_HALF {
            c = [0.7, 0.7, 0.7, 1.0];
        }
        this.component_colors.push(c);

        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        g.glx.push_matrix();
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        match MODELS.get(i) {
            Some(src) => GlList::parse(src).render(&mut g.glx, wire),
            None => this.draw_outer_frame(g),
        }
        g.glx.pop_matrix();
        g.glx.end_list();
        this.dlists.push(list);
    }

    for i in 0..grid_width * grid_height {
        let spool: &'static [&'static str] = if clock == 0 {
            &ASCII_SPOOL
        } else {
            match i {
                0 => {
                    if clock == 12 {
                        &DIGIT_S1_SPOOL
                    } else {
                        &DIGIT_01_SPOOL
                    }
                }
                1 | 3 | 5 => &DIGIT_SPOOL,
                2 | 4 => &DIGIT_05_SPOOL,
                6 => &AP_SPOOL,
                _ => &M_SPOOL,
            }
        };
        let target_index = random() as usize % spool.len();
        this.flappers.push(Flapper {
            target_index,
            current_index: target_index as f64,
            sticky: 0.0,
            // One cell in ten has snapped a fin off.
            missing: if random().is_multiple_of(10) {
                Some(random() as usize % spool.len())
            } else {
                None
            },
            spool,
        });
    }
    if clock == 0 {
        g.text_reshape(grid_width as i32, grid_height as i32);
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Splitflap {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(40.0, self.aspect, 0.5, 25.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        // Ten times lower than usual, for better depth resolution.
        g.glx
            .look_at([0.0, 0.0, 3.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.color_material(false);
        g.glx.lighting(!self.wire);
        if !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.4, 0.2, 0.4, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        }

        g.glx.push_matrix();
        g.glx.scale(0.1, 0.1, 0.1);

        let turning = !self.trackball.button_down();
        let (x, y, z) = self.rot.position(turning);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 8.0,
            (z as f32 - 0.5) * 8.0,
        );
        g.glx.mult_matrix(self.trackball.matrix());

        if self.face_front {
            let (maxx, maxy, maxz) = (120.0f32, 60.0f32, 45.0f32);
            let (x, y, z) = match &mut self.rot2 {
                Some(r) => r.position(turning),
                None => (0.0, 0.0, 0.0),
            };
            if self.spin[0] {
                g.glx.rotate(maxy / 2.0 - x as f32 * maxy, 1.0, 0.0, 0.0);
            }
            if self.spin[1] {
                g.glx.rotate(maxx / 2.0 - y as f32 * maxx, 0.0, 1.0, 0.0);
            }
            if self.spin[2] {
                g.glx.rotate(maxz / 2.0 - z as f32 * maxz, 0.0, 0.0, 1.0);
            }
        } else {
            let (x, y, z) = self.rot.rotation(turning);
            g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
            g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        }

        // Fit the whole grid on the screen.
        let r = g.height() as f32 / g.width() as f32;
        let cells = if self.grid_width > self.grid_height {
            self.grid_width as f32 * r
        } else {
            self.grid_height as f32
        };
        let s = 8.0 / cells.max(1.0);
        g.glx.scale(s, s, s);

        self.draw_component(g, OUTER_FRAME);

        let xoff = match self.clock {
            12 => COLON_WIDTH * 3.0,
            24 => COLON_WIDTH * 2.0,
            _ => 0.0,
        };
        g.glx.translate(
            1.0 - (self.grid_width as f32 + xoff),
            1.0 - self.grid_height as f32,
            0.0,
        );

        // All the text goes after all the polygons, or the blending is wrong.
        self.draw_flappers(g, false);
        self.draw_flappers(g, true);

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       20000",
    "*showFPS:     False",
    "*wireframe:   False",
    "*speed:       1.0",
    "*width:       22",
    "*height:      8",
    "*spin:        XYZ",
    "*wander:      True",
    "*faceFront:   True",
    "*mode:        text",
    "*flapFont:    sans-serif bold 72",
    "*textColor:   #FFFFFF",
    "*frameColor:  #444444",
    "*caseColor:   #666666",
    "*discColor:   #888888",
    "*finColor:    #222222",
];

const MODES: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "text",
        label: "Text",
    },
    crate::runtime::opts::SelectItem {
        value: "clock12",
        label: "12 Hour Clock",
    },
    crate::runtime::opts::SelectItem {
        value: "clock24",
        label: "24 Hour Clock",
    },
];

const SPINS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "XYZ",
        label: "Sway all ways",
    },
    crate::runtime::opts::SelectItem {
        value: "0",
        label: "Do not sway",
    },
    crate::runtime::opts::SelectItem {
        value: "X",
        label: "Sway up and down",
    },
    crate::runtime::opts::SelectItem {
        value: "Y",
        label: "Sway side to side",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 5.0, 0.1, 1, "1.0"),
    Opt::select("mode", "Display", MODES, "text"),
    Opt::slider("width", "Columns", 1.0, 40.0, 1.0, 0, "22"),
    Opt::slider("height", "Rows", 1.0, 20.0, 1.0, 0, "8"),
    Opt::select("spin", "Sway", SPINS, "XYZ"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("faceFront", "Always face front", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "splitflap",
    label: "Split Flap",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2015",
        video: Some("https://www.youtube.com/watch?v=rZOL2jyDey0"),
        blurb: "A split-flap display, the kind that used to hang in railway \
                stations.",
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

    /// Every clock spool can show every digit its position needs, so the
    /// time always reads correctly rather than falling back to the first fin.
    #[test]
    fn a_clock_can_show_the_time() {
        for (mode, n) in [("clock12", 8), ("clock24", 6)] {
            let mut r = start(StartArgs::new(640, 480, &format!("mode={mode}"), 20260812));
            for _ in 0..40 {
                r.step();
            }
            let f = r.frame();
            assert!(!f.vertices.is_empty(), "{mode} drew nothing");
            let _ = n;
        }
        // The twelve-hour board has room for the hour, the minutes, the
        // seconds and the meridiem.
        assert_eq!(DIGIT_S1_SPOOL.len(), 2);
        assert_eq!(DIGIT_05_SPOOL.len(), 6);
        assert_eq!(AP_SPOOL, ["A", "P"]);
        for d in DIGIT_SPOOL {
            assert!(ASCII_SPOOL.contains(&d), "{d} has no texture");
        }
        for d in [DIGIT_S1_SPOOL, DIGIT_01_SPOOL, AP_SPOOL].concat() {
            assert!(ASCII_SPOOL.contains(&d), "{d} has no texture");
        }
        assert!(ASCII_SPOOL.contains(&M_SPOOL[0]));
    }

    /// The four modelled pieces all parse and are real geometry.
    #[test]
    fn the_cell_is_four_pieces() {
        for (i, src) in MODELS.iter().enumerate() {
            let m = GlList::parse(src);
            assert!(m.points > 20, "piece {i} is only {} vertices", m.points);
        }
    }

    /// Turning the crank reaches the wanted character and stops there rather
    /// than running past it for ever.
    #[test]
    fn the_spool_stops_where_it_is_asked() {
        let mut r = start(StartArgs::new(320, 240, "width=4&height=1", 20260812));
        for _ in 0..2000 {
            r.step();
        }
        assert!(!r.frame().vertices.is_empty(), "nothing was drawn");
    }

    /// `MOD` has to work with negatives: the fin below the current one is one
    /// step back round the spool.
    #[test]
    fn the_spool_wraps_backwards() {
        assert_eq!(modulo(-1, 10), 9);
        assert_eq!(modulo(0, 10), 0);
        assert_eq!(modulo(10, 10), 0);
        assert_eq!(modulo(11, 10), 1);
    }
}
