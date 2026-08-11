/* xscreensaver, Copyright (c) 1998-2014 Jamie Zawinski <jwz@jwz.org>
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Upstream's `gllist.c`: the models that were modelled elsewhere.
//!
//! Two dozen savers are a program wrapped around a shape someone drew in a
//! modelling tool. Upstream converts each such shape to C source, a flat array
//! of interleaved floats plus a `struct gllist` header saying how to read it,
//! and draws it with one `glInterleavedArrays` and one `glDrawArrays`.
//!
//! Here the arrays are assets rather than source, because a Rust file with
//! tens of thousands of float literals in it takes minutes to compile. The
//! conversion is `web/homepage/gen-gllist.nu`, which keeps upstream's literals
//! character for character; [`GlList::parse`] reads what it writes.

use crate::runtime::gl::{Glx, Shape};

/// What each vertex carries. Upstream spells these as the `glInterleavedArrays`
/// formats they are, and never uses one with texture coordinates in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Position only.
    V3f,
    /// A colour, then a position.
    C3fV3f,
    /// A normal, then a position.
    N3fV3f,
}

impl Format {
    /// Floats per vertex.
    fn stride(self) -> usize {
        match self {
            Format::V3f => 3,
            _ => 6,
        }
    }
}

/// One drawable list: a primitive, a format, and the vertices.
///
/// Upstream's `next` field chains several of these into one model, but none of
/// the converted files uses it, so the chain is a plain [`Vec`] here and the
/// converter rejects anything that would need more.
#[derive(Clone, Debug)]
pub struct GlList {
    pub format: Format,
    pub primitive: Shape,
    pub points: usize,
    pub data: Vec<f32>,
}

impl GlList {
    /// Read one converted model.
    ///
    /// The header line names the format, the primitive and the vertex count,
    /// and every line after it is one vertex. Anything malformed is a bug in
    /// the converter rather than in a saver, so this panics rather than
    /// returning an error nobody could act on.
    pub fn parse(text: &str) -> Self {
        let mut lines = text.lines();
        let header = lines.next().unwrap_or_default();
        let mut words = header.split_whitespace();
        assert_eq!(words.next(), Some("GLL1"), "not a converted gllist");

        let format = match words.next() {
            Some("v3f") => Some(Format::V3f),
            Some("c3f_v3f") => Some(Format::C3fV3f),
            Some("n3f_v3f") => Some(Format::N3fV3f),
            _ => None,
        }
        .expect("gllist header names no format this reader knows");
        let primitive = match words.next() {
            Some("points") => Some(Shape::Points),
            Some("lines") => Some(Shape::Lines),
            Some("triangles") => Some(Shape::Triangles),
            Some("quads") => Some(Shape::Quads),
            _ => None,
        }
        .expect("gllist header names no primitive this reader knows");
        let points: usize = words
            .next()
            .and_then(|w| w.parse().ok())
            .expect("gllist header has no vertex count");

        let data: Vec<f32> = text
            .lines()
            .skip(1)
            .flat_map(str::split_whitespace)
            .map(|w| w.parse::<f32>().expect("gllist holds a non-number"))
            .collect();
        assert_eq!(
            data.len(),
            points * format.stride(),
            "gllist vertex count disagrees with its data"
        );

        GlList {
            format,
            primitive,
            points,
            data,
        }
    }

    /// The triangles this model is worth, for the polygon counter.
    pub fn polys(&self) -> usize {
        self.points / 3
    }

    /// Draw it, as `renderList` does.
    ///
    /// In wireframe upstream declines to draw the interleaved array and walks
    /// the vertices itself, closing a line loop around every triangle or quad,
    /// which is why the edges of a wireframe model are drawn twice over.
    pub fn render(&self, gl: &mut Glx, wire: bool) {
        let stride = self.format.stride();
        let skip = if self.format == Format::V3f { 0 } else { 3 };

        if !wire || self.primitive == Shape::Lines || self.primitive == Shape::Points {
            gl.begin(self.primitive);
            for v in self.data.chunks_exact(stride) {
                match self.format {
                    Format::V3f => {}
                    Format::C3fV3f => gl.color3f(v[0], v[1], v[2]),
                    Format::N3fV3f => gl.normal3f(v[0], v[1], v[2]),
                }
                gl.vertex3f(v[skip], v[skip + 1], v[skip + 2]);
            }
            gl.end();
            return;
        }

        // Points and lines returned above and the reader admits nothing else,
        // so what is left here is a triangle or a quad.
        let tick = if self.primitive == Shape::Quads { 4 } else { 3 };

        gl.begin(Shape::LineLoop);
        for (i, v) in self.data.chunks_exact(stride).enumerate() {
            if i > 0 && i % tick == 0 {
                gl.end();
                gl.begin(Shape::LineLoop);
            }
            gl.vertex3f(v[skip], v[skip + 1], v[skip + 2]);
        }
        gl.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TETRA: &str = "GLL1 n3f_v3f triangles 3\n\
        0 0 1 -1 0 0\n\
        0 0 1 1 0 0\n\
        0 0 1 0 1 0\n";

    #[test]
    fn the_header_says_how_to_read_the_rest() {
        let list = GlList::parse(TETRA);
        assert_eq!(list.format, Format::N3fV3f);
        assert_eq!(list.primitive, Shape::Triangles);
        assert_eq!(list.points, 3);
        assert_eq!(list.data.len(), 18);
        assert_eq!(list.polys(), 1);
    }

    #[test]
    fn the_normal_and_the_position_do_not_get_swapped() {
        // The interleaved order is normal first, position second. Getting it
        // backwards gives a model that is lit as if inside out, which is easy
        // to miss and hard to explain, so it is worth an assertion.
        let list = GlList::parse(TETRA);
        let mut gl = Glx::new();
        gl.start_frame(64, 64);
        list.render(&mut gl, false);

        let frame = gl.frame();
        assert_eq!(frame.vertices.len(), 3);
        assert_eq!(frame.vertices[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(frame.vertices[0].pos, [-1.0, 0.0, 0.0]);
        assert_eq!(frame.vertices[2].pos, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_wireframe_model_is_line_loops_and_carries_no_normals() {
        let list = GlList::parse(TETRA);
        let mut gl = Glx::new();
        gl.start_frame(64, 64);
        list.render(&mut gl, true);

        let frame = gl.frame();
        // One loop per triangle, and a loop stays a loop: the recorder keeps
        // the primitive rather than cutting it into separate lines, so the
        // three corners are three vertices and the host closes them.
        assert_eq!(frame.batches.len(), 1);
        assert_eq!(
            frame.batches[0].primitive,
            crate::runtime::gl::Primitive::LineLoop
        );
        assert_eq!(frame.vertices.len(), 3);
    }

    #[test]
    fn every_bundled_model_parses() {
        // The converter is a separate program and its output is checked in, so
        // the only thing standing between a bad conversion and a saver that
        // panics in the browser is this.
        for text in crate::models::ALL {
            let list = GlList::parse(text);
            assert!(list.points > 0);
        }
    }
}
