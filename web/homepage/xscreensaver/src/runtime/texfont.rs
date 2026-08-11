//! Port of `hacks/glx/texfont.c`.
//!
//! ```text
//! texfonts, Copyright © 2005-2025 Jamie Zawinski <jwz@jwz.org>
//! Loads X11 fonts into textures for use with OpenGL.
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
//! Text for the OpenGL savers, of which thirty-one want some.
//!
//! Upstream asks the X server or Xft to rasterise a string and uploads the
//! result as a texture, one texture per string, rebuilt whenever the string
//! changes. There is no X server here and [`super::font`] has exactly one
//! compiled-in bitmap font, so this goes the other way round: the whole font is
//! baked into a single 16-by-16 atlas once, and a string is drawn as one quad
//! per character reading out of it. That is cheaper than upstream and it means
//! a string can change every frame for nothing.
//!
//! What is kept is the part that shows: [`TexFont::print_label`] draws its text
//! five times, four in an outline colour a pixel out in each diagonal and once
//! on top in the text colour, which is what makes a caption readable over
//! whatever the saver happens to be drawing behind it.
//!
//! Not implemented: upstream renders digits inside square brackets as
//! subscripts, for the chemical formulae in `molecule` and `dnalogo`. Neither
//! is ported yet, and the brackets pass through as themselves until one is.

use super::font::{Font, glyph_row};
use super::gl::{Blend, Glx, Shape};

/// The atlas is sixteen cells square, which covers the font's 256 glyphs.
const COLS: i32 = 16;
const ROWS: i32 = 16;

/// A font baked into one texture, with the metrics to lay it out.
pub struct TexFont {
    font: Font,
    texture: u32,
    /// The size of one cell in the atlas, in texture pixels.
    cell_w: i32,
    cell_h: i32,
}

/// The bounding box of a laid-out string, in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    pub width: i32,
    /// How far the first line rises above its baseline.
    pub ascent: i32,
    /// How far the last line falls below it, which for a multi-line string
    /// includes every line after the first.
    pub descent: i32,
}

impl TexFont {
    /// `load_texture_font`. The name is read for a size the same way
    /// [`Font::load`] reads it, and everything else about it is ignored.
    pub fn load(g: &mut Glx, spec: &str) -> TexFont {
        let font = Font::load(spec);
        let (cell_w, cell_h) = (font.char_width(), font.line_height());

        // One RGBA pixel per texel: white where the glyph is, and clear
        // everywhere else, so the colour comes from the vertex.
        let (tw, th) = (cell_w * COLS, cell_h * ROWS);
        let mut px = vec![0u8; (tw * th * 4) as usize];
        let scale = cell_h / super::font::CELL_H;
        for c in 0..256u32 {
            let (col, row) = (c as i32 % COLS, c as i32 / COLS);
            let ch = char::from_u32(c).unwrap_or(' ');
            for y in 0..cell_h {
                let bits = glyph_row(ch, y / scale.max(1));
                for x in 0..cell_w {
                    // The glyph's twelve bits sit in the top of the word.
                    let bit = 15 - (x / scale.max(1));
                    if bits >> bit & 1 == 0 {
                        continue;
                    }
                    let (px_x, px_y) = (col * cell_w + x, row * cell_h + y);
                    let at = ((px_y * tw + px_x) * 4) as usize;
                    px[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }

        let texture = g.gen_texture();
        g.bind_texture(texture);
        g.tex_image_2d(tw, th, px);
        g.tex_clamp(true);
        g.tex_nearest(true);

        TexFont {
            font,
            texture,
            cell_w,
            cell_h,
        }
    }

    pub fn ascent(&self) -> i32 {
        self.font.ascent()
    }

    pub fn descent(&self) -> i32 {
        self.font.descent()
    }

    pub fn line_height(&self) -> i32 {
        self.cell_h
    }

    /// A tab stop is seven `m` widths, and every glyph here is an `m` width.
    fn tab_width(&self) -> i32 {
        self.cell_w * 7
    }

    /// Walk a string, calling `place` with the cell rectangle of every glyph.
    /// Newlines and tab stops are honoured. Returns the bounding box.
    ///
    /// `y` grows downwards, as it does in upstream's measuring pass: the
    /// origin is the first line's baseline, so later lines look like very deep
    /// descenders.
    fn iterate(&self, s: &str, mut place: impl FnMut(char, i32, i32)) -> Metrics {
        let (mut x, mut y) = (0, 0);
        let mut width = 0;
        for ch in s.chars() {
            match ch {
                '\n' => {
                    width = width.max(x);
                    x = 0;
                    y += self.cell_h;
                }
                '\t' => {
                    let t = self.tab_width();
                    x = ((x + t) / t) * t;
                }
                _ => {
                    place(ch, x, y);
                    x += self.cell_w;
                }
            }
        }
        width = width.max(x);
        Metrics {
            width,
            ascent: self.font.ascent(),
            descent: self.font.descent() + y,
        }
    }

    /// `texture_string_metrics`: how big the string comes out.
    pub fn metrics(&self, s: &str) -> Metrics {
        self.iterate(s, |_, _, _| {})
    }

    /// `print_texture_string`: draw the string in the scene, its first
    /// baseline at the origin of the current matrix, one unit per pixel.
    pub fn print_string(&self, g: &mut Glx, s: &str) {
        let (tw, th) = ((self.cell_w * COLS) as f32, (self.cell_h * ROWS) as f32);
        let (cw, ch) = (self.cell_w as f32, self.cell_h as f32);
        let (asc, desc) = (self.ascent() as f32, self.descent() as f32);

        g.texturing(true);
        g.bind_texture(self.texture);
        g.begin(Shape::Quads);

        let mut quads: Vec<(char, i32, i32)> = Vec::new();
        self.iterate(s, |c, x, y| quads.push((c, x, y)));

        for (c, x, y) in quads {
            let code = c as u32;
            if code > 255 {
                continue; /* the atlas has no cell for it */
            }
            let (col, row) = ((code as i32 % COLS) as f32, (code as i32 / COLS) as f32);
            let u0 = col * cw / tw;
            let u1 = (col + 1.0) * cw / tw;
            let v0 = row * ch / th;
            let v1 = (row + 1.0) * ch / th;

            // The measuring pass counts y downwards; the scene counts it up.
            let (x, base) = (x as f32, -(y as f32));
            for (u, v, px, py) in [
                (u0, v1, x, base - desc),
                (u1, v1, x + cw, base - desc),
                (u1, v0, x + cw, base + asc),
                (u0, v0, x, base + asc),
            ] {
                g.tex_coord2f(u, v);
                g.vertex3f(px, py, 0.0);
            }
        }
        g.end();
        g.texturing(false);
    }

    /// `print_texture_label`: draw the string over the window at a fixed place,
    /// with an outline so that it reads over anything.
    ///
    /// `position` is 0 for the centre, 1 for the top left, 2 for the bottom
    /// left, as upstream numbers them. `color` is the text colour; the outline
    /// is black or white, whichever contrasts with it.
    pub fn print_label(
        &self,
        g: &mut Glx,
        s: &str,
        window_width: i32,
        window_height: i32,
        position: i32,
        color: [f32; 4],
    ) {
        if s.is_empty() {
            return;
        }

        // Light text takes a dark outline and dark text a light one. A solid
        // white outline round black text looks wrong, so the outline is drawn
        // half transparent either way.
        let luma = color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
        let edge = if luma > 0.4 { 0.0 } else { 1.0 };
        let color2 = [edge, edge, edge, color[3] * 0.5];

        let m = self.metrics(s);
        let h = m.ascent + m.descent;
        let (x, y) = match position {
            1 => (m.ascent, window_height - m.ascent * 2),
            2 => (m.ascent, h),
            _ => (
                (window_width - m.width) / 2,
                (window_height + h) / 2 - m.ascent,
            ),
        };

        g.matrix_mode_projection();
        g.push_matrix();
        g.load_identity();
        g.ortho(
            0.0,
            window_width as f32,
            0.0,
            window_height as f32,
            -1.0,
            1.0,
        );
        g.matrix_mode_modelview();
        g.push_matrix();
        g.load_identity();

        let (depth, lighting, fog) = (g.depth_test_enabled(), g.lighting_enabled(), g.fog_set());
        g.depth_test(false);
        g.lighting(false);
        g.fog(None);
        g.cull_face(false);
        g.blend(Blend::Alpha);

        // Five passes: four for the border and one on top.
        for (dx, dy) in [(-1, -1), (-1, 1), (1, 1), (1, -1), (0, 0)] {
            let c = if (dx, dy) == (0, 0) { color } else { color2 };
            g.color4f(c[0], c[1], c[2], c[3]);
            g.push_matrix();
            g.translate((x + dx) as f32, (y + dy) as f32, 0.0);
            self.print_string(g, s);
            g.pop_matrix();
        }

        g.blend(Blend::Off);
        g.depth_test(depth);
        g.lighting(lighting);
        g.fog(fog);

        g.pop_matrix();
        g.matrix_mode_projection();
        g.pop_matrix();
        g.matrix_mode_modelview();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_font() -> (Glx, TexFont) {
        let mut g = Glx::new();
        let f = TexFont::load(&mut g, "-*-helvetica-medium-r-normal-*-220-*");
        (g, f)
    }

    /// The atlas holds every one of the font's glyphs, and has ink in it.
    #[test]
    fn the_whole_font_is_one_texture() {
        let (g, f) = a_font();
        let t = g.texture(f.texture).expect("no atlas");
        assert_eq!(t.width, f.cell_w * COLS);
        assert_eq!(t.height, f.cell_h * ROWS);
        assert!(t.nearest, "text should not be smoothed");

        // A capital A has ink; a space does not.
        let ink_in = |c: char| {
            let code = c as i32;
            let (col, row) = (code % COLS, code / COLS);
            let mut lit = 0;
            for y in 0..f.cell_h {
                for x in 0..f.cell_w {
                    let at = (((row * f.cell_h + y) * t.width + col * f.cell_w + x) * 4) as usize;
                    if t.data[at + 3] != 0 {
                        lit += 1;
                    }
                }
            }
            lit
        };
        assert!(ink_in('A') > 10, "the letter A is blank");
        assert_eq!(ink_in(' '), 0, "the space has ink in it");
    }

    /// A string measures as wide as its characters and as tall as its lines.
    #[test]
    fn a_string_measures_by_its_characters() {
        let (_, f) = a_font();
        let one = f.metrics("hello");
        assert_eq!(one.width, 5 * f.cell_w);
        assert_eq!(one.ascent, f.ascent());
        assert_eq!(one.descent, f.descent());

        // Two lines are as wide as the wider and one line deeper.
        let two = f.metrics("hello\nworld!");
        assert_eq!(two.width, 6 * f.cell_w);
        assert_eq!(two.descent, f.descent() + f.line_height());

        // A tab jumps to the next stop of seven characters.
        let tabbed = f.metrics("a\tb");
        assert_eq!(tabbed.width, f.cell_w * 7 + f.cell_w);
    }

    /// Every character is one quad, and the newlines and tabs are not.
    #[test]
    fn a_string_is_a_quad_per_character() {
        let (mut g, f) = a_font();
        g.start_frame(640, 480);
        f.print_string(&mut g, "ab\tc\nd");
        let frame = g.frame();
        // Four printable characters, two triangles each: the tab and the
        // newline move the pen and draw nothing.
        let verts: usize = frame.batches.iter().map(|b| b.count).sum();
        assert_eq!(verts, 4 * 6);
        assert!(
            frame.batches.iter().all(|b| b.texture == Some(f.texture)),
            "the text was drawn without its atlas"
        );
    }

    /// The label is drawn five times, four of them offset by a pixel, which is
    /// what gives it its outline.
    #[test]
    fn a_label_is_drawn_five_times_for_its_outline() {
        let (mut g, f) = a_font();
        g.start_frame(640, 480);
        f.print_label(&mut g, "hi", 640, 480, 1, [1.0, 1.0, 0.0, 1.0]);
        let frame = g.frame();

        let verts: usize = frame.batches.iter().map(|b| b.count).sum();
        assert_eq!(verts, 5 * 2 * 6, "two glyphs, five passes");

        // Four passes in the outline colour and one in the text colour.
        let text = frame
            .vertices
            .iter()
            .filter(|v| v.color[0] == 1.0 && v.color[1] == 1.0 && v.color[2] == 0.0)
            .count();
        assert_eq!(text, 2 * 6);
        // Yellow is light, so the outline is black and half transparent.
        let outline = frame
            .vertices
            .iter()
            .filter(|v| v.color[0] == 0.0 && v.color[3] == 0.5)
            .count();
        assert_eq!(outline, 4 * 2 * 6);

        // And it is drawn flat, over whatever was there.
        assert!(frame.batches.iter().all(|b| !b.depth_test && !b.lighting));
    }

    /// Drawing a label puts the matrices back the way it found them, or the
    /// saver's next frame would be drawn through the label's projection.
    #[test]
    fn a_label_leaves_the_matrices_alone() {
        let (mut g, f) = a_font();
        g.start_frame(640, 480);
        g.matrix_mode_projection();
        g.load_identity();
        g.perspective(30.0, 1.33, 1.0, 100.0);
        g.matrix_mode_modelview();
        g.load_identity();
        g.translate(1.0, 2.0, 3.0);

        g.begin(Shape::Points);
        g.vertex3f(0.0, 0.0, 0.0);
        g.end();
        let before = g.frame().batches[0].mvp.0;

        f.print_label(&mut g, "hi", 640, 480, 0, [1.0; 4]);

        g.begin(Shape::Points);
        g.vertex3f(0.0, 0.0, 0.0);
        g.end();
        let after = g.frame().batches.last().unwrap().mvp.0;
        assert_eq!(before, after, "the label left the matrices changed");
    }
}
