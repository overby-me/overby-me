//! Port of `hacks/glx/cube21.c`.
//!
//! ```text
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! Cube 21 - a Rubik-like puzzle.  It changes its shape and has more than
//! 200 configurations.  It is known better as Square-1, but it is called
//! Cube 21 in the Czech republic, where it was invented in 1992.
//!
//! This file is derived from cage.c,
//! "cage --- the Impossible Cage, an Escher like scene",
//! by Marcelo F. Vienna,
//! parts from gltext.c by Jamie Zawinski
//!
//! Vaclav (Vasek) Potocek
//! vasek.potocek@post.cz
//! ```
//!
//! Square-1, shuffling itself. Two faces of twelve thirty-degree slots each,
//! filled with a mixture of narrow pieces one slot wide and wide pieces two
//! slots wide, and a middle that can be flipped end over end.
//!
//! The interesting part is that a face can only be turned where a cut runs all
//! the way through: a wide piece straddling the seam blocks the turn. So
//! `find_matches` walks the ring looking for offsets where *both* halves have
//! a piece boundary, and the shuffle only ever picks one of those. That is why
//! it never tears a piece in half, and it is the whole of the puzzle's
//! mechanics in twenty lines.
//!
//! There is no board here either. The pieces are two rings of twelve slots
//! holding a flag saying whether a piece starts there, and turning a face is a
//! rotation of that ring and of the colour rings alongside it. Flipping the
//! halves swaps six slots of one ring with six reversed slots of the other.
//!
//! All the coordinates come out of a handful of tangents and cosines of
//! fifteen and thirty degrees, computed once, because the edges of adjacent
//! pieces have to line up exactly or the seams show.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, SelectItem, StartArgs, Trackball, XEvent, frand,
    random,
};

const SHUFFLE: usize = 100;

const COS15: f32 = 0.965_925_8;
const SIN15: f32 = 0.258_819_04;
const COS30: f32 = 0.866_025_4;
const SIN30: f32 = 0.5;

const TEX_WIDTH: i32 = 128;
const TEX_HEIGHT: i32 = 128;
/// Where on the texture the flat grey of an inner face is taken from.
const TEX_GRAY: [f32; 2] = [0.7, 0.7];
const BORDER: i32 = 3;
const BORDER2: i32 = 9;

const ZPOS: f32 = -18.0;

/// What the puzzle is doing. A turn of one face, or a flip of the halves,
/// with a pause on either side.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pause1,
    RotTop,
    RotBottom,
    Pause2,
    Half1,
    Half2,
}

/// How the faces are coloured.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorMode {
    White,
    Random,
    Silver,
    TwoRnd,
    Classic,
    SixRnd,
}

fn rnd01() -> i32 {
    (random() % 2) as i32
}

fn rndcolor() -> f32 {
    frand(0.5) as f32 + 0.3
}

/// The offsets at which this face can be turned: the ones where both halves
/// have a piece boundary, so no wide piece straddles the cut.
fn find_matches(pieces: &[[i32; 13]; 2], s: usize) -> Vec<i32> {
    let mut out = Vec::with_capacity(12);
    for i in 1..6 {
        if pieces[s][i] != 0 && pieces[s][i + 6] != 0 {
            out.push(i as i32);
        }
    }
    let n = out.len();
    for k in 0..n {
        out.push(out[k] - 6);
    }
    out.push(6);
    out
}

/// Turn one face by `o` slots, carrying its two colour rings with it.
fn rot_face(pieces: &mut [[i32; 13]; 2], colors: &mut [[i32; 12]; 5], s: usize, o: i32) {
    let c0 = 2 * s;
    let c1 = c0 + 1;
    let tmp = pieces[s];
    let tmpc = [colors[c0], colors[c1]];
    let mut o = if o < 0 { o + 12 } else { o } as usize;
    for i in 0..12 {
        if o == 12 {
            o = 0;
        }
        pieces[s][i] = tmp[o];
        colors[c0][i] = tmpc[0][o];
        colors[c1][i] = tmpc[1][o];
        o += 1;
    }
}

/// Flip one half of the puzzle end over end, which swaps six slots of the top
/// ring with six reversed slots of the bottom.
fn rot_halves(
    pieces: &mut [[i32; 13]; 2],
    colors: &mut [[i32; 12]; 5],
    hf: &mut [i32; 2],
    s: usize,
) {
    let ss = 6 * s;
    for i in 0..6 {
        let j = ss + i;
        let k = ss + 6 - i;
        let t = pieces[0][j];
        pieces[0][j] = pieces[1][k];
        pieces[1][k] = t;
        let k = k - 1;
        let t = colors[0][j];
        colors[0][j] = colors[2][k];
        colors[2][k] = t;
        let t = colors[1][j];
        colors[1][j] = colors[3][k];
        colors[3][k] = t;
    }
    hf[s] ^= 1;
}

struct Cube21 {
    trackball: Trackball,
    ratio: f32,
    state: State,
    /// The object's own spin, and where it has wandered to.
    xrot: f32,
    yrot: f32,
    posarg: f32,
    /// The clock for the move, and what it is counting to.
    t: f32,
    tmax: f32,
    /// Which halves are flipped, and which faces have been turned this move.
    hf: [i32; 2],
    fr: [i32; 2],
    /// The face being turned and by how many thirty-degree slots.
    rface: usize,
    ramount: i32,
    /// Where the narrow and wide pieces are, and what colour each face is.
    pieces: [[i32; 13]; 2],
    cind: [[i32; 12]; 5],
    colors: [[f32; 3]; 6],

    cmat: bool,
    /// The tangent-derived coordinates every piece is built from.
    texp: f32,
    texq: f32,
    posc: [f32; 6],
    color_inner: [f32; 4],
    texid: u32,

    spin: bool,
    wander: bool,
    spinspeed: f32,
    tspeed: f32,
    wspeed: f32,
    twait: f32,
    size: f32,
}

impl Cube21 {
    fn randomize(&mut self) {
        for _ in 0..SHUFFLE {
            let s = rnd01() as usize;
            let matches = find_matches(&self.pieces, s);
            let j = matches[random() as usize % matches.len()];
            rot_face(&mut self.pieces, &mut self.cind, s, j);
            let s = rnd01() as usize;
            rot_halves(&mut self.pieces, &mut self.cind, &mut self.hf, s);
        }
    }

    /// Pick a face and an offset that the puzzle will actually allow.
    fn pick_turn(&mut self, s: usize) {
        let matches = find_matches(&self.pieces, s);
        let mut j = matches[random() as usize % matches.len()];
        if j == 6 && rnd01() != 0 {
            j = -6;
        }
        self.state = if s == 0 {
            State::RotTop
        } else {
            State::RotBottom
        };
        self.tmax = 30.0 * j.abs() as f32;
        self.rface = s;
        self.ramount = j;
    }

    fn finish(&mut self) {
        match self.state {
            State::Pause1 => {
                let s = rnd01() as usize;
                self.pick_turn(s);
                self.fr = [0, 0];
            }
            State::RotTop | State::RotBottom => {
                let s = self.rface;
                rot_face(&mut self.pieces, &mut self.cind, s, self.ramount);
                self.fr[s] = 1;
                let s = s ^ 1;
                // Sometimes turn the other face too before pausing, which is
                // what makes a move sometimes look like one gesture and
                // sometimes like two.
                if self.fr[s] == 0 && rnd01() != 0 {
                    self.pick_turn(s);
                } else {
                    self.state = State::Pause2;
                    self.tmax = self.twait;
                }
            }
            State::Pause2 => {
                let s = rnd01() as usize;
                /* 0 or -1, only sign is significant in this case */
                self.ramount = -rnd01();
                self.state = if s == 0 { State::Half1 } else { State::Half2 };
                self.tmax = 180.0;
                self.rface = s;
            }
            State::Half1 | State::Half2 => {
                rot_halves(&mut self.pieces, &mut self.cind, &mut self.hf, self.rface);
                self.state = State::Pause1;
                self.tmax = self.twait;
            }
        }
        self.t = 0.0;
    }

    fn color(&self, c: usize) -> [f32; 4] {
        let c = self.colors[c.min(5)];
        [c[0], c[1], c[2], 1.0]
    }

    /// Set the material, if the colour mode says the faces are coloured at
    /// all. With lighting on it is the material that is shaded, so upstream's
    /// `glColor` under `GL_COLOR_MATERIAL` comes here.
    fn set(&self, g: &mut Gl, c: [f32; 4]) {
        if self.cmat {
            g.glx.material_ambient_diffuse(c);
        }
    }

    /// A piece one slot wide: a wedge of thirty degrees.
    fn draw_narrow_piece(&self, g: &mut Gl, s: f32, c1: usize, c2: usize) {
        let p = self.posc;
        let s1 = p[0] * s;
        let gray = TEX_GRAY;

        g.glx.begin(Shape::Triangles);
        g.glx.normal3f(0.0, 0.0, s);
        self.set(g, self.color(c1));
        for (u, v, x, y, z) in [
            (0.5, 0.5, 0.0, 0.0, s),
            (self.texq, 0.0, p[1], 0.0, s),
            (self.texp, 0.0, p[2], p[3], s),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(0.0, 0.0, -s);
        self.set(g, self.color_inner);
        g.glx.tex_coord2f(gray[0], gray[1]);
        for (x, y, z) in [(0.0, 0.0, s1), (p[1], 0.0, s1), (p[2], p[3], s1)] {
            g.glx.vertex3f(x, y, z);
        }
        g.glx.end();

        g.glx.begin(Shape::Quads);
        g.glx.normal3f(0.0, -1.0, 0.0);
        self.set(g, self.color_inner);
        g.glx.tex_coord2f(gray[0], gray[1]);
        for (x, y, z) in [
            (0.0, 0.0, s),
            (p[1], 0.0, s),
            (p[1], 0.0, s1),
            (0.0, 0.0, s1),
        ] {
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(COS15, SIN15, 0.0);
        self.set(g, self.color(c2));
        for (u, v, x, y, z) in [
            (self.texq, self.texq, p[1], 0.0, s),
            (self.texq, self.texp, p[2], p[3], s),
            (1.0, self.texp, p[2], p[3], s1),
            (1.0, self.texq, p[1], 0.0, s1),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(-SIN30, COS30, 0.0);
        self.set(g, self.color_inner);
        g.glx.tex_coord2f(gray[0], gray[1]);
        for (x, y, z) in [
            (0.0, 0.0, s),
            (p[2], p[3], s),
            (p[2], p[3], s1),
            (0.0, 0.0, s1),
        ] {
            g.glx.vertex3f(x, y, z);
        }
        g.glx.end();
        g.glx.rotate(30.0, 0.0, 0.0, 1.0);
    }

    /// A piece two slots wide, which is what blocks a turn when it straddles
    /// the seam.
    fn draw_wide_piece(&self, g: &mut Gl, s: f32, c1: usize, c2: usize, c3: usize) {
        let p = self.posc;
        let s1 = p[0] * s;
        let gray = TEX_GRAY;

        g.glx.begin(Shape::Triangles);
        g.glx.normal3f(0.0, 0.0, s);
        self.set(g, self.color(c1));
        for (u, v, x, y, z) in [
            (0.5, 0.5, 0.0, 0.0, s),
            (self.texp, 0.0, p[1], 0.0, s),
            (0.0, 0.0, p[4], p[5], s),
            (0.0, 0.0, p[4], p[5], s),
            (0.0, self.texp, p[3], p[2], s),
            (0.5, 0.5, 0.0, 0.0, s),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(0.0, 0.0, -s);
        self.set(g, self.color_inner);
        g.glx.tex_coord2f(gray[0], gray[1]);
        for (x, y, z) in [
            (0.0, 0.0, s1),
            (p[1], 0.0, s1),
            (p[4], p[5], s1),
            (p[4], p[5], s1),
            (p[3], p[2], s1),
            (0.0, 0.0, s1),
        ] {
            g.glx.vertex3f(x, y, z);
        }
        g.glx.end();

        g.glx.begin(Shape::Quads);
        g.glx.normal3f(0.0, -1.0, 0.0);
        self.set(g, self.color_inner);
        g.glx.tex_coord2f(gray[0], gray[1]);
        for (x, y, z) in [
            (0.0, 0.0, s),
            (p[1], 0.0, s),
            (p[1], 0.0, s1),
            (0.0, 0.0, s1),
        ] {
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(COS15, -SIN15, 0.0);
        self.set(g, self.color(c2));
        for (u, v, x, y, z) in [
            (self.texq, self.texp, p[1], 0.0, s),
            (self.texq, 0.0, p[4], p[5], s),
            (1.0, 0.0, p[4], p[5], s1),
            (1.0, self.texp, p[1], 0.0, s1),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(SIN15, COS15, 0.0);
        self.set(g, self.color(c3));
        for (u, v, x, y, z) in [
            (self.texq, self.texp, p[4], p[5], s),
            (self.texq, 0.0, p[3], p[2], s),
            (1.0, 0.0, p[3], p[2], s1),
            (1.0, self.texp, p[4], p[5], s1),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(-COS30, SIN30, 0.0);
        self.set(g, self.color_inner);
        g.glx.tex_coord2f(gray[0], gray[1]);
        for (x, y, z) in [
            (0.0, 0.0, s),
            (p[3], p[2], s),
            (p[3], p[2], s1),
            (0.0, 0.0, s1),
        ] {
            g.glx.vertex3f(x, y, z);
        }
        g.glx.end();
        g.glx.rotate(60.0, 0.0, 0.0, 1.0);
    }

    fn draw_middle_piece(&self, g: &mut Gl, s: usize) {
        let p = self.posc;
        let s = s * 6;
        let gray = TEX_GRAY;

        g.glx.begin(Shape::Quads);
        self.set(g, self.color_inner);
        g.glx.normal3f(0.0, 0.0, 1.0);
        g.glx.tex_coord2f(gray[0], gray[1]);
        for (x, y, z) in [
            (p[1], 0.0, p[0]),
            (p[4], p[5], p[0]),
            (-p[5], p[4], p[0]),
            (-p[1], 0.0, p[0]),
        ] {
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(0.0, 0.0, -1.0);
        g.glx.tex_coord2f(gray[0], gray[1]);
        for (x, y, z) in [
            (p[1], 0.0, -p[0]),
            (p[4], p[5], -p[0]),
            (-p[5], p[4], -p[0]),
            (-p[1], 0.0, -p[0]),
        ] {
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(0.0, -1.0, 0.0);
        g.glx.tex_coord2f(gray[0], gray[1]);
        for (x, y, z) in [
            (-p[1], 0.0, p[0]),
            (p[1], 0.0, p[0]),
            (p[1], 0.0, -p[0]),
            (-p[1], 0.0, -p[0]),
        ] {
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(COS15, -SIN15, 0.0);
        self.set(g, self.color(self.cind[4][s] as usize));
        for (u, v, x, y, z) in [
            (self.texq, self.texp, p[1], 0.0, p[0]),
            (1.0, self.texp, p[4], p[5], p[0]),
            (1.0, self.texq, p[4], p[5], -p[0]),
            (self.texq, self.texq, p[1], 0.0, -p[0]),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(SIN15, COS15, 0.0);
        self.set(g, self.color(self.cind[4][s + 1] as usize));
        for (u, v, x, y, z) in [
            (0.0, 0.5, p[4], p[5], p[0]),
            (self.texq, 0.5, -p[5], p[4], p[0]),
            (self.texq, 0.75, -p[5], p[4], -p[0]),
            (0.0, 0.75, p[4], p[5], -p[0]),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.normal3f(-COS15, SIN15, 0.0);
        self.set(g, self.color(self.cind[4][s + 4] as usize));
        for (u, v, x, y, z) in [
            (0.0, 0.75, -p[5], p[4], p[0]),
            (1.0, 0.75, -p[1], 0.0, p[0]),
            (1.0, 1.0, -p[1], 0.0, -p[0]),
            (0.0, 1.0, -p[5], p[4], -p[0]),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.end();
    }

    fn draw_middle(&self, g: &mut Gl) {
        if self.hf[0] != 0 {
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        }
        self.draw_middle_piece(g, 0);
        if self.hf[0] != 0 {
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        }
        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        if self.hf[1] != 0 {
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        }
        self.draw_middle_piece(g, 1);
        if self.hf[1] != 0 {
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        }
    }

    /// Walk half a face's six slots, drawing whichever piece starts at each.
    fn draw_half_face(&self, g: &mut Gl, s: usize, o: usize) {
        let s1 = 1.0 - s as f32 * 2.0;
        let s2 = s * 2;
        let mut i = o;
        while i < o + 6 {
            if self.pieces[s][i + 1] != 0 {
                self.draw_narrow_piece(
                    g,
                    s1,
                    self.cind[s2][i] as usize,
                    self.cind[s2 + 1][i] as usize,
                );
            } else {
                self.draw_wide_piece(
                    g,
                    s1,
                    self.cind[s2][i] as usize,
                    self.cind[s2 + 1][i] as usize,
                    self.cind[s2 + 1][i + 1] as usize,
                );
                i += 1;
            }
            i += 1;
        }
    }

    fn draw_top_face(&self, g: &mut Gl) {
        self.draw_half_face(g, 0, 0);
        self.draw_half_face(g, 0, 6);
    }

    fn draw_bottom_face(&self, g: &mut Gl) {
        self.draw_half_face(g, 1, 0);
        self.draw_half_face(g, 1, 6);
    }
}

/// The face texture: white, with dark lines along the edges of every facet a
/// piece can present, so the seams read even when the whole puzzle is one
/// colour.
fn make_texture(g: &mut Gl, texp: f32, texq: f32) -> u32 {
    let mut tex = vec![255u8; (TEX_WIDTH * TEX_HEIGHT) as usize];
    let at = |x: i32, y: i32| (y * TEX_WIDTH + x) as usize;
    let darken = |tex: &mut Vec<u8>, x: i32, y: i32, w: i32| {
        if x < 0 || y < 0 || x >= TEX_WIDTH || y >= TEX_HEIGHT {
            return;
        }
        let w = w.clamp(0, 255) as u8;
        let i = at(x, y);
        if tex[i] > w {
            tex[i] = w;
        }
    };

    let horz_line = |tex: &mut Vec<u8>, x1: i32, x2: i32, y0: i32| {
        let mut y = if y0 < BORDER { -y0 } else { -BORDER };
        while y < BORDER {
            if y0 + y >= TEX_HEIGHT {
                break;
            }
            let w = y * y * 255 / BORDER2;
            for x in x1..=x2 {
                darken(tex, x, y0 + y, w);
            }
            y += 1;
        }
    };
    let vert_line = |tex: &mut Vec<u8>, x0: i32, y1: i32, y2: i32| {
        let mut x = if x0 < BORDER { -x0 } else { -BORDER };
        while x < BORDER {
            if x0 + x >= TEX_WIDTH {
                break;
            }
            let w = x * x * 255 / BORDER2;
            for y in y1..=y2 {
                darken(tex, x0 + x, y, w);
            }
            x += 1;
        }
    };
    // The diagonal edges, where a piece's outer corner is cut off. Upstream
    // walks a pixel either side of the line and weights by the square of the
    // distance from it, which is what makes them as soft as the straight ones.
    let slanted_horz = |tex: &mut Vec<u8>, x1: i32, y1: i32, x2: i32, y2: i32| {
        let (dx, dy) = (x2 - x1, y2 - y1);
        if dx == 0 {
            return;
        }
        for x in x1..=x2 {
            let y0 = y1 + (y2 - y1) * (x - x1) / (x2 - x1);
            for y in -1 - BORDER..2 + BORDER {
                let w = dx * (y0 + y - y1) - dy * (x - x1);
                let w = w * w / (dx * dx + dy * dy);
                darken(tex, x, y0 + y, w * 255 / BORDER2);
            }
        }
    };
    let slanted_vert = |tex: &mut Vec<u8>, x1: i32, y1: i32, x2: i32, y2: i32| {
        let (dx, dy) = (x2 - x1, y2 - y1);
        if dy == 0 {
            return;
        }
        for y in y1..=y2 {
            let x0 = x1 + (x2 - x1) * (y - y1) / (y2 - y1);
            for x in -1 - BORDER..2 + BORDER {
                let w = dy * (x0 + x - x1) - dx * (y - y1);
                let w = w * w / (dy * dy + dx * dx);
                darken(tex, x0 + x, y, w * 255 / BORDER2);
            }
        }
    };

    let qw = (texq * TEX_WIDTH as f32) as i32;
    let qh = (texq * TEX_HEIGHT as f32) as i32;
    let pw = (texp * TEX_WIDTH as f32) as i32;
    let ph = (texp * TEX_HEIGHT as f32) as i32;

    horz_line(&mut tex, 0, TEX_WIDTH - 1, 0);
    horz_line(&mut tex, qw, TEX_WIDTH - 1, ph);
    horz_line(&mut tex, qw, TEX_WIDTH - 1, qh);
    horz_line(&mut tex, 0, qw, TEX_HEIGHT / 2);
    horz_line(&mut tex, 0, TEX_WIDTH - 1, TEX_HEIGHT * 3 / 4);
    horz_line(&mut tex, 0, TEX_WIDTH - 1, TEX_HEIGHT - 1);
    vert_line(&mut tex, 0, 0, TEX_HEIGHT - 1);
    vert_line(&mut tex, qw, 0, TEX_HEIGHT * 3 / 4);
    vert_line(&mut tex, TEX_WIDTH - 1, 0, TEX_HEIGHT - 1);
    slanted_horz(&mut tex, 0, ph, TEX_WIDTH / 2, TEX_HEIGHT / 2);
    slanted_vert(&mut tex, pw, 0, TEX_WIDTH / 2, TEX_HEIGHT / 2);
    slanted_vert(&mut tex, qw, 0, TEX_WIDTH / 2, TEX_HEIGHT / 2);

    // The one dark speck the inner faces sample, so they come out flat grey.
    let x0 = (TEX_GRAY[0] * TEX_WIDTH as f32) as i32;
    let y0 = (TEX_GRAY[1] * TEX_HEIGHT as f32) as i32;
    for y in -1..=1 {
        for x in -1..=1 {
            tex[at(x0 + x, y0 + y)] = 100;
        }
    }

    // Upstream's is GL_LUMINANCE, which the fixed pipeline reads as that value
    // in all three colour channels and an opaque alpha.
    let data: Vec<u8> = tex.iter().flat_map(|&l| [l, l, l, 255]).collect();
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    g.glx.tex_clamp(true);
    g.glx.tex_image_2d(TEX_WIDTH, TEX_HEIGHT, data);
    id
}

impl Hack3d for Cube21 {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let theta = if self.ramount < 0 { self.t } else { -self.t };
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.load_identity();

        if self.wander {
            g.glx.translate(
                3.0 * self.ratio * (13.0 * self.posarg).sin(),
                3.0 * (17.0 * self.posarg).sin(),
                ZPOS,
            );
        } else {
            g.glx.translate(0.0, 0.0, ZPOS);
        }
        g.glx.scale(self.size, self.size, self.size);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        g.glx.rotate(self.xrot, 1.0, 0.0, 0.0);
        g.glx.rotate(self.yrot, 0.0, 1.0, 0.0);

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        match self.state {
            State::Pause1 | State::Pause2 => {
                self.draw_top_face(g);
                self.draw_bottom_face(g);
                self.draw_middle(g);
            }
            State::RotTop => {
                g.glx.rotate(theta, 0.0, 0.0, 1.0);
                self.draw_top_face(g);
                g.glx.rotate(-theta, 0.0, 0.0, 1.0);
                self.draw_bottom_face(g);
                self.draw_middle(g);
            }
            State::RotBottom => {
                self.draw_top_face(g);
                g.glx.rotate(theta, 0.0, 0.0, 1.0);
                self.draw_bottom_face(g);
                g.glx.rotate(-theta, 0.0, 0.0, 1.0);
                self.draw_middle(g);
            }
            State::Half1 | State::Half2 => {
                // Half of the puzzle swings out of the way; the two states
                // differ only in which side of the flip is turning.
                if self.state == State::Half1 {
                    g.glx.rotate(theta, 0.0, 1.0, 0.0);
                }
                self.draw_half_face(g, 0, 0);
                g.glx.rotate(-180.0, 0.0, 0.0, 1.0);
                self.draw_half_face(g, 1, 0);
                g.glx.rotate(-180.0, 0.0, 0.0, 1.0);
                if self.hf[0] != 0 {
                    g.glx.rotate(180.0, 0.0, 1.0, 0.0);
                }
                self.draw_middle_piece(g, 0);
                if self.hf[0] != 0 {
                    g.glx.rotate(180.0, 0.0, 1.0, 0.0);
                }
                if self.state == State::Half1 {
                    g.glx.rotate(-theta, 0.0, 1.0, 0.0);
                } else {
                    g.glx.rotate(theta, 0.0, 1.0, 0.0);
                }
                g.glx.rotate(180.0, 0.0, 0.0, 1.0);
                self.draw_half_face(g, 0, 6);
                g.glx.rotate(-180.0, 0.0, 0.0, 1.0);
                self.draw_half_face(g, 1, 6);
                g.glx.rotate(-180.0, 0.0, 0.0, 1.0);
                if self.hf[1] != 0 {
                    g.glx.rotate(180.0, 0.0, 1.0, 0.0);
                }
                self.draw_middle_piece(g, 1);
            }
        }

        if self.spin {
            self.xrot += self.spinspeed;
            if self.xrot > 360.0 {
                self.xrot -= 360.0;
            }
            self.yrot += self.spinspeed;
            if self.yrot > 360.0 {
                self.yrot -= 360.0;
            }
        }
        if self.wander {
            self.posarg += self.wspeed / 1000.0;
            if self.posarg > 360.0 {
                self.posarg -= 360.0;
            }
        }
        self.t += self.tspeed;
        if self.t > self.tmax {
            self.finish();
        }

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut y = 0;
        if !height.is_positive() {
            height = 1;
        }
        self.ratio = width as f32 / height as f32;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width;
            y = -height / 2;
            self.ratio = width as f32 / height as f32;
            self.posarg = 0.0;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, self.ratio, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    // Upstream turns wireframe off under GLES, where a line polygon mode does
    // not exist; the same applies here.
    let tex = g.res.bool("texture");
    let colmode = match g.res.string("colormode") {
        s if s.contains("se") || s.contains("sil") => ColorMode::Silver,
        s if s.contains("ce") || s.contains("cla") => ColorMode::Classic,
        s if s.contains('2') || s.contains("two") => ColorMode::TwoRnd,
        s if s.contains('6') || s.contains("six") => ColorMode::SixRnd,
        s if s.contains('1') || s.contains("ran") || s.contains("rnd") => ColorMode::Random,
        _ => ColorMode::White,
    };

    let texp = (1.0 - (std::f32::consts::PI / 12.0).tan()) / 2.0;
    let texq = 1.0 - texp;
    let pi = std::f64::consts::PI;
    // The edges of adjacent pieces have to line up exactly, so these are
    // computed rather than written out.
    let posc = [
        (pi / 12.0).tan() as f32,                      /* 0.268 */
        (1.0 / (pi / 12.0).cos()) as f32,              /* 1.035 */
        ((pi / 6.0).cos() / (pi / 12.0).cos()) as f32, /* 0.897 */
        ((pi / 6.0).sin() / (pi / 12.0).cos()) as f32, /* 0.518 */
        (2.0f64.sqrt() * (pi / 6.0).cos()) as f32,     /* 1.225 */
        (2.0f64.sqrt() * (pi / 6.0).sin()) as f32,     /* 0.707 */
    ];

    let texid = if tex { make_texture(g, texp, texq) } else { 0 };
    let inner = if tex { 1.0 } else { 0.4 };

    let wspeed = g.res.float("wanderspeed") as f32;
    let mut st = Cube21 {
        trackball: Trackball::new(),
        ratio: 1.0,
        state: State::Pause1,
        xrot: -65.0,
        yrot: 185.0,
        posarg: if wspeed != 0.0 {
            (random() % 360) as f32
        } else {
            0.0
        },
        t: 0.0,
        tmax: g.res.float("wait") as f32,
        hf: [0, 0],
        fr: [0, 0],
        rface: 0,
        ramount: 0,
        // Twelve slots a face, plus a thirteenth that only the half-flip
        // ever touches. Upstream never rotates that one, so it keeps the value
        // it was built with; the shape depends on it and it is left alone.
        pieces: [[0; 13]; 2],
        cind: [[0; 12]; 5],
        colors: [[1.0; 3]; 6],
        cmat: colmode != ColorMode::White,
        texp,
        texq,
        posc,
        color_inner: [inner, inner, inner, 1.0],
        texid,
        spin: g.res.bool("spin"),
        wander: g.res.bool("wander"),
        spinspeed: g.res.float("spinspeed") as f32,
        tspeed: g.res.float("rotspeed") as f32,
        wspeed,
        twait: g.res.float("wait") as f32,
        size: g.res.float("cubesize") as f32,
    };

    for i in 0..13 {
        let v = i32::from(i % 3 != 1);
        st.pieces[0][i] = v;
        st.pieces[1][i] = v;
    }

    const CE_COLORS: [[f32; 3]; 6] = [
        [1.0, 1.0, 1.0],
        [1.0, 0.5, 0.0],
        [0.0, 0.9, 0.0],
        [0.8, 0.0, 0.0],
        [0.1, 0.1, 1.0],
        [0.9, 0.9, 0.0],
    ];
    match colmode {
        ColorMode::Random | ColorMode::TwoRnd | ColorMode::SixRnd => {
            for c in &mut st.colors {
                for v in c.iter_mut() {
                    *v = rndcolor();
                }
            }
        }
        ColorMode::Silver => {
            st.colors[0] = [1.0, 1.0, 1.0];
            st.colors[1] = [rndcolor(), rndcolor(), rndcolor()];
        }
        ColorMode::Classic => {
            for (c, ce) in st.colors.iter_mut().zip(CE_COLORS) {
                for k in 0..3 {
                    c[k] = 0.2 + 0.7 * ce[k];
                }
            }
        }
        ColorMode::White => {}
    }
    match colmode {
        ColorMode::Silver | ColorMode::TwoRnd => {
            for (i, row) in st.cind.iter_mut().enumerate() {
                for (j, v) in row.iter_mut().enumerate() {
                    *v = match i {
                        0 => 0,
                        2 => 1,
                        _ => i32::from((j + 5) % 12 >= 6),
                    };
                }
            }
        }
        ColorMode::Classic | ColorMode::SixRnd => {
            for (i, row) in st.cind.iter_mut().enumerate() {
                for (j, v) in row.iter_mut().enumerate() {
                    *v = match i {
                        0 => 4,
                        2 => 5,
                        _ => ((j + 5) % 12 / 3) as i32,
                    };
                }
            }
        }
        _ => {}
    }

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if tex {
        g.glx.texturing(true);
        g.glx.bind_texture(st.texid);
    }
    g.glx.lighting(true);
    g.glx.light_model_ambient([0.1, 0.1, 0.1, 1.0]);
    for (i, pos) in [[1.0, 1.0, 1.0, 0.0], [-1.0, -1.0, 1.0, 0.0]]
        .into_iter()
        .enumerate()
    {
        g.glx.light_enable(i, true);
        g.glx.light_position(i, pos[0], pos[1], pos[2], pos[3]);
        g.glx.light_ambient(i, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(i, [1.0, 1.0, 1.0, 1.0]);
        // Upstream sets no specular, so each light keeps OpenGL's default:
        // white for the first and black for every other.
        let spec = if i == 0 { 1.0 } else { 0.0 };
        g.glx.light_specular(i, [spec, spec, spec, 1.0]);
    }
    // GL_COLOR_MATERIAL is on, so the ambient and diffuse follow the colour a
    // piece sets. White is what a piece that sets none gets.
    g.glx.material_ambient_diffuse([1.0, 1.0, 1.0, 1.0]);
    g.glx.material_specular([0.2, 0.2, 0.2, 1.0]);
    g.glx.material_shininess(20.0);

    if g.res.bool("randomize") {
        st.randomize();
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*spin:         True",
    "*wander:       True",
    "*texture:      True",
    "*randomize:    True",
    "*spinspeed:    1.0",
    "*rotspeed:     3.0",
    "*wanderspeed:  0.02",
    "*wait:         40.0",
    "*cubesize:     0.7",
    "*colormode:    six",
];

const STARTS: &[SelectItem] = &[
    SelectItem {
        value: "true",
        label: "Start as random shape",
    },
    SelectItem {
        value: "false",
        label: "Start as cube",
    },
];

const COLORS: &[SelectItem] = &[
    SelectItem {
        value: "six",
        label: "Six random colors",
    },
    SelectItem {
        value: "white",
        label: "White",
    },
    SelectItem {
        value: "rnd",
        label: "Random color",
    },
    SelectItem {
        value: "se",
        label: "Silver edition",
    },
    SelectItem {
        value: "two",
        label: "Two random colors",
    },
    SelectItem {
        value: "ce",
        label: "Classic edition",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("cubesize", "Cube size", 0.4, 1.0, 0.05, 2, "0.7"),
    Opt::slider("rotspeed", "Rotation", 1.0, 10.0, 0.1, 1, "3.0"),
    Opt::slider("spinspeed", "Spin", 0.01, 4.0, 0.01, 2, "1.0"),
    Opt::slider("wanderspeed", "Wander", 0.001, 0.1, 0.001, 3, "0.02"),
    Opt::slider("wait", "Linger", 10.0, 100.0, 1.0, 0, "40.0"),
    Opt::select("randomize", "Start", STARTS, "true"),
    Opt::select("colormode", "Colors", COLORS, "six"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("texture", "Outlines", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cube21",
    label: "Cube 21",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Vasek Potocek",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=AFtxL6--lTQ"),
        blurb: "The Cube 21 Rubik-like puzzle, also known as Square-1.",
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

    fn solved() -> [[i32; 13]; 2] {
        let mut row = [0i32; 13];
        for (i, v) in row.iter_mut().enumerate() {
            *v = i32::from(i % 3 != 1);
        }
        [row, row]
    }

    /// A face can only turn where a cut runs through both halves. Every offset
    /// the puzzle offers has to be one of those, or a turn would tear a wide
    /// piece in two.
    #[test]
    fn a_turn_only_lands_where_both_halves_are_cut() {
        let pieces = solved();
        for s in 0..2 {
            let m = find_matches(&pieces, s);
            assert!(!m.is_empty());
            for &o in &m {
                // Twelve slots, so an offset and its complement are the same
                // cut seen from either side.
                let o = o.rem_euclid(12) as usize;
                assert!(
                    o == 0 || (pieces[s][o] != 0 && pieces[s][(o + 6) % 12] != 0),
                    "offset {o} cuts a piece in half"
                );
            }
        }
    }

    /// Turning a face by twelve slots is turning it all the way round, so it
    /// has to leave the puzzle exactly as it was.
    #[test]
    fn a_whole_turn_changes_nothing() {
        let mut pieces = solved();
        let mut cind = [[0; 12]; 5];
        for (i, row) in cind.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (i * 12 + j) as i32;
            }
        }
        let (p0, c0) = (pieces, cind);
        for _ in 0..12 {
            rot_face(&mut pieces, &mut cind, 0, 1);
        }
        assert_eq!(pieces, p0);
        assert_eq!(cind, c0);
    }

    /// Flipping the same half twice puts everything back, which is what makes
    /// it a flip rather than a shuffle.
    #[test]
    fn flipping_twice_is_doing_nothing() {
        let mut pieces = solved();
        let mut cind = [[0; 12]; 5];
        for (i, row) in cind.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (i * 12 + j) as i32;
            }
        }
        let mut hf = [0, 0];
        let (p0, c0) = (pieces, cind);
        rot_halves(&mut pieces, &mut cind, &mut hf, 0);
        assert_eq!(hf, [1, 0]);
        assert_ne!(cind, c0, "the flip did nothing at all");
        rot_halves(&mut pieces, &mut cind, &mut hf, 0);
        assert_eq!(pieces, p0);
        assert_eq!(cind, c0);
        assert_eq!(hf, [0, 0]);
    }

    /// The whole puzzle is always twelve slots of pieces on each face, however
    /// shuffled, so it always draws a whole object.
    #[test]
    fn it_shuffles_without_falling_apart() {
        let mut r = start(StartArgs::new(640, 480, "wait=10&rotspeed=10", 20260811));
        // Turning and flipping only ever move pieces around, so the number of
        // them never changes; what changes is which colour is where.
        let mut faces = std::collections::BTreeSet::new();
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            assert!(!f.vertices.is_empty());
            for v in &f.vertices {
                let d = (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt();
                assert!(d < 2.0, "a corner {d} from the middle");
            }
            let colours: Vec<i32> = f
                .batches
                .iter()
                .take(20)
                .map(|b| (b.material.ambient_diffuse[0] * 1000.0) as i32)
                .collect();
            faces.insert(colours);
        }
        assert!(faces.len() > 3, "the pieces never moved");
    }
}
