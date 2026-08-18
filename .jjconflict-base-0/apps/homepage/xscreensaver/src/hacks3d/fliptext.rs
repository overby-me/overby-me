//! Port of `hacks/glx/fliptext.c`.
//!
//! ```text
//! fliptext, Copyright (c) 2005-2019 Jamie Zawinski <jwz@jwz.org>
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
//! Text flying in and out again, a clusterful of lines at a time.
//!
//! A line is a little state machine: it waits its turn, slides or spins in
//! from somewhere off to one side, holds still, and then leaves the other way.
//! The wait is what staggers a cluster, so the lines arrive one after another
//! rather than as a block, and which end of the cluster waits longest is what
//! makes the same three paths look like six.
//!
//! Along a path it moves exponentially rather than evenly, slow side toward
//! the middle, so a line decelerates as it arrives and accelerates as it goes.
//! Its opacity follows the same curve, so it fades in exactly as it slows.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Blend;
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};

const TAB_WIDTH: usize = 8;

/// Three uniform numbers averaged: the middle of the range far more often
/// than either end.
fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    New,
    Hesitate,
    In,
    Linger,
    Out,
    Dead,
}

impl State {
    fn next(self) -> State {
        match self {
            State::New => State::Hesitate,
            State::Hesitate => State::In,
            State::In => State::Linger,
            State::Linger => State::Out,
            _ => State::Dead,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Anim {
    ScrollBottom,
    ScrollTop,
    Spin,
}

struct Line {
    text: String,
    width: f32,
    height: f32,
    from: [f32; 3],
    to: [f32; 3],
    current: [f32; 3],
    /// Rotation about the y axis: where it starts, where it ends, and where
    /// it has got to.
    fth: f32,
    tth: f32,
    cth: f32,
    cluster_size: i32,
    cluster_pos: i32,
    state: State,
    step: i32,
    steps: i32,
    color: [f32; 4],
}

struct FlipText {
    lines: Vec<Line>,
    /// Text read but not yet broken into lines.
    buf: String,
    buf_size: usize,
    font: TexFont,
    char_width: i32,
    line_height: i32,
    font_scale: f32,
    font_wrap_pixels: i32,
    top_margin: f32,
    bottom_margin: f32,
    left_margin: f32,
    right_margin: f32,
    anim_type: Anim,
    in_pos: [f32; 3],
    mid: [f32; 3],
    out: [f32; 3],
    rotation: [f32; 3],
    color: [f32; 4],
    /// -1 for flush left, 0 for centred, 1 for flush right, and whether to
    /// pick a new one for every cluster.
    alignment: i32,
    alignment_random: bool,
    max_lines: i32,
    min_lines: i32,
    speed: f32,
    aspect: f32,
    wire: bool,
}

impl FlipText {
    fn char_width_of(&self, c: char) -> i32 {
        self.font.metrics(&c.to_string()).width
    }

    /// One line of text from the buffer, wrapped to the column width, or
    /// nothing if there is not a whole line to be had yet.
    fn get_one_line(&mut self, g: &mut Gl) -> Option<String> {
        let wrap_pix = self.font_wrap_pixels;

        // Fill up, but stop at a newline.
        while self.buf.len() < self.buf_size {
            match g.text_getc() {
                Some(c) => {
                    self.buf.push(c as char);
                    if c == b'\r' || c == b'\n' {
                        break;
                    }
                }
                None => break,
            }
        }

        let chars: Vec<char> = self.buf.chars().collect();
        let mut col = 0usize;
        let mut col_pix = 0;
        let mut i = 0;
        loop {
            // Reached the end of the buffer before the end of a line.
            if i >= chars.len() {
                return None;
            }
            let c = chars[i];
            let cw = self.char_width_of(c);
            if c == '\r' || c == '\n' || col_pix + cw >= wrap_pix {
                let mut end = i;
                let mut next = i + 1;
                if c == '\r' || c == '\n' {
                    if c == '\r' && chars.get(i + 1) == Some(&'\n') {
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
                return Some(line.trim_end_matches([' ', '\t']).to_string());
            }
            col += 1;
            col_pix += cw;
            if c == '\t' {
                let tab_pix = TAB_WIDTH as i32 * self.char_width;
                col = TAB_WIDTH * ((col / TAB_WIDTH) + 1);
                col_pix = tab_pix * ((col_pix / tab_pix) + 1);
            }
            i += 1;
        }
    }

    /// Read a line and add it to the list. With `skip_blanks` it keeps
    /// reading until it gets one with something on it.
    fn make_line(&mut self, g: &mut Gl, skip_blanks: bool) -> bool {
        let mut s;
        loop {
            match self.get_one_line(g) {
                None => return false,
                Some(t) => {
                    if skip_blanks && t.trim().is_empty() {
                        continue;
                    }
                    s = t;
                    break;
                }
            }
        }
        let width = self.font_scale * self.font.metrics(&s).width as f32;
        let height = self.font_scale * self.line_height as f32;
        s = s.trim_end_matches(['\r', '\n']).to_string();
        self.lines.push(Line {
            text: s,
            width,
            height,
            from: [0.0; 3],
            to: [0.0; 3],
            current: [0.0; 3],
            fth: 0.0,
            tth: 0.0,
            cth: 0.0,
            cluster_size: 0,
            cluster_pos: 0,
            state: State::New,
            step: 0,
            steps: 0,
            color: self.color,
        });
        true
    }

    /// One step of a line's journey.
    fn tick_line(&self, line: &mut Line) {
        let mut stagger = 30; // Frames of delay between the lines of a cluster.
        let slide = 600; // Frames in a slide in or out.
        let linger = 0; // Frames to pause with no motion.

        line.step += 1;
        if line.step >= line.steps {
            line.state = line.state.next();
            line.step = 0;
            if linger == 0 && line.state == State::Linger {
                line.state = line.state.next();
            }
            if self.anim_type != Anim::Spin {
                stagger *= 2;
            }

            match line.state {
                State::Hesitate => {
                    line.steps = match self.anim_type {
                        Anim::Spin => line.cluster_pos * stagger,
                        Anim::ScrollTop => stagger * (line.cluster_size - line.cluster_pos),
                        Anim::ScrollBottom => stagger * line.cluster_pos,
                    };
                }
                State::In => {
                    line.color[3] = 0.0;
                    let mid_off =
                        line.height * ((line.cluster_size as f32 / 2.0) - line.cluster_pos as f32);
                    match self.anim_type {
                        Anim::ScrollBottom => {
                            line.from = self.in_pos;
                            line.to = self.mid;
                            line.from[1] =
                                self.bottom_margin - (line.height * (line.cluster_pos + 1) as f32);
                            line.to[1] += mid_off;
                        }
                        Anim::ScrollTop => {
                            line.from = self.in_pos;
                            line.to = self.mid;
                            line.from[1] = self.top_margin
                                + (line.height * (line.cluster_size - line.cluster_pos) as f32);
                            line.to[1] += mid_off;
                        }
                        Anim::Spin => {
                            line.from = self.in_pos;
                            line.to = self.mid;
                            line.to[1] += mid_off;
                            line.from[1] += mid_off;
                            line.fth = 270.0;
                            line.tth = 0.0;
                        }
                    }
                    line.steps = slide;
                }
                State::Out => {
                    line.from = line.to;
                    line.to = self.out;
                    match self.anim_type {
                        Anim::ScrollBottom => {
                            line.to[1] = self.top_margin
                                + (line.height * (line.cluster_size - line.cluster_pos) as f32);
                        }
                        Anim::ScrollTop => {
                            line.to[1] =
                                self.bottom_margin - (line.height * (line.cluster_pos + 1) as f32);
                        }
                        Anim::Spin => {
                            line.to[1] += line.height
                                * ((line.cluster_size as f32 / 2.0) - line.cluster_pos as f32);
                            line.fth = line.tth;
                            line.tth = -270.0;
                        }
                    }
                    line.steps = slide;
                }
                State::Linger => {
                    line.from = line.to;
                    line.steps = linger;
                }
                _ => {}
            }
            line.steps = (line.steps as f32 / self.speed) as i32;
        }

        if line.state == State::In || line.state == State::Out {
            let i = line.step as f32 / line.steps.max(1) as f32;
            // Along the path exponentially, slow side toward the middle.
            let mut ii = if line.state == State::Out {
                i * i
            } else {
                1.0 - ((1.0 - i) * (1.0 - i))
            };
            for k in 0..3 {
                line.current[k] = line.from[k] + (ii * (line.to[k] - line.from[k]));
            }
            line.cth = line.fth + (ii * (line.tth - line.fth));
            if line.state == State::Out {
                ii = 1.0 - ii;
            }
            line.color[3] = self.color[3] * ii;
        }
    }

    /// Start a new cluster: pick how it moves and where it comes from, goes
    /// to and rests.
    fn reset_lines(&mut self, g: &mut Gl) {
        self.rotation = [
            5.0 - bellrand(10.0),
            5.0 - bellrand(10.0),
            5.0 - bellrand(10.0),
        ];
        self.anim_type = match random() % 8 {
            0 => Anim::ScrollTop,
            1 => Anim::ScrollBottom,
            _ => Anim::Spin,
        };

        let mut minx = self.left_margin * 0.9;
        let mut maxx = self.right_margin * 0.9;
        let mut miny = self.bottom_margin * 0.9;
        let mut maxy = self.top_margin * 0.9;
        let minz = self.left_margin * 5.0;
        let maxz = self.right_margin * 2.0;

        let mut maxw = self.font_wrap_pixels as f32 * self.font_scale;
        let mut maxh = self.max_lines as f32 * self.line_height as f32 * self.font_scale;
        maxw = maxw.min(maxx - minx);
        maxh = maxh.min(maxy - miny);

        if self.alignment_random {
            self.alignment = (random() % 3) as i32 - 1;
        }
        match self.alignment {
            -1 => maxx -= maxw,
            1 => minx += maxw,
            _ => {
                minx += maxw / 2.0;
                maxx -= maxw / 2.0;
            }
        }
        miny += maxh / 2.0;
        maxy -= maxh / 2.0;

        self.mid[0] = minx + frand((maxx - minx) as f64) as f32;
        self.mid[1] = if self.anim_type == Anim::Spin {
            miny + bellrand((maxy - miny) as f64)
        } else {
            miny + frand((maxy - miny) as f64) as f32
        };
        self.mid[2] = 0.0;

        self.in_pos[0] = bellrand((self.right_margin * 2.0) as f64) - self.right_margin;
        self.out[0] = bellrand((self.right_margin * 2.0) as f64) - self.right_margin;
        self.in_pos[1] = miny + frand((maxy - miny) as f64) as f32;
        self.out[1] = miny + frand((maxy - miny) as f64) as f32;
        self.in_pos[2] = minz + frand((maxz - minz) as f64) as f32;
        self.out[2] = minz + frand((maxz - minz) as f64) as f32;

        if self.anim_type == Anim::Spin && self.in_pos[2] > 0.0 {
            self.in_pos[2] /= 4.0;
        }
        if self.anim_type == Anim::Spin && self.out[2] > 0.0 {
            self.out[2] /= 4.0;
        }

        for i in 0..self.max_lines {
            if !self.make_line(g, i == 0) {
                break; // No text available.
            }
            if i >= self.min_lines && self.lines.last().is_some_and(|l| l.text.is_empty()) {
                break; // Blank after the minimum.
            }
        }

        let bottom = self.bottom_margin;
        let n = self.lines.len() as i32;
        let mut prev: Option<(f32, f32, f32)> = None;
        for (i, line) in self.lines.iter_mut().enumerate() {
            match prev {
                None => {
                    line.from[1] = bottom;
                    line.to[1] = 0.0;
                }
                Some((fy, ty, h)) => {
                    line.from[1] = fy - h;
                    line.to[1] = ty - h;
                }
            }
            line.cluster_pos = i as i32;
            line.cluster_size = n;
            prev = Some((line.from[1], line.to[1], line.height));
        }
    }
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let font = TexFont::load(&mut g.glx, g.res.string("font"));
    let m = font.metrics("n");
    let char_width = m.width;
    let line_height = m.ascent + m.descent;

    // The default font is, by fiat, eighteen points. A size asked for is
    // relative to that.
    let font_scale = 3.0 * (g.res.float("fontSize") as f32 / 18.0);
    let target_columns = g.res.int("columns").max(2);

    // The wrap column in font units, from the column count but never wider
    // than the screen.
    let maxw = (110 * line_height) as f32 / font_scale;
    let mut font_wrap_pixels = target_columns * char_width;
    if font_wrap_pixels as f32 > maxw || font_wrap_pixels <= 0 {
        font_wrap_pixels = maxw as i32;
    }

    let max_lines = g.res.int("lines").max(1);
    let mut min_lines = (max_lines as f32 * 0.66) as i32;
    if min_lines > max_lines - 3 {
        min_lines = max_lines - 4;
    }
    min_lines = min_lines.max(1);

    let align = g.res.string("alignment").to_ascii_lowercase();
    let (alignment, alignment_random) = match align.as_str() {
        "center" | "middle" => (0, false),
        "right" => (1, false),
        "random" => (-1, true),
        _ => (-1, false),
    };

    let top_margin = (char_width * 100) as f32;
    let mut this = FlipText {
        lines: Vec::new(),
        buf: String::new(),
        buf_size: (target_columns * max_lines) as usize,
        font,
        char_width,
        line_height,
        font_scale,
        font_wrap_pixels,
        top_margin,
        bottom_margin: -top_margin,
        left_margin: -top_margin,
        right_margin: top_margin,
        anim_type: Anim::Spin,
        in_pos: [0.0; 3],
        mid: [0.0; 3],
        out: [0.0; 3],
        rotation: [0.0; 3],
        color: resource_color(g, "foreground"),
        alignment,
        alignment_random,
        max_lines,
        min_lines,
        speed: g.res.float("speed") as f32,
        aspect: 1.0,
        wire: g.res.bool("wireframe"),
    };

    g.text_reshape(target_columns, 0);
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for FlipText {
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
        self.right_margin = self.top_margin * self.aspect;
        self.left_margin = -self.right_margin;
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(60.0, self.aspect, 0.01, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 2.6], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.clear();
        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.lighting(false);
        if !self.wire {
            g.glx.blend(Blend::Alpha);
        }

        g.glx.push_matrix();
        let s = 3.0 / (self.top_margin - self.bottom_margin);
        g.glx.scale(s, s, s);
        g.glx.rotate(self.rotation[0], 1.0, 0.0, 0.0);
        g.glx.rotate(self.rotation[1], 0.0, 1.0, 0.0);
        g.glx.rotate(self.rotation[2], 0.0, 0.0, 1.0);

        for i in 0..self.lines.len() {
            let line = &self.lines[i];
            if !line.text.is_empty()
                && line.state != State::New
                && line.state != State::Hesitate
                && line.state != State::Dead
            {
                let (c, w, cur, cth) = (line.color, line.width, line.current, line.cth);
                g.glx.push_matrix();
                g.glx.translate(cur[0], cur[1], cur[2]);
                g.glx.rotate(cth, 0.0, 1.0, 0.0);
                if self.alignment == 1 {
                    g.glx.translate(-w, 0.0, 0.0);
                } else if self.alignment == 0 {
                    g.glx.translate(-w / 2.0, 0.0, 0.0);
                }
                g.glx
                    .scale(self.font_scale, self.font_scale, self.font_scale);
                g.glx.color4f(c[0], c[1], c[2], c[3]);
                let glx = &mut g.glx;
                self.font.print_string(glx, &self.lines[i].text);
                g.glx.pop_matrix();
            }

            let mut line = std::mem::replace(
                &mut self.lines[i],
                Line {
                    text: String::new(),
                    width: 0.0,
                    height: 0.0,
                    from: [0.0; 3],
                    to: [0.0; 3],
                    current: [0.0; 3],
                    fth: 0.0,
                    tth: 0.0,
                    cth: 0.0,
                    cluster_size: 0,
                    cluster_pos: 0,
                    state: State::Dead,
                    step: 0,
                    steps: 0,
                    color: [0.0; 4],
                },
            );
            self.tick_line(&mut line);
            self.lines[i] = line;
        }

        self.lines.retain(|l| l.state != State::Dead);
        if self.lines.is_empty() {
            self.reset_lines(g);
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       10000",
    "*showFPS:     False",
    "*wireframe:   False",
    "*usePty:      False",
    "*font:        sans-serif bold 72",
    ".foreground:  #00CCFF",
    "*lines:       8",
    "*fontSize:    20",
    "*columns:     80",
    "*alignment:   random",
    "*speed:       1.0",
];

const ALIGNMENTS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "random",
        label: "Random text alignment",
    },
    crate::runtime::opts::SelectItem {
        value: "left",
        label: "Flush left text",
    },
    crate::runtime::opts::SelectItem {
        value: "center",
        label: "Centered text",
    },
    crate::runtime::opts::SelectItem {
        value: "right",
        label: "Flush right text",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::select("alignment", "Alignment", ALIGNMENTS, "random"),
    Opt::slider("fontSize", "Font point size", 4.0, 60.0, 1.0, 0, "20"),
    Opt::slider("columns", "Text columns", 2.0, 200.0, 1.0, 0, "80"),
    Opt::slider("lines", "Text lines", 1.0, 50.0, 1.0, 0, "8"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "fliptext",
    label: "Flip Text",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=vcB-6S7Hfuk"),
        blurb: "Lines of text flying in and out again, spinning or scrolling, \
                a clusterful at a time.",
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

    /// A line goes through its states in order and then dies, and it is only
    /// drawn for the middle of them.
    #[test]
    fn a_line_lives_and_dies() {
        assert_eq!(State::New.next(), State::Hesitate);
        assert_eq!(State::Hesitate.next(), State::In);
        assert_eq!(State::In.next(), State::Linger);
        assert_eq!(State::Linger.next(), State::Out);
        assert_eq!(State::Out.next(), State::Dead);
        assert_eq!(State::Dead.next(), State::Dead);
    }

    /// Text arrives, flies in and eventually flies out again, and the cluster
    /// is replaced by another.
    #[test]
    fn the_text_comes_and_goes() {
        let mut r = start(StartArgs::new(640, 480, "speed=8", 20260811));
        let mut drew = 0;
        for _ in 0..900 {
            r.step();
            if !r.frame().vertices.is_empty() {
                drew += 1;
            }
        }
        assert!(drew > 50, "the text was only on screen {drew} frames");
        let f = r.frame();
        assert!(
            f.batches.iter().all(|b| !b.depth_test),
            "the text is depth tested"
        );
    }

    /// A line fades in as it slows down: its opacity and its position follow
    /// the same curve.
    #[test]
    fn it_fades_in_as_it_arrives() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "speed=8&alignment=center",
            20260811,
        ));
        let mut alphas: Vec<f32> = Vec::new();
        for _ in 0..900 {
            r.step();
            let f = r.frame();
            if let Some(v) = f.vertices.first() {
                alphas.push(v.color[3]);
            }
        }
        assert!(!alphas.is_empty(), "nothing was ever drawn");
        let lo = alphas.iter().copied().fold(f32::MAX, f32::min);
        let hi = alphas.iter().copied().fold(f32::MIN, f32::max);
        assert!(lo < 0.5, "it never faded in: {lo}");
        assert!(hi > 0.5, "it never got solid: {hi}");
    }

    /// Tabs become spaces to the next multiple of eight, and a backspace eats
    /// the character before it.
    #[test]
    fn tabs_are_bad_mmmkay() {
        assert_eq!(untabify("a\tb"), "a       b");
        assert_eq!(untabify("ab\u{8}c"), "ac");
    }
}
