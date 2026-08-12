//! Port of `hacks/glx/chessmodels.c`.
//!
//! ```text
//! models for the xss chess screensavers
//! hacked from:
//!
//! glChess - A 3D chess interface
//!
//! Copyright (C) 2006  John-Paul Gignac <jjgignac@users.sf.net>
//!
//! Copyright (C) 2002  Robert  Ancell <bob27@users.sourceforge.net>
//!                     Michael Duelli <duelli@users.sourceforge.net>
//!
//! This program is free software; you can redistribute it and/or modify
//! it under the terms of the GNU General Public License as published by
//! the Free Software Foundation; either version 2 of the License, or
//! (at your option) any later version.
//!
//! This program is distributed in the hope that it will be useful,
//! but WITHOUT ANY WARRANTY; without even the implied warranty of
//! MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//! GNU General Public License for more details.
//!
//! You should have received a copy of the GNU General Public License
//! along with this program; if not, write to the Free Software
//! Foundation, Inc., 59 Temple Place, Suite 330, Boston, MA  02111-1307  USA
//! ```
//!
//! A chess piece as a very small program. The pieces are not vertex lists but
//! tiny bytecodes of `u16`, and this module is the interpreter for them: a
//! handful of opcodes above `65522`, everything below that a number.
//!
//! Nearly all of a piece is `SPIN`, which is a lathe. It carries a step count
//! and then a run of radius/height pairs; each pair becomes a ring of that many
//! points around the Y axis, and consecutive rings are stitched into a tube.
//! `STEPUP` and `STEPDOWN` double and halve the step count part way up, so the
//! wide middle of a queen is turned finely and her narrow stem coarsely without
//! paying for the fine count throughout. `SEAM` emits a ring twice so the two
//! copies can carry different normals and the surface creases there instead of
//! rounding over. `PATTERN` makes a ring that is not round: it gives a short
//! run of pairs that is repeated around the circle, which is how the queen gets
//! her crown.
//!
//! The rest of the opcodes are for the knight, which is not a solid of
//! revolution and is spelled out as loose vertices and faces.
//!
//! Normals are not in the data. Every face contributes its own normal to each
//! of its corners and the sum is normalised at the end, so the surface comes
//! out smooth except across a seam, where the doubled ring gives the two sides
//! separate corners to average into.

use super::gl::{Glx, Shape};

/* Section headers */
const ENDOFDATA: u16 = 65535;
const SPIN: u16 = 65534;
const VERTICES: u16 = 65533;
const QUADS: u16 = 65532;
const TRIANGLES: u16 = 65531;
const POLARQUADSTRIP: u16 = 65530;
const QUADSTRIP: u16 = 65529;

/* Special spin-related commands */
const SEAM: u16 = 65528;
const PATTERN: u16 = 65527;
const STEPUP: u16 = 65526;
const STEPDOWN: u16 = 65525;
const SETBACKREF: u16 = 65524;
const BACKREF: u16 = 65523;

/// The queen, from the detailed set. The other five pieces are the same format
/// and are not here because nothing has asked for them yet.
#[rustfmt::skip]
const QUEEN_DATA: &[u16] = &[
    SPIN, 24,
    11092, 0, 11092, 914, SEAM, 10653, 1284,
    11018, 1798, 11018, 2358, 10787, 2866,
    STEPDOWN, 8739, 3726, 7412, 5168, 6937, 7171,
    STEPUP, 6737, 9556, 6537, 9762, STEPDOWN, 5536, 10191, 5073, 10546,
    4368, 11485, 3678, 15137, SEAM, 3259, 26879,
    5966, 27091, STEPUP, 7332, 27515, 7619, 27882, 7545, 28455, 7317, 28751,
    5654, 29177, 5538, 29326, 5542, 29982, 5377, 30278,
    STEPDOWN, SEAM, 4194, 30585,
    SEAM, 4226, 31822, 5002, 32218, STEPUP, 5139, 32477, 5058, 32774,
    SEAM, 4227, 33040, STEPDOWN, 4421, 34778, 5042, 36612, 5874, 38429,
    STEPUP, SEAM, PATTERN, 3, 6018, 39660, 6018, 39660, 6804, 39977,
    SEAM, PATTERN, 3, 5015, 41139, 5015, 41139, 5673, 41460,
    SEAM, 4349, 40044,
    STEPDOWN, SEAM, 1381, 41188,
    1396, 42332, STEPDOWN, 1082, 43072, 481, 43476, 0, 43543,
    ENDOFDATA,
];

/// How much a unit of the model data is worth. The detailed set is drawn in
/// units of a 8192-wide board square; the classic set, which is not here, uses
/// `0.095 / 100`.
const PIECE_SIZE: f64 = 0.3 / 8192.0;

/// A face is three or four vertex indices; a triangle leaves the fourth as
/// `None`.
type Face = [Option<usize>; 4];

/// A chess piece, run out of its bytecode into vertices, smoothed normals and
/// faces.
pub struct Piece {
    vertices: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    faces: Vec<Face>,
}

impl Piece {
    /// `draw_piece (queen_data)`, all but the drawing.
    pub fn queen() -> Piece {
        Piece::new(QUEEN_DATA)
    }

    fn new(data: &[u16]) -> Piece {
        let mut vertices = Vec::new();
        enumerate_vertices(data, &mut |x, y, z| {
            vertices.push([x as f32, y as f32, z as f32]);
        });

        let mut faces = Vec::new();
        enumerate_faces(data, &mut |f| faces.push(f));

        // Add up all the face normals at each vertex, then normalise.
        let mut normals = vec![[0.0f32; 3]; vertices.len()];
        for f in &faces {
            add_normal(&vertices, &mut normals, *f);
        }
        for n in &mut normals {
            normalize(n);
        }

        Piece {
            vertices,
            normals,
            faces,
        }
    }

    /// How many faces the piece has, which is what upstream reports as its
    /// polygon count.
    pub fn polys(&self) -> usize {
        self.faces.len()
    }

    /// `glCallList` of the piece's list. Upstream opens a `glBegin` per face;
    /// here the quads and the triangles are each drawn in one run, since the
    /// piece is one opaque colour and the order within it does not show.
    pub fn draw(&self, g: &mut Glx) -> usize {
        for (shape, quads) in [(Shape::Quads, true), (Shape::Triangles, false)] {
            let mut any = false;
            for f in self.faces.iter().filter(|f| f[3].is_some() == quads) {
                if !any {
                    g.begin(shape);
                    any = true;
                }
                for v in f.iter().flatten() {
                    let n = self.normals[*v];
                    g.normal3f(n[0], n[1], n[2]);
                    let p = self.vertices[*v];
                    g.vertex3f(p[0], p[1], p[2]);
                }
            }
            if any {
                g.end();
            }
        }
        self.polys()
    }
}

fn normalize(v: &mut [f32; 3]) {
    let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if d == 0.0 {
        // The direction is undefined - normalize it anyway
        *v = [1.0, 0.0, 0.0];
        return;
    }
    v[0] /= d;
    v[1] /= d;
    v[2] /= d;
}

fn normcrossprod(v1: [f32; 3], v2: [f32; 3]) -> [f32; 3] {
    let mut out = [
        v1[1] * v2[2] - v1[2] * v2[1],
        v1[2] * v2[0] - v1[0] * v2[2],
        v1[0] * v2[1] - v1[1] * v2[0],
    ];
    normalize(&mut out);
    out
}

fn diff(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add_normal(vertices: &[[f32; 3]], normals: &mut [[f32; 3]], f: Face) {
    let mut add = |at: usize, n: [f32; 3]| {
        let dst = &mut normals[at];
        dst[0] += n[0];
        dst[1] += n[1];
        dst[2] += n[2];
    };
    let face = |at: usize, next: usize, prev: usize| {
        normcrossprod(
            diff(vertices[next], vertices[at]),
            diff(vertices[prev], vertices[at]),
        )
    };
    match f {
        [Some(v1), Some(v2), Some(v3), None] => {
            // A triangle is flat, so its three corners share one normal.
            let n = face(v1, v2, v3);
            add(v1, n);
            add(v2, n);
            add(v3, n);
        }
        [Some(v1), Some(v2), Some(v3), Some(v4)] => {
            // A quad is not necessarily flat, so each corner takes the cross
            // product of the two edges that meet there.
            add(v1, face(v1, v2, v4));
            add(v2, face(v2, v3, v1));
            add(v3, face(v3, v4, v2));
            add(v4, face(v4, v1, v3));
        }
        _ => {}
    }
}

/// One ring of a `SPIN`. Returns how much of `data` it consumed.
fn enumerate_ring_vertices(
    steps: usize,
    data: &[u16],
    process_vertex: &mut impl FnMut(f64, f64, f64),
) -> usize {
    let mut patlen = 1;
    let mut at = 0;
    let dtheta = std::f64::consts::PI * 2.0 / steps as f64;
    let mut steps = steps;

    if data[0] == PATTERN {
        patlen = data[1] as usize;
        at += 2;
    }
    let pts = &data[at..];

    // A ring of radius zero is one point, however many steps are current.
    if pts[0] == 0 {
        steps = 1;
    }

    for i in 0..steps {
        let r = pts[(i % patlen) * 2] as f64 * PIECE_SIZE;
        let y = pts[(i % patlen) * 2 + 1] as f64 * PIECE_SIZE;
        let theta = dtheta * i as f64;
        process_vertex(r * theta.cos(), y, r * theta.sin());
    }

    at + patlen * 2
}

fn enumerate_vertices(data: &[u16], process_vertex: &mut impl FnMut(f64, f64, f64)) {
    let mut at = 0;
    loop {
        if data[at] == SPIN {
            let mut steps = data[at + 1] as usize;
            at += 2;

            while data[at] <= SEAM {
                if data[at] == SETBACKREF || data[at] == BACKREF {
                    at += 2;
                } else if data[at] == STEPUP {
                    steps *= 2;
                    at += 1;
                } else if data[at] == STEPDOWN {
                    steps /= 2;
                    at += 1;
                } else if data[at] == SEAM {
                    at += 1;
                    // Visit seam vertices twice
                    enumerate_ring_vertices(steps, &data[at..], process_vertex);
                    at += enumerate_ring_vertices(steps, &data[at..], process_vertex);
                } else {
                    at += enumerate_ring_vertices(steps, &data[at..], process_vertex);
                }
            }
        } else if data[at] == POLARQUADSTRIP {
            let steps = data[at + 1] as usize;
            at += 2;
            let dtheta = std::f64::consts::PI * 2.0 / steps as f64;

            while data[at] <= SEAM {
                if data[at] != BACKREF {
                    let theta = dtheta * data[at] as f64;
                    let r = data[at + 1] as f64 * PIECE_SIZE;
                    let y = data[at + 2] as f64 * PIECE_SIZE;
                    process_vertex(r * theta.cos(), y, r * theta.sin());
                }
                at += 3;
            }
        } else if data[at] == QUADSTRIP || data[at] == VERTICES {
            at += 1;

            while data[at] <= SEAM {
                if data[at] == SETBACKREF {
                    at += 2;
                    continue;
                }
                if data[at] != BACKREF {
                    let x = data[at] as i16 as f64 * PIECE_SIZE;
                    let y = data[at + 1] as f64 * PIECE_SIZE;
                    let z = data[at + 2] as i16 as f64 * PIECE_SIZE;
                    process_vertex(x, y, z);
                }
                at += 3;
            }
        } else if data[at] == QUADS || data[at] == TRIANGLES {
            at += 1;
            while data[at] <= SEAM {
                at += 1;
            }
        } else {
            break;
        }
    }
}

/// Stitch two consecutive rings of a `SPIN` together. The counts need not
/// match: `STEPUP` and `STEPDOWN` change them mid-piece, and a ring of one
/// point closes the shape off at the ends.
fn enumerate_ring_faces(
    basevertex: usize,
    steps: usize,
    prevbase: usize,
    prevsteps: usize,
    process_face: &mut impl FnMut(Face),
) {
    if steps == 1 {
        for i in 0..prevsteps {
            process_face([
                Some(basevertex),
                Some(prevbase + i),
                Some(prevbase + if i != 0 { i - 1 } else { prevsteps - 1 }),
                None,
            ]);
        }
    } else if steps == prevsteps {
        for i in 0..steps {
            process_face([
                Some(basevertex + i),
                Some(prevbase + i),
                Some(prevbase + if i != 0 { i - 1 } else { steps - 1 }),
                Some(basevertex + if i != 0 { i - 1 } else { steps - 1 }),
            ]);
        }
    } else {
        // The two rings have different counts, so walk them together and take
        // whichever is behind: a fan of triangles rather than a ladder.
        let mut j = 0;
        let mut i = 0;
        loop {
            while j < prevsteps && steps * (1 + 2 * j) < prevsteps * (1 + 2 * i) {
                process_face([
                    Some(basevertex + (i % steps)),
                    Some(prevbase + ((j + 1) % prevsteps)),
                    Some(prevbase + j),
                    None,
                ]);
                j += 1;
            }
            if i == steps {
                break;
            }
            process_face([
                Some(basevertex + i),
                Some(basevertex + ((i + 1) % steps)),
                Some(prevbase + (j % prevsteps)),
                None,
            ]);
            i += 1;
        }
    }
}

fn enumerate_faces(data: &[u16], process_face: &mut impl FnMut(Face)) {
    let mut basevertex = 0usize;
    let mut startofvertices = 0usize;
    let mut backrefs = [0usize; 5];
    let mut at = 0;

    loop {
        if data[at] == SPIN {
            let mut steps = data[at + 1] as usize;
            let mut prevsteps: Option<usize> = None;
            let mut prevbase = 0usize;
            at += 2;

            while data[at] <= SEAM {
                if data[at] == SETBACKREF {
                    backrefs[data[at + 1] as usize] = basevertex;
                    at += 2;
                    continue;
                }

                if data[at] == STEPUP {
                    steps *= 2;
                    at += 1;
                    continue;
                } else if data[at] == STEPDOWN {
                    steps /= 2;
                    at += 1;
                    continue;
                }

                if data[at] == BACKREF {
                    let b = backrefs[data[at + 1] as usize];
                    if let Some(prevsteps) = prevsteps {
                        enumerate_ring_faces(b, steps, prevbase, prevsteps, process_face);
                    }
                    prevbase = b;
                    at += 2;
                } else {
                    let mut isseam = false;
                    if data[at] == SEAM {
                        isseam = true;
                        at += 1;
                    }

                    if data[at] == PATTERN {
                        at += 2 + data[at + 1] as usize * 2;
                    } else {
                        if data[at] == 0 {
                            steps = 1;
                        }
                        at += 2;
                    }

                    if let Some(prevsteps) = prevsteps {
                        enumerate_ring_faces(basevertex, steps, prevbase, prevsteps, process_face);
                    }

                    if isseam {
                        basevertex += steps;
                    }
                    prevbase = basevertex;
                    basevertex += steps;
                }

                prevsteps = Some(steps);
            }
        } else if data[at] == POLARQUADSTRIP || data[at] == QUADSTRIP {
            let mut v0: Option<usize> = None;
            let mut v1 = 0usize;
            if data[at] == POLARQUADSTRIP {
                at += 2;
            } else {
                at += 1;
            }
            while data[at] <= SEAM {
                let v2 = if data[at] == BACKREF {
                    backrefs[data[at + 1] as usize] + data[at + 2] as usize
                } else {
                    basevertex += 1;
                    basevertex - 1
                };
                let v3 = if data[at + 3] == BACKREF {
                    backrefs[data[at + 4] as usize] + data[at + 5] as usize
                } else {
                    basevertex += 1;
                    basevertex - 1
                };
                at += 6;
                if let Some(v0) = v0 {
                    process_face([Some(v0), Some(v1), Some(v3), Some(v2)]);
                }
                v0 = Some(v2);
                v1 = v3;
            }
        } else if data[at] == VERTICES {
            at += 1;
            startofvertices = basevertex;
            while data[at] <= SEAM {
                if data[at] == SETBACKREF {
                    backrefs[data[at + 1] as usize] = basevertex;
                    at += 2;
                    continue;
                }
                at += 3;
                basevertex += 1;
            }
        } else if data[at] == QUADS {
            at += 1;
            while data[at] <= SEAM {
                process_face([
                    Some(data[at] as usize + startofvertices),
                    Some(data[at + 1] as usize + startofvertices),
                    Some(data[at + 2] as usize + startofvertices),
                    Some(data[at + 3] as usize + startofvertices),
                ]);
                at += 4;
            }
        } else if data[at] == TRIANGLES {
            at += 1;
            while data[at] <= SEAM {
                process_face([
                    Some(data[at] as usize + startofvertices),
                    Some(data[at + 1] as usize + startofvertices),
                    Some(data[at + 2] as usize + startofvertices),
                    None,
                ]);
                at += 3;
            }
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queen_is_a_solid_of_revolution() {
        let q = Piece::queen();
        assert!(q.vertices.len() > 500, "{}", q.vertices.len());
        assert_eq!(q.vertices.len(), q.normals.len());
        assert!(q.polys() > 500, "{}", q.polys());

        // Every face names a vertex that exists.
        for f in &q.faces {
            for v in f.iter().flatten() {
                assert!(*v < q.vertices.len(), "vertex {v} of {}", q.vertices.len());
            }
        }

        // She stands on the origin and reaches up the Y axis, about 1.6 board
        // squares tall and 0.8 wide.
        let y = |f: fn(f32, f32) -> f32, i: usize| {
            q.vertices.iter().fold(q.vertices[0][i], |a, v| f(a, v[i]))
        };
        assert!((y(f32::min, 1) - 0.0).abs() < 1e-6, "{}", y(f32::min, 1));
        assert!((y(f32::max, 1) - 1.594).abs() < 0.01, "{}", y(f32::max, 1));
        let r = q
            .vertices
            .iter()
            .fold(0.0f32, |a, v| a.max((v[0] * v[0] + v[2] * v[2]).sqrt()));
        assert!((r - 0.406).abs() < 0.01, "{r}");
    }

    #[test]
    fn her_normals_point_outwards() {
        let q = Piece::queen();
        // A lathe's normals lean away from the axis, bar the very top and the
        // ring that closes the base, so most of them have a positive dot
        // product with the vertex's own direction from the axis.
        let out = q
            .vertices
            .iter()
            .zip(&q.normals)
            .filter(|(v, n)| v[0] * n[0] + v[2] * n[2] > 0.0)
            .count();
        assert!(out * 4 > q.vertices.len() * 3, "{out}/{}", q.vertices.len());
        for n in &q.normals {
            let d = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((d - 1.0).abs() < 1e-4, "{d}");
        }
    }
}
