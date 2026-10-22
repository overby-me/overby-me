/* unicrud, Copyright © 2015-2026 Jamie Zawinski <jwz@jwz.org>
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Port of `hacks/glx/unicrud.c`.
//!
//! One character of Unicode at a time, spun in, held, and spun out again,
//! captioned with which plane and block it belongs to and what its bytes are.
//!
//! This was on the blocked list because it picks a codepoint anywhere from 0
//! to 0x2F800 and `runtime::font` is one compiled-in bitmap font of Latin
//! glyphs. That is only a blocker if the *crate* has to own the font, and it
//! does not: the saver draws its character as a texture on a quad rather than
//! as an outline it manipulates, and the host is a browser, which has fonts.
//! So `runtime::glyph` is a channel like the picture one, the host draws the
//! codepoint with whatever it has, and the coverage is better than upstream's
//! rather than worse, since upstream is limited to the X server's fonts.
//!
//! Upstream deals with a codepoint its font lacks by drawing it, noticing the
//! result is blank, and picking another. The same thing happens here, one
//! step earlier: the host says it has no glyph and the saver rolls again.
//! That matters more than it sounds, because most of the range is unassigned.
//!
//! The one thing that cannot be done is the character's *name*. Upstream
//! shells out to perl for it, noting in a comment that the alternative is to
//! embed the 943 KB of `NamesList.txt`. There is no perl here and that file
//! is not worth its weight, so the name line is left empty, which is exactly
//! what upstream shows when the lookup fails.

use crate::runtime::gl::{Blend, Shape};
use crate::runtime::texfont::TexFont;
#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, Saver3d, SaverDef, StartArgs};
#[cfg(target_arch = "wasm32")]
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs};
use crate::runtime::{Rotator, SelectItem, Trackball, XEvent, frand, random};

use super::unicodeblocks::BLOCKS;

/// The top of the range upstream picks from: through the CJK compatibility
/// ideographs supplement.
const MAX_CODEPOINT: u32 = 0x2F800;

/// How tall to ask the host to draw the character. It is scaled to fit
/// afterwards, so this is only how much detail there is to scale.
const GLYPH_PIXELS: i32 = 256;

/// Which part of the animation is running.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    In,
    Linger,
    Out,
}

/// The plane and block a codepoint falls in.
fn plane_and_block(c: u32) -> (&'static str, &'static str) {
    let mut plane = "Unassigned";
    let mut block = "Unassigned";
    for &(start, name) in BLOCKS {
        if c < start {
            break;
        }
        if let Some(p) = name.strip_prefix('*') {
            plane = p;
            block = "Unassigned";
        } else {
            block = name;
        }
    }
    (plane, block)
}

/// `utf8_encode`, which the caption prints the bytes of.
fn utf8_bytes(c: u32) -> Vec<u8> {
    match char::from_u32(c) {
        Some(ch) => {
            let mut buf = [0u8; 4];
            ch.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        None => Vec::new(),
    }
}

/// Whether a codepoint is in one of the named blocks. An empty list, or
/// "all", means every block.
fn matches_blocks(wanted: &str, block: &str) -> bool {
    if wanted.is_empty() || wanted.eq_ignore_ascii_case("all") {
        return true;
    }
    // Upstream matches its `--block` list against the block name with spaces
    // written as underscores, comma separated.
    let flat = block.replace(' ', "_");
    wanted
        .split(',')
        .any(|w| w.trim().eq_ignore_ascii_case(&flat))
}

struct UnicrudState {
    rot: Rotator,
    trackball: Trackball,
    color: [f32; 4],
    unichar: u32,
    plane: &'static str,
    block: &'static str,
    phase: Phase,
    spin_direction: f32,
    ratio: f64,
    /// The glyph the host drew, and how big it is.
    texid: u32,
    glyph_w: i32,
    glyph_h: i32,
    /// A request is out and the answer has not come back.
    waiting: bool,
    /// How many codepoints have been rejected for this turn, so a host with a
    /// thin font does not spin forever.
    tries: u32,
    title_font: TexFont,
    /// What draws the character when no host will: the compiled-in font,
    /// which has Latin and nothing else.
    char_font: TexFont,
    /// Whether anything is going to draw glyphs. Without a host the saver
    /// still runs, it just has nothing to show.
    glyphs: bool,
    spin: bool,
    wander: bool,
    titles: bool,
    speed: f64,
    blocks: String,
    width: i32,
    height: i32,
}

impl UnicrudState {
    /// `pick_unichar`: roll a codepoint in one of the wanted blocks and ask
    /// the host for it.
    fn pick_unichar(&mut self, g: &mut Gl) {
        // With nothing to draw glyphs, the only characters that can be shown
        // are the ones the compiled-in font has, which is printable ASCII.
        // Ranging over all of Unicode then would be an empty screen.
        let top = if self.glyphs { MAX_CODEPOINT } else { 0x7F };
        for _ in 0..0xF_0000 / 2 {
            let c = if self.glyphs {
                random() % top
            } else {
                0x21 + random() % (top - 0x21)
            };
            let (plane, block) = plane_and_block(c);
            if !matches_blocks(&self.blocks, block) {
                continue;
            }
            // Surrogates and non-characters are not things to draw.
            if char::from_u32(c).is_none() {
                continue;
            }
            self.unichar = c;
            self.plane = plane;
            self.block = block;
            break;
        }

        self.color = [
            0.5 + frand(0.5) as f32,
            0.5 + frand(0.5) as f32,
            0.5 + frand(0.5) as f32,
            1.0,
        ];

        if self.glyphs {
            g.request_glyph(self.unichar, GLYPH_PIXELS);
            self.waiting = true;
        }
    }

    /// Collect the glyph, and roll again if the host has none.
    fn collect_glyph(&mut self, g: &mut Gl) {
        let Some((c, image)) = g.take_glyph() else {
            return;
        };
        self.waiting = false;
        if c != self.unichar {
            return; /* an answer to a question we have moved on from */
        }
        match image {
            Some(img) => {
                let (w, h) = (img.width(), img.height());
                let mut pixels = Vec::with_capacity((w * h * 4) as usize);
                for p in img.pixels() {
                    // The host draws white on nothing, so the colour is the
                    // saver's and the glyph supplies only coverage.
                    let (r, _, _) = crate::runtime::color::unrgb(*p);
                    pixels.extend_from_slice(&[255, 255, 255, r]);
                }
                let id = if self.texid != 0 {
                    self.texid
                } else {
                    g.glx.gen_texture()
                };
                g.glx.bind_texture(id);
                g.glx.tex_image_2d(w, h, pixels);
                g.glx.tex_nearest(false);
                g.glx.tex_clamp(true);
                self.texid = id;
                self.glyph_w = w;
                self.glyph_h = h;
                self.tries = 0;
            }
            None => {
                // Nothing in any font the host has. Upstream draws it, sees a
                // blank, and picks again; this hears so first.
                self.tries += 1;
                if self.tries < 200 {
                    self.pick_unichar(g);
                } else {
                    self.glyph_w = 0;
                    self.tries = 0;
                }
            }
        }
    }

    /// `draw_unichar`: the character, scaled to a unit square, and the
    /// caption under it.
    fn draw_unichar(&self, g: &mut Gl) {
        if self.texid == 0 || self.glyph_w == 0 {
            // No host drew it, so the compiled-in font does. It only has
            // Latin, which is why `pick_unichar` stays inside ASCII when
            // there is no host to ask.
            let Some(ch) = char::from_u32(self.unichar) else {
                return;
            };
            let text = ch.to_string();
            let m = self.char_font.metrics(&text);
            let (w, h) = (m.width.max(1) as f32, (m.ascent + m.descent).max(1) as f32);
            g.glx.push_matrix();
            let s = 9.0;
            g.glx.scale(s, s, s);
            let s = 1.0 / w.max(h);
            g.glx.scale(s, s, s);
            g.glx.translate(-w / 2.0, -h / 2.0, 0.0);
            g.glx
                .color4f(self.color[0], self.color[1], self.color[2], self.color[3]);
            self.char_font.print_string(&mut g.glx, &text);
            g.glx.pop_matrix();
            return;
        }
        {
            let (w, h) = (self.glyph_w as f32, self.glyph_h as f32);
            g.glx.push_matrix();
            let s = 9.0;
            g.glx.scale(s, s, s);
            let s = 1.0 / w.max(h); /* Scale to unit */
            g.glx.scale(s, s, s);
            g.glx.translate(-w / 2.0, -h / 2.0, 0.0);

            g.glx.texturing(true);
            g.glx.bind_texture(self.texid);
            g.glx
                .color4f(self.color[0], self.color[1], self.color[2], self.color[3]);
            g.glx.front_face_cw(false);
            g.glx.begin(Shape::Quads);
            // Row zero of the glyph is its top, so the texture runs the other
            // way up from the quad, as it does for a map tile.
            g.glx.tex_coord2f(0.0, 1.0);
            g.glx.vertex3f(0.0, 0.0, 0.0);
            g.glx.tex_coord2f(1.0, 1.0);
            g.glx.vertex3f(w, 0.0, 0.0);
            g.glx.tex_coord2f(1.0, 0.0);
            g.glx.vertex3f(w, h, 0.0);
            g.glx.tex_coord2f(0.0, 0.0);
            g.glx.vertex3f(0.0, h, 0.0);
            g.glx.end();
            g.glx.texturing(false);
            g.glx.pop_matrix();
        }
    }

    /// The caption: plane, block, name, codepoint and the UTF-8 bytes.
    fn title(&self) -> String {
        let bytes = utf8_bytes(self.unichar);
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
        format!(
            "Plane:\t{}\nBlock:\t{}\nName:\t\nUnicode:\t{:04X}\nUTF-8:\t{}",
            self.plane,
            self.block,
            self.unichar,
            hex.join(" ")
        )
    }
}

impl Hack3d for UnicrudState {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        let h = f64::from(self.height) / f64::from(self.width);
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, (1.0 / h) as f32, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.collect_glyph(g);

        g.glx.clear_color(0.0, 0.0, 0.0, 1.0);
        g.glx.clear();
        g.glx.lighting(false);
        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.blend(Blend::Alpha);

        // Hold still until there is something to show, so the first character
        // is not spun in while it is still being drawn.
        if !self.waiting {
            self.ratio += match self.phase {
                Phase::In | Phase::Out => self.speed * 0.05,
                Phase::Linger => self.speed * 0.005,
            };
            if self.ratio > 1.0 {
                self.ratio = 0.0;
                self.phase = match self.phase {
                    Phase::In => Phase::Linger,
                    Phase::Linger => {
                        self.spin_direction = if random() & 1 != 0 { 1.0 } else { -1.0 };
                        Phase::Out
                    }
                    Phase::Out => {
                        self.pick_unichar(g);
                        Phase::In
                    }
                };
            }
        }

        g.glx.push_matrix();

        let button_down = self.trackball.button_down();
        if self.wander {
            let (x, y, z) = self.rot.position(!button_down);
            g.glx.translate(
                ((x - 0.5) * 6.0) as f32,
                ((y - 0.5) * 6.0) as f32,
                ((z - 0.5) * 6.0) as f32,
            );
        }
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        if self.spin {
            let (_, _, z) = self.rot.rotation(!button_down);
            g.glx.rotate((z * 360.0) as f32, 0.0, 0.0, 1.0);
        }

        // `SINOID` in the C: the scale eases in and out rather than ramping.
        let s = match self.phase {
            Phase::In => (std::f64::consts::PI - self.ratio / 2.0 * std::f64::consts::PI).sin(),
            Phase::Out => {
                (std::f64::consts::PI - (1.0 - self.ratio) / 2.0 * std::f64::consts::PI).sin()
            }
            Phase::Linger => 1.0,
        } as f32;
        g.glx.scale(s, s, s);
        g.glx.rotate(
            360.0 * s * self.spin_direction * if self.phase == Phase::In { -1.0 } else { 1.0 },
            0.0,
            0.0,
            1.0,
        );

        self.draw_unichar(g);
        g.glx.pop_matrix();

        if self.titles {
            let title = self.title();
            self.title_font.print_label(
                &mut g.glx,
                &title,
                self.width,
                self.height,
                1,
                [1.0, 1.0, 0.0, 1.0],
            );
        }

        g.res.int("delay").max(0) as u32
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let speed = g.res.float("speed");
    let mut st = UnicrudState {
        rot: Rotator::new(0.0, 0.0, 0.0, 0.0, speed * 0.003, true),
        trackball: Trackball::new(),
        color: [1.0; 4],
        unichar: 0,
        plane: "Unassigned",
        block: "Unassigned",
        phase: Phase::In,
        spin_direction: 1.0,
        ratio: 0.0,
        texid: 0,
        glyph_w: 0,
        glyph_h: 0,
        waiting: false,
        tries: 0,
        title_font: TexFont::load(&mut g.glx, "sans-serif 18"),
        char_font: TexFont::load(&mut g.glx, "sans-serif 48"),
        glyphs: g.glyphs_available(),
        spin: g.res.bool("spin"),
        wander: g.res.bool("wander"),
        titles: g.res.bool("titles"),
        speed,
        blocks: g.res.string("block").to_string(),
        width: g.width(),
        height: g.height(),
    };
    st.pick_unichar(g);
    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     30000",
    "*showFPS:   False",
    "*wireframe: False",
    "*spin:      True",
    "*wander:    True",
    "*speed:     1.0",
    "*block:     ALL",
    "*titles:    True",
];

/// The block presets from `hacks/config/unicrud.xml`.
const SETS: &[SelectItem] = &[
    SelectItem {
        value: "ALL",
        label: "Display all characters",
    },
    SelectItem {
        value: "Latin1,Latin_Extended-A,Latin_Extended-B,Spacing_Modifier_Letters",
        label: "Display Latin1",
    },
    SelectItem {
        value: "Latin1,Latin_Extended-A,Latin_Extended-B,Spacing_Modifier_Letters,\
                Phonetic_Extensions,Latin_Extended_Additional,Greek_Extended,\
                General_Punctuation,Superscripts_and_Subscripts,Currency_Symbols,\
                Letterlike_Symbols,Number_Forms",
        label: "Display simple characters",
    },
    SelectItem {
        value: "Greek_and_Coptic,Mathematical_Operators,\
                Miscellaneous_Mathematical_Symbols-A,Supplemental_Arrows-A,\
                Supplemental_Arrows-B,Miscellaneous_Mathematical_Symbols-B,\
                Supplemental_Mathematical_Operators,Miscellaneous_Symbols_and_Arrows",
        label: "Display mathematical symbols",
    },
    SelectItem {
        value: "Currency_Symbols,Miscellaneous_Technical,Box_Drawing,\
                Geometric_Shapes,Miscellaneous_Symbols,Dingbats,Mahjong_Tiles,\
                Domino_Tiles,Playing_Cards,Miscellaneous_Symbols_and_Pictographs,\
                Emoticons,Ornamental_Dingbats,Transport_and_Map_Symbols,\
                Alchemical_Symbols,Geometric_Shapes_Extended,\
                Supplemental_Symbols_and_Pictographs,Egyptian_Hieroglyphs",
        label: "Display emoticons",
    },
    SelectItem {
        value: "Egyptian_Hieroglyphs",
        label: "Display hieroglyphs",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::select("block", "Characters", SETS, "ALL"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("titles", "Show titles", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "unicrud",
    label: "Unicrud",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2015",
        video: Some("https://www.youtube.com/watch?v=prEzdYMZ7xA"),
        blurb: "Random Unicode characters, one at a time.",
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

    /// The block table is in ascending order over the range the saver draws
    /// from, which the lookup walks off the end of the moment it is not.
    ///
    /// Only over that range: upstream's table really is out of order once,
    /// with `Tags` at 0xE0020 listed after an `Unassigned` at 0xE0080. It
    /// never bites, because the loop stops at the block holding the character
    /// and characters stop at 0x2F800, a long way below. The disorder is
    /// carried across as it is rather than tidied, since tidying upstream's
    /// data invites the question of what else was tidied.
    #[test]
    fn the_block_table_is_sorted_where_it_is_used() {
        let mut last = 0;
        for &(start, name) in BLOCKS {
            if start > MAX_CODEPOINT {
                break;
            }
            assert!(start >= last, "{name} at {start:04X} follows {last:04X}");
            last = start;
        }

        // And the one disordered pair is where it is known to be, so this
        // test starts failing if a future table moves it down into range.
        let disordered: Vec<u32> = BLOCKS
            .windows(2)
            .filter(|w| w[1].0 < w[0].0)
            .map(|w| w[1].0)
            .collect();
        assert_eq!(disordered, vec![0xE0020]);
        assert!(disordered.iter().all(|&c| c > MAX_CODEPOINT));
    }

    /// Codepoints land in the block they belong to. These are the ones anyone
    /// can check by eye against the standard.
    #[test]
    fn characters_land_in_the_right_block() {
        for (c, block) in [
            (0x0041, "ASCII"),
            (0x00E9, "Latin1"),
            (0x0391, "Greek and Coptic"),
            (0x2200, "Mathematical Operators"),
            (0x3042, "Hiragana"),
        ] {
            let (_, got) = plane_and_block(c);
            assert_eq!(got, block, "U+{c:04X}");
        }
        // And the planes are named too.
        let (plane, _) = plane_and_block(0x1F600);
        assert_eq!(plane, "Supplementary Multilingual");
    }

    /// The block filter is what makes the Latin presets work, and it has to
    /// match the underscored spelling upstream's configuration file uses.
    #[test]
    fn the_block_filter_matches_the_presets() {
        assert!(matches_blocks("ALL", "Hiragana"));
        assert!(matches_blocks("", "Hiragana"));
        assert!(matches_blocks("Latin1,Latin_Extended-A", "Latin1"));
        assert!(matches_blocks(
            "Latin1,Greek_and_Coptic",
            "Greek and Coptic"
        ));
        assert!(!matches_blocks("Latin1,Latin_Extended-A", "Hiragana"));
    }

    /// The caption's bytes are the character's real UTF-8, which is one of
    /// the five things it prints and the only one that can be got wrong
    /// quietly.
    #[test]
    fn the_caption_prints_real_utf8() {
        assert_eq!(utf8_bytes(0x41), vec![0x41]);
        assert_eq!(utf8_bytes(0xE9), vec![0xC3, 0xA9]);
        assert_eq!(utf8_bytes(0x3042), vec![0xE3, 0x81, 0x82]);
        assert_eq!(utf8_bytes(0x1F600), vec![0xF0, 0x9F, 0x98, 0x80]);
        // A surrogate is not a character and encodes to nothing.
        assert!(utf8_bytes(0xD800).is_empty());
    }

    /// With no host to draw with, the saver runs and animates rather than
    /// stalling: it simply has no character to show.
    #[test]
    fn it_runs_with_no_glyphs() {
        let mut r = start(StartArgs::new(640, 480, "", 20260813));
        for _ in 0..200 {
            r.step();
        }
        assert!(!r.frame().vertices.is_empty(), "the caption was not drawn");
    }

    /// And with a host answering, it takes the glyph and draws it. A host
    /// that has no glyph makes it roll again, which is upstream's own answer
    /// to an unassigned codepoint and matters because most of the range is.
    #[test]
    fn it_rolls_again_when_the_host_has_no_glyph() {
        let mut r = Runner3d::start(
            &DEF,
            init,
            StartArgs::new(640, 480, "", 20260813).with_glyph_host(true),
        );
        let mut asked = Vec::new();
        let mut drawn = 0;
        for i in 0..400 {
            r.step();
            if let Some((c, size)) = r.take_glyph_request() {
                assert_eq!(size, GLYPH_PIXELS);
                asked.push(c);
                // Refuse the first several, as a thin font would.
                if asked.len() > 5 && i % 3 == 0 {
                    let mut img = crate::runtime::XImage::new(64, 64);
                    for y in 20..44 {
                        for x in 20..44 {
                            img.put_pixel(x, y, crate::runtime::color::rgb(255, 255, 255));
                        }
                    }
                    r.deliver_glyph(c, Some(img));
                    drawn += 1;
                } else {
                    r.deliver_glyph(c, None);
                }
            }
        }
        assert!(asked.len() > 6, "it asked for only {} glyphs", asked.len());
        assert!(drawn > 0, "it never accepted one");
        assert!(
            asked.iter().all(|&c| c < MAX_CODEPOINT),
            "a codepoint outside the range was asked for"
        );
    }
}
