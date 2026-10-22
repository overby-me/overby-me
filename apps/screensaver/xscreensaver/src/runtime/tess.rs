//! A polygon tessellator, standing in for `gluNewTess` and friends.
//!
//! Immediate-mode OpenGL will fill a `GL_POLYGON` only if it is convex;
//! anything else is what GLU's tessellator is for, and a handful of savers
//! reach for it. `polyhedra` is the one that cannot do without: the faces of
//! several of the duals are three-pointed stars, and fanning one of those out
//! from a corner fills in the notches.
//!
//! This is ear clipping, which is the simplest thing that works on a simple
//! polygon: repeatedly find a corner whose triangle sticks out of the outline
//! and contains none of the other corners, cut it off, and carry on. It is
//! quadratic in the number of corners, which for a polyhedron face (at most
//! ten) is nothing.
//!
//! GLU's tessellator will also unpick a contour that crosses itself, filling
//! by the odd winding rule. This one will not, and nothing here needs it: a
//! self-crossing face gets split before it ever arrives.

/// Cut a simple polygon into triangles, given its corners in order.
///
/// The corners are 3D but must be roughly coplanar, which is what a face of
/// anything is. Returns triangles as indices into the input, wound the same
/// way round as the input was.
pub fn triangulate(points: &[[f32; 3]]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }

    // Newell's normal, which is right even when the corners are not exactly
    // coplanar, and then the axis it leans on most. Dropping that axis is the
    // projection that keeps the most area.
    let mut normal = [0.0f64; 3];
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        normal[0] += ((a[1] - b[1]) * (a[2] + b[2])) as f64;
        normal[1] += ((a[2] - b[2]) * (a[0] + b[0])) as f64;
        normal[2] += ((a[0] - b[0]) * (a[1] + b[1])) as f64;
    }
    let mut axis = 0;
    for k in 1..3 {
        if normal[k].abs() > normal[axis].abs() {
            axis = k;
        }
    }

    // The projection is chosen so that the sign of the polygon's area in it
    // is the sign of that component of the normal.
    let flat: Vec<[f64; 2]> = points.iter().map(|&p| project(p, axis)).collect();

    // Ear clipping wants the outline anticlockwise. If it is not, walk it
    // backwards and put the winding back at the end.
    let backwards = normal[axis] < 0.0;
    let mut ring: Vec<usize> = if backwards {
        (0..n).rev().collect()
    } else {
        (0..n).collect()
    };

    let mut out: Vec<[usize; 3]> = Vec::with_capacity(n - 2);
    while ring.len() > 3 {
        let m = ring.len();
        let mut cut = None;
        for i in 0..m {
            let (a, b, c) = (ring[(i + m - 1) % m], ring[i], ring[(i + 1) % m]);
            if turn(flat[a], flat[b], flat[c]) <= 0.0 {
                continue; // A notch, or three corners in a line.
            }
            if ring
                .iter()
                .any(|&j| j != a && j != b && j != c && inside(flat[a], flat[b], flat[c], flat[j]))
            {
                continue; // Some other corner is in the way.
            }
            cut = Some(i);
            break;
        }
        let Some(i) = cut else {
            // Not a simple polygon after all: a doubled corner, or an outline
            // that crosses itself. Fan the rest of it so that something is
            // drawn rather than nothing.
            break;
        };
        out.push([ring[(i + m - 1) % m], ring[i], ring[(i + 1) % m]]);
        ring.remove(i);
    }
    for i in 1..ring.len() - 1 {
        out.push([ring[0], ring[i], ring[i + 1]]);
    }

    if backwards {
        for t in &mut out {
            t.swap(1, 2);
        }
    }
    out
}

/// Drop the axis the face leans on, keeping the other two in an order that
/// leaves the area's sign equal to that component of the normal.
fn project(p: [f32; 3], axis: usize) -> [f64; 2] {
    match axis {
        0 => [p[1] as f64, p[2] as f64],
        1 => [p[2] as f64, p[0] as f64],
        _ => [p[0] as f64, p[1] as f64],
    }
}

/// Twice the signed area of the corner: positive if it turns anticlockwise.
fn turn(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Is `p` within the triangle `abc`, which is known to turn anticlockwise?
fn inside(a: [f64; 2], b: [f64; 2], c: [f64; 2], p: [f64; 2]) -> bool {
    turn(a, b, p) >= 0.0 && turn(b, c, p) >= 0.0 && turn(c, a, p) >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twice the area of a polygon lying in the xy plane.
    fn area2(points: &[[f32; 3]]) -> f64 {
        let n = points.len();
        (0..n)
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % n]);
                (a[0] * b[1] - b[0] * a[1]) as f64
            })
            .sum()
    }

    fn tri_area2(points: &[[f32; 3]], t: [usize; 3]) -> f64 {
        area2(&[points[t[0]], points[t[1]], points[t[2]]])
    }

    /// The triangles between them cover exactly the polygon: same total area,
    /// and every one of them wound the same way as the whole.
    fn covers(points: &[[f32; 3]]) {
        let tris = triangulate(points);
        assert_eq!(tris.len(), points.len() - 2, "{tris:?}");
        let whole = area2(points);
        let mut sum = 0.0;
        for &t in &tris {
            let a = tri_area2(points, t);
            assert!(
                a * whole > 0.0,
                "triangle {t:?} is wound the wrong way or is flat"
            );
            sum += a;
        }
        assert!((sum - whole).abs() < 1e-4, "{sum} of {whole}");
    }

    fn ring(xy: &[(f32, f32)]) -> Vec<[f32; 3]> {
        xy.iter().map(|&(x, y)| [x, y, 0.0]).collect()
    }

    #[test]
    fn a_convex_polygon_is_fanned_out() {
        covers(&ring(&[(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (0.0, 1.0)]));
        let pent: Vec<(f32, f32)> = (0..5)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 5.0;
                (a.cos(), a.sin())
            })
            .collect();
        covers(&ring(&pent));
    }

    #[test]
    fn a_notch_is_not_filled_in() {
        // An arrowhead: the fourth corner is pushed in past the other three,
        // and fanning from corner zero would cover the notch.
        let arrow = ring(&[(0.0, 0.0), (2.0, -1.0), (0.0, 3.0), (-2.0, -1.0)]);
        covers(&arrow);
        let tris = triangulate(&arrow);
        // Whichever two triangles come out, neither may reach into the notch,
        // which is the region just below the middle.
        for &t in &tris {
            let mid = [
                (arrow[t[0]][0] + arrow[t[1]][0] + arrow[t[2]][0]) / 3.0,
                (arrow[t[0]][1] + arrow[t[1]][1] + arrow[t[2]][1]) / 3.0,
            ];
            assert!(mid[1] > -0.7, "a triangle sits in the notch at {mid:?}");
        }
    }

    #[test]
    fn a_three_pointed_star_keeps_its_points() {
        // The face of a triambic icosahedron, in outline: three points and
        // three notches between them.
        let mut star = Vec::new();
        for i in 0..3 {
            let a = i as f32 * std::f32::consts::TAU / 3.0;
            star.push((a.cos(), a.sin()));
            let b = a + std::f32::consts::TAU / 6.0;
            star.push((b.cos() * 0.3, b.sin() * 0.3));
        }
        covers(&ring(&star));
    }

    #[test]
    fn winding_and_plane_do_not_matter() {
        let square = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        covers(&ring(&square));
        let mut back = ring(&square);
        back.reverse();
        covers(&back);

        // The same square standing up in the yz plane: the area check is only
        // written for the xy plane, so check the triangle count and that no
        // triangle is degenerate.
        let standing: Vec<[f32; 3]> = square.iter().map(|&(x, y)| [0.0, x, y]).collect();
        let tris = triangulate(&standing);
        assert_eq!(tris.len(), 2);
        for t in tris {
            assert_ne!(t[0], t[1]);
            assert_ne!(t[1], t[2]);
        }
    }

    #[test]
    fn a_broken_outline_still_draws_something() {
        // A bow tie crosses itself, which ear clipping cannot unpick. It must
        // come back with triangles rather than nothing or a hang.
        let bow = ring(&[(0.0, 0.0), (2.0, 2.0), (0.0, 2.0), (2.0, 0.0)]);
        assert_eq!(triangulate(&bow).len(), 2);
        // And a doubled corner.
        let doubled = ring(&[(0.0, 0.0), (1.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
        assert_eq!(triangulate(&doubled).len(), 2);
        assert!(triangulate(&[[0.0; 3]; 2]).is_empty());
    }
}
