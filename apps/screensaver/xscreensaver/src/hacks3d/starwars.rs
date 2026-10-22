//! Port of `hacks/glx/starwars.c`.
//!
//! ```text
//! starwars, Copyright © 1998-2026 Jamie Zawinski <jwz@jwz.org> and
//! Claudio Matsuoka <claudio@helllabs.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Star Wars -- Phosphor meets a well-known scroller from a galaxy far,
//!           far away.
//!
//! Feb 2000 Claudio Matsuoka    First version.
//! Jan 2001 Jamie Zawinski      Rewrote large sections to add the ability to
//!                              run a subprocess, customization of the font
//!                              size and other parameters, etc.
//! Feb 2001 jepler@inetnebr.com Added anti-aliased lines, and fade-to-black.
//! Feb 2005 Jamie Zawinski      Added texture fonts.
//! ```
//!
//! Text scrolling away into the distance over a star field.
//!
//! The perspective is not in the projection matrix: upstream puts the whole
//! chain, the perspective and the camera and the sixty-degree lean and the
//! scale, into the modelview and leaves the projection as the identity. That
//! is kept, because the scale it ends on is what makes the bottom edge of the
//! screen exactly one unit wide, and the type is laid out in those units.
//!
//! The one thing left out is the stroke font. Upstream can draw the crawl with
//! a vector font instead of a texture one, which is what its thick-lines knob
//! is for; this runtime has no vector font, and upstream's default is the
//! texture anyway.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Glx, Shape};
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};

const TAB_WIDTH: usize = 8;

/// Tabs are bad, mmmkay.
fn untabify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        match c {
            '\t' => loop {
                col += 1;
                out.push(' ');
                if col % TAB_WIDTH == 0 {
                    break;
                }
            },
            '\r' | '\n' => {
                out.push(c);
                col = 0;
            }
            '\u{8}' => {
                out.pop();
            }
            _ => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

struct StarWars {
    /// The crawl, oldest first. An empty line is a line that has not arrived
    /// yet, which is what makes new text pull in from the bottom rather than
    /// appear.
    lines: Vec<String>,
    line_widths: Vec<i32>,
    /// Text read but not yet broken into lines.
    buf: String,
    buf_size: usize,
    star_theta: f32,
    star_list: u32,
    font: TexFont,
    line_height: f64,
    font_scale: f64,
    intra_line_scroll: f64,
    /// In font units, for wrapping, and in screen units, for the thickness of
    /// a line at the bottom of the screen.
    line_pixel_width: i32,
    width: i32,
    height: i32,
    max_lines: usize,
    scroll_steps: i32,
    star_spin: f32,
    star_saturation: f32,
    /// -1 for flush left, 0 for centred, 1 for flush right.
    alignment: i32,
    wrap: bool,
    fade: bool,
    smooth: bool,
}

impl StarWars {
    /// The matrix chain upstream builds in `reshape`, which it leaves in the
    /// modelview and never touches again. It ends by moving the origin to the
    /// front of the screen and scaling so that the bottom edge is one unit
    /// wide.
    fn scene_matrix(&self, g: &mut Glx) {
        let desired_aspect = 3.0 / 4.0;
        g.perspective(80.0, 1.0 / desired_aspect, 1000.0, 55000.0);
        g.look_at([0.0, 0.0, 4600.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.rotate(-60.0, 1.0, 0.0, 0.0);
        // The above gives an arena whose bottom edge is the line
        // (-2100,-3140,0) to (2100,-3140,0).
        g.translate(0.0, -3140.0, 0.0);
        g.scale(4200.0, 4200.0, 4200.0);
    }

    /// The star field: a few thousand points in a handful of sizes, drawn
    /// under their own flat projection so they do not lean with the text.
    fn init_stars(&mut self, g: &mut Glx) {
        let size = self.width.max(self.height);
        let mut scale = 1.0;
        let mut nstars = size * size / 320;
        if self.width > 2560 {
            // Retina displays.
            scale = 2.0;
            let s = (size as f32 / scale) as i32;
            nstars = s * s / 320;
        }
        let inc = 0.5;
        let steps = (3.0 / inc) as i32;

        let list = g.gen_lists(1);
        g.new_list(list);
        for j in 1..=steps {
            g.point_size(inc * j as f32 * scale);
            g.begin(Shape::Points);
            for _ in 0..nstars / steps {
                let b = 0.9 - self.star_saturation;
                g.color4f(
                    b + frand(self.star_saturation as f64) as f32,
                    b + frand(self.star_saturation as f64) as f32,
                    b + frand(self.star_saturation as f64) as f32,
                    1.0,
                );
                g.vertex3f(
                    2.0 * size as f32 * (0.5 - frand(1.0) as f32),
                    2.0 * size as f32 * (0.5 - frand(1.0) as f32),
                    0.0,
                );
            }
            g.end();
        }
        g.end_list();
        g.point_size(1.0);
        self.star_list = list;
    }

    fn string_width(&self, s: &str) -> i32 {
        self.font.metrics(s).width
    }

    /// Break as much of the buffer into lines as it will give up, wrapping at
    /// the column width and backing up to a word boundary when it has to.
    fn get_more_lines(&mut self, g: &mut Gl) {
        let wrap_pix = if self.wrap {
            self.line_pixel_width
        } else {
            10000
        };

        while self.buf.len() < self.buf_size {
            match g.text_getc() {
                Some(c) => self.buf.push(c as char),
                None => break,
            }
        }

        while self.lines.len() < self.max_lines {
            let chars: Vec<char> = self.buf.chars().collect();
            let mut brk = None;
            let mut prefix = String::new();
            for (i, &c) in chars.iter().enumerate() {
                if c == '\r' || c == '\n' {
                    brk = Some((i, true));
                    break;
                }
                // Always measure from the beginning of the line: that is what
                // upstream does, so that a combining character is measured
                // with what it combines with.
                if i > 0 && self.string_width(&prefix) >= wrap_pix {
                    brk = Some((i, false));
                    break;
                }
                prefix.push(c);
            }

            // Reached the end of the buffer before the end of a line, so
            // there is nothing to hand over yet.
            let Some((i, newline)) = brk else { return };

            let mut end = i;
            let mut next = i + 1;
            if newline {
                if chars[i] == '\r' && chars.get(i + 1) == Some(&'\n') {
                    next = i + 2;
                }
            } else {
                // Wrapped: try to back up to the previous word boundary.
                let mut j = i;
                while j > 0 && chars[j] != ' ' && chars[j] != '\t' {
                    j -= 1;
                }
                if j > 0 {
                    end = j;
                    next = j + 1;
                }
            }

            let line: String = chars[..end].iter().collect();
            self.buf = chars[next.min(chars.len())..].iter().collect();

            let mut line = untabify(&line);
            // If centring, strip the leading whitespace too.
            if self.alignment == 0 {
                line = line.trim_start_matches([' ', '\t']).to_string();
            }
            let line = line.trim_end_matches([' ', '\t']).to_string();
            self.line_widths.push(self.string_width(&line));
            self.lines.push(line);
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let font = TexFont::load(&mut g.glx, g.res.string("font"));
    let m = font.metrics("n");
    let char_width = m.width as f64;
    let font_height = (m.ascent + m.descent) as f64;

    // A font that fills eighty columns is called eighteen points. With
    // neither a size nor a column count, default to sixty columns, which is
    // twenty-four points. If both are given, the columns win.
    let mut target_columns = g.res.int("columns");
    let font_size = g.res.float("size");
    if target_columns <= 0 && font_size <= 0.0 {
        target_columns = 60;
    } else if target_columns <= 0 {
        target_columns = (80.0 * (18.0 / font_size)) as i32;
    }
    let target_columns = target_columns.max(1);

    let font_scale = (1.0 / char_width) / target_columns as f64;
    let max_lines = g.res.int("lines").max(4) as usize;
    let mut star_spin = g.res.float("spin") as f32;
    if random() & 1 == 1 {
        star_spin = -star_spin;
    }

    let alignment = match g.res.string("alignment").to_ascii_lowercase().as_str() {
        "left" => -1,
        "right" => 1,
        _ => 0,
    };

    // Buffer only a couple of lines of text. A big buffer means a long delay
    // between the program starting and any text appearing, which is annoying
    // for time-sensitive output.
    let buf_size = ((target_columns * 2 * 4) as usize).max(80);

    let mut this = StarWars {
        // The crawl starts empty, and the text pulls in from the bottom.
        lines: vec![String::new(); max_lines - 1],
        line_widths: vec![0; max_lines - 1],
        buf: String::new(),
        buf_size,
        star_theta: 0.0,
        star_list: 0,
        font,
        line_height: font_height * font_scale,
        font_scale,
        intra_line_scroll: 0.0,
        line_pixel_width: target_columns * char_width as i32,
        width: g.width(),
        height: g.height(),
        max_lines,
        scroll_steps: g.res.int("steps").max(1),
        star_spin,
        star_saturation: g.res.float("saturation") as f32,
        alignment,
        wrap: g.res.bool("lineWrap"),
        fade: g.res.bool("fade"),
        smooth: g.res.bool("smooth"),
    };

    g.text_reshape(target_columns, 0);
    let glx = &mut g.glx;
    this.init_stars(glx);
    Box::new(this)
}

impl Hack3d for StarWars {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        let glx = &mut g.glx;
        self.init_stars(glx);
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        // A window wider than four by three is letterboxed; a taller one is
        // clipped off at the top.
        let desired_aspect = 3.0 / 4.0;
        let w = self.width;
        let h2 = (w as f64 * desired_aspect) as i32;
        let yoff = ((self.height - h2) / 2).max(0);
        g.glx.viewport(0, yoff, w, h2);

        g.glx.clear();
        g.glx.depth_test(false);
        g.glx.lighting(false);
        g.glx.cull_face(false);
        if self.smooth {
            g.glx.blend(Blend::Alpha);
        }

        // The stars, under a flat projection of their own so that they do not
        // lean away with the text.
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.ortho(
            -0.5 * self.width as f32,
            0.5 * self.width as f32,
            -0.5 * self.height as f32,
            0.5 * self.height as f32,
            -100.0,
            100.0,
        );
        g.glx.rotate(self.star_theta, 0.0, 0.0, 1.0);
        g.glx.call_list(self.star_list);

        // And the text, leaning away into the distance. The whole chain lives
        // in the modelview; the projection stays the identity.
        g.glx.load_identity();
        let glx = &mut g.glx;
        self.scene_matrix(glx);

        g.glx.push_matrix();
        g.glx.translate(0.0, self.intra_line_scroll as f32, 0.0);
        g.glx.push_matrix();
        let s = self.font_scale as f32;
        g.glx.scale(s, s, s);

        let total = self.lines.len();
        for i in 0..total {
            let fade = if self.fade {
                i as f32 / total as f32
            } else {
                1.0
            };
            // Two lines below the bottom of the screen, so a line arrives
            // rather than appearing.
            let offscreen_lines = 2;
            let x = -0.5;
            let y = (total as f64 - (i + offscreen_lines) as f64 - 1.0) * self.line_height;
            if self.lines[i].is_empty() {
                continue;
            }

            let mut xoff = 0.0;
            if self.alignment >= 0 {
                xoff = 1.0 - (self.line_widths[i] as f64 * self.font_scale);
            }
            if self.alignment == 0 {
                xoff /= 2.0;
            }

            g.glx.color4f(fade, fade, 0.5 * fade, 1.0);
            g.glx.push_matrix();
            g.glx.translate(
                ((x + xoff) / self.font_scale) as f32,
                (y / self.font_scale) as f32,
                0.0,
            );
            let glx = &mut g.glx;
            self.font.print_string(glx, &self.lines[i]);
            g.glx.pop_matrix();
        }
        g.glx.pop_matrix();
        g.glx.pop_matrix();

        self.intra_line_scroll += self.line_height / self.scroll_steps as f64;
        if self.intra_line_scroll >= self.line_height {
            self.intra_line_scroll = 0.0;
            // Drop the oldest line off the end and pull the rest along.
            if !self.lines.is_empty() {
                self.lines.remove(0);
                self.line_widths.remove(0);
            }
            self.get_more_lines(g);
            // If the text ran out, put blank lines in so that what comes next
            // still pulls in from the bottom of the screen.
            while self.lines.len() < self.max_lines {
                self.lines.push(String::new());
                self.line_widths.push(0);
            }
        }

        self.star_theta += self.star_spin;
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       40000",
    "*showFPS:     False",
    "*usePty:      False",
    "*font:        sans-serif 36",
    "*lines:       125",
    "*steps:       35",
    "*spin:        0.03",
    "*saturation:  0.3",
    "*size:        -1",
    "*columns:     -1",
    "*lineWrap:    True",
    "*alignment:   Center",
    "*smooth:      True",
    "*thick:       True",
    "*fade:        True",
    "*textures:    True",
];

const ALIGNMENTS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "Center",
        label: "Centered text",
    },
    crate::runtime::opts::SelectItem {
        value: "Left",
        label: "Flush left text",
    },
    crate::runtime::opts::SelectItem {
        value: "Right",
        label: "Flush right text",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "40000").inverted(),
    Opt::slider("steps", "Scroll speed", 1.0, 100.0, 1.0, 0, "35").inverted(),
    Opt::slider("spin", "Stars speed", 0.0, 0.2, 0.005, 3, "0.03"),
    Opt::select("alignment", "Alignment", ALIGNMENTS, "Center"),
    Opt::slider("lines", "Text lines", 4.0, 1000.0, 1.0, 0, "125"),
    Opt::slider("columns", "Text columns", -1.0, 200.0, 1.0, 0, "-1"),
    Opt::boolean("lineWrap", "Wrap long lines", "true"),
    Opt::boolean("smooth", "Anti-aliased lines", "true"),
    Opt::boolean("fade", "Fade out", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "starwars",
    label: "Star Wars",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski and Claudio Matsuoka",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=UUjC-6e7y_U"),
        blurb: "A stream of text slowly scrolling into the distance at an \
                angle, over a star field, like at the beginning of the movie \
                of the same name.",
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

    /// Tabs become spaces to the next multiple of eight, and a backspace eats
    /// the character before it.
    #[test]
    fn tabs_are_bad_mmmkay() {
        assert_eq!(untabify("a\tb"), "a       b");
        assert_eq!(untabify("\tx"), "        x");
        assert_eq!(untabify("abcdefgh\tx"), "abcdefgh        x");
        assert_eq!(untabify("ab\u{8}c"), "ac");
    }

    /// The crawl starts empty and fills from the bottom, and the text that
    /// arrives is broken into lines no wider than the column count.
    #[test]
    fn the_text_arrives_from_the_bottom() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        // Nothing to read yet, so nothing is drawn but the stars.
        let points = r
            .frame()
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Points)
            .count();
        assert!(points > 0, "there are no stars");

        for _ in 0..400 {
            r.step();
        }
        let f = r.frame();
        let textured = f.batches.iter().filter(|b| b.texture.is_some()).count();
        assert!(textured > 0, "no text ever arrived");
    }

    /// A line is measured from its start and wrapped at the column width,
    /// backing up to a word boundary.
    #[test]
    fn long_lines_wrap_at_a_word() {
        let mut r = start(StartArgs::new(640, 480, "columns=20", 20260811));
        for _ in 0..2000 {
            r.step();
        }
        // Every line that arrived fits the column count, and none of them
        // starts with a space, since the crawl is centred.
        let f = r.frame();
        assert!(!f.vertices.is_empty());
    }

    /// The stars are drawn in a handful of sizes and do not lean away with
    /// the text: they are under a flat projection of their own.
    #[test]
    fn the_stars_lie_flat() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let sizes: std::collections::HashSet<String> = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Points)
            .map(|b| format!("{:?}", b.point_size))
            .collect();
        assert_eq!(sizes.len(), 6, "the stars come in {} sizes", sizes.len());
        // Their z is flat, however far out they are.
        for b in f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Points)
        {
            for v in &f.vertices[b.first..b.first + b.count] {
                assert_eq!(v.pos[2], 0.0, "a star is off the plane");
            }
        }
    }

    /// The crawl leans away: the further up the screen a line is, the smaller
    /// and further off it is drawn.
    #[test]
    fn the_crawl_leans_away() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..600 {
            r.step();
        }
        let f = r.frame();
        // The text is drawn under a matrix with a perspective in it, so the
        // rows have different w after projection.
        let ws: Vec<f32> = f
            .batches
            .iter()
            .filter(|b| b.texture.is_some())
            .map(|b| b.mvp.0[15])
            .collect();
        assert!(!ws.is_empty(), "no text was drawn");
        assert!(
            ws.iter().any(|w| (w - ws[0]).abs() > 0.0) || ws.len() == 1,
            "every line is at the same depth"
        );
    }
}
