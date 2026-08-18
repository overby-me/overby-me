/* buildlwo.c: Lightwave Object Display List Generation
 *
 * Copyright (c) 1998 by Ed Mackey
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Upstream's `buildlwo.c`: the shapes that came out of Lightwave.
//!
//! `pipes` is the last saver still drawing models Ed Mackey converted out of
//! Lightwave 3D in 1997. The format is three flat arrays: the points, one
//! normal per polygon, and a stream of polygon records. A record is a vertex
//! count, that many point indices, and one filler slot; a count of nought ends
//! the stream. Nothing indexes the normals: they are read in order, one per
//! polygon, which is why the whole thing has to be walked rather than drawn.
//!
//! As with [`gllist`], the arrays are assets rather than Rust source, because a
//! file with a hundred thousand literals in it takes minutes to compile. The
//! conversion is `apps/homepage/gen-lwo.nu`, which keeps upstream's numbers
//! character for character; [`Lwo::parse`] reads what it writes.
//!
//! [`gllist`]: crate::runtime::gllist

use crate::runtime::gl::{Glx, Shape};

/// One converted model.
pub struct Lwo {
    /// The name Lightwave gave it, kept for the panic messages.
    pub name: String,
    /// Upstream's `num_pnts`: how many points there are, three floats each.
    /// It is used for nothing but a guess at a polygon count.
    pub num_pnts: usize,
    pub points: Vec<f32>,
    pub normals: Vec<f32>,
    pub pols: Vec<u16>,
}

/// `glVertex3fv`.
fn vertex(gl: &mut Glx, p: [f32; 3]) {
    gl.vertex3f(p[0], p[1], p[2]);
}

/// Read the count that follows a section keyword.
fn section<'a>(words: &mut impl Iterator<Item = &'a str>, name: &str) -> usize {
    assert_eq!(words.next(), Some(name), "lwo section out of order");
    let count = words.next().and_then(|w| w.parse().ok());
    assert!(count.is_some(), "lwo section {name} has no count");
    count.unwrap_or_default()
}

impl Lwo {
    /// Read one converted model.
    ///
    /// Anything malformed is a bug in the converter rather than in a saver, so
    /// this panics rather than returning an error nobody could act on.
    pub fn parse(text: &str) -> Self {
        let mut words = text.split_whitespace();
        assert_eq!(
            words.next(),
            Some("LWO1"),
            "not a converted Lightwave object"
        );
        let name = words.next().unwrap_or_default().to_string();
        let num_pnts = words
            .next()
            .and_then(|w| w.parse().ok())
            .expect("lwo header has no point count");

        let n = section(&mut words, "pnts");
        let points: Vec<f32> = (&mut words)
            .take(n)
            .map(|w| w.parse().expect("lwo point is not a number"))
            .collect();
        assert_eq!(points.len(), n, "lwo ran out of points");

        let n = section(&mut words, "normals");
        let normals: Vec<f32> = (&mut words)
            .take(n)
            .map(|w| w.parse().expect("lwo normal is not a number"))
            .collect();
        assert_eq!(normals.len(), n, "lwo ran out of normals");

        let n = section(&mut words, "pols");
        let pols: Vec<u16> = (&mut words)
            .take(n)
            .map(|w| w.parse().expect("lwo polygon word is not a number"))
            .collect();
        assert_eq!(pols.len(), n, "lwo ran out of polygon words");

        Lwo {
            name,
            num_pnts,
            points,
            normals,
            pols,
        }
    }

    /// The polygon count upstream reports for the FPS meter, which is this
    /// arithmetic and not the number of polygons.
    pub fn polys(&self) -> usize {
        self.num_pnts / 3
    }

    /// One point, by index.
    fn point(&self, i: usize) -> [f32; 3] {
        let k = i * 3;
        [self.points[k], self.points[k + 1], self.points[k + 2]]
    }

    /// Draw it, as `BuildLWO` does.
    ///
    /// Upstream draws each face as a `GL_POLYGON`, which the runtime can only
    /// take as a triangle fan, and a fan cannot be merged with the fan beside
    /// it. Six hundred unmergeable faces per valve is six hundred draw calls,
    /// so the fan is done here instead: the faces are planar and convex, so
    /// splaying each one from its first vertex draws exactly the same triangles
    /// in one batch. The flat normal is per face either way.
    pub fn render(&self, gl: &mut Glx, wire: bool) {
        let mut normal = 0;
        let mut i = 0;
        // Every face of every one of these models is a triangle or bigger, so
        // the fill case can hold one `glBegin` open across all of them.
        if !wire {
            gl.begin(Shape::Triangles);
        }
        while i < self.pols.len() {
            let count = self.pols[i] as usize;
            if count == 0 {
                break;
            }
            let idx = &self.pols[i + 1..i + 1 + count];
            i += count + 2;

            match count {
                1 => {
                    // Upstream opens a `GL_POINTS` per record here, which none
                    // of the converted models needs.
                    if !wire {
                        gl.end();
                    }
                    gl.begin(Shape::Points);
                    vertex(gl, self.point(idx[0] as usize));
                    gl.end();
                    if !wire {
                        gl.begin(Shape::Triangles);
                    }
                }
                2 => {
                    if !wire {
                        gl.end();
                    }
                    gl.begin(Shape::Lines);
                    for &p in idx {
                        vertex(gl, self.point(p as usize));
                    }
                    gl.end();
                    if !wire {
                        gl.begin(Shape::Triangles);
                    }
                }
                _ => {
                    let n = &self.normals[normal..normal + 3];
                    gl.normal3f(n[0], n[1], n[2]);
                    normal += 3;
                    if wire {
                        gl.begin(Shape::LineLoop);
                        for &p in idx {
                            vertex(gl, self.point(p as usize));
                        }
                        gl.end();
                    } else {
                        let hub = self.point(idx[0] as usize);
                        for w in idx[1..].windows(2) {
                            vertex(gl, hub);
                            vertex(gl, self.point(w[0] as usize));
                            vertex(gl, self.point(w[1] as usize));
                        }
                    }
                }
            }
        }
        if !wire {
            gl.end();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every model the converter emitted reads back, and reads back with the
    /// point count its header claims.
    #[test]
    fn every_model_parses() {
        for text in crate::models::PIPES_MODELS {
            let m = Lwo::parse(text);
            assert_eq!(m.points.len(), m.num_pnts * 3, "{}", m.name);
            assert!(!m.pols.is_empty(), "{}", m.name);
        }
    }

    /// The polygon records name points that exist, and there is exactly one
    /// normal for every face of three points or more. Getting either wrong
    /// would draw a model inside out or out of nothing.
    #[test]
    fn the_records_line_up_with_the_arrays() {
        for text in crate::models::PIPES_MODELS {
            let m = Lwo::parse(text);
            let (mut i, mut faces) = (0, 0);
            while i < m.pols.len() {
                let count = m.pols[i] as usize;
                if count == 0 {
                    break;
                }
                for &p in &m.pols[i + 1..i + 1 + count] {
                    assert!(
                        (p as usize) < m.num_pnts,
                        "{}: point {p} is off the end",
                        m.name
                    );
                }
                if count >= 3 {
                    faces += 1;
                }
                i += count + 2;
            }
            assert!(faces > 0, "{}", m.name);
            assert_eq!(m.normals.len(), faces * 3, "{}", m.name);
        }
    }

    /// A face becomes a fan of triangles rather than a `GL_POLYGON`, so that
    /// the whole model is one batch. Six hundred and twenty faces of four,
    /// sixteen and twenty-four points make one run of triangles.
    #[test]
    fn a_model_draws_in_one_batch() {
        let mut gl = Glx::new();
        gl.start_frame(64, 64);
        Lwo::parse(crate::models::PIPES_BIGVALVE).render(&mut gl, false);
        let f = gl.frame();
        assert_eq!(f.batches.len(), 1, "the valve came apart into batches");
        // 616 quads, two sixteen-gons and two twenty-four-gons.
        let tris = 616 * 2 + 2 * 14 + 2 * 22;
        assert_eq!(f.batches[0].count, tris * 3);
    }

    /// The models are read the right way round.
    ///
    /// Nothing about the format says which of the three floats is which, so a
    /// transposed or misindexed read would still parse and still draw. What
    /// catches it is that `pipes` assembles these shapes into each other: the
    /// collar is half a cell either way about the pipe it clamps, the gauge
    /// stands on a stalk that stops where the head begins, and the head is a
    /// disc whose centre is the 1.33333 that `MakeGuage` translates the needle
    /// to. Upstream leaves a comment on that number saying not to tidy it into
    /// one and a third, which is how firmly it belongs to the model.
    #[test]
    fn the_models_line_up_with_each_other() {
        let bounds = |text: &str| {
            let m = Lwo::parse(text);
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for p in m.points.chunks_exact(3) {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            (lo, hi)
        };

        let (lo, hi) = bounds(crate::models::PIPES_PIPEBETWEENBOLTS);
        assert_eq!((lo, hi), ([-0.5; 3], [0.5; 3]), "the collar");

        let (lo, hi) = bounds(crate::models::PIPES_GUAGEHEAD);
        let centre = (lo[1] + hi[1]) / 2.0;
        assert!(
            (centre - 1.33333).abs() < 0.001,
            "the gauge head is centred on {centre}, not on 1.33333"
        );
        assert!(
            (hi[1] - lo[1] - 1.0).abs() < 0.001,
            "the head is a unit disc"
        );

        // The stalk reaches from the pipe up to where the head starts.
        let (slo, shi) = bounds(crate::models::PIPES_GUAGECONNECTOR);
        assert!(
            slo[1] < lo[1] && shi[1] > lo[1],
            "the stalk misses the head"
        );

        // The face and the needle sit in front of the head, not inside it.
        let (flo, fhi) = bounds(crate::models::PIPES_GUAGEFACE);
        assert_eq!(flo[2], fhi[2], "the gauge face is not flat");
        assert!(flo[1] > lo[1] && fhi[1] < hi[1], "the face is off the head");
        let (nlo, nhi) = bounds(crate::models::PIPES_GUAGEDIAL);
        assert!(nlo[2] >= flo[2], "the needle is behind the face");
        assert!(nhi[0] < 0.1 && nhi[1] < 0.1, "the needle is off its pivot");
    }

    /// Wireframe keeps upstream's line loop around each face, which is a
    /// separate primitive and cannot merge.
    #[test]
    fn wireframe_draws_the_edges_of_each_face() {
        let mut gl = Glx::new();
        gl.start_frame(64, 64);
        Lwo::parse(crate::models::PIPES_BOLTS3D).render(&mut gl, true);
        let f = gl.frame();
        assert_eq!(f.batches.len(), 18, "sixteen quads and two octagons");
        assert_eq!(f.vertices.len(), 16 * 4 + 2 * 8);
    }
}
