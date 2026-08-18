//! Port of `hacks/delaunay.c`.
//!
//! ```text
//! Triangulate
//! Efficient Triangulation Algorithm Suitable for Terrain Modelling
//! or
//! An Algorithm for Interpolating Irregularly-Spaced Data
//! with Applications in Terrain Modelling
//!
//! Written by Paul Bourke
//! Presented at Pan Pacific Computer Conference, Beijing, China.
//! January 1989
//!
//! http://paulbourke.net/papers/triangulate/
//! http://paulbourke.net/papers/triangulate/triangulate.c
//! ```
//!
//! Bowyer-Watson: start with one triangle big enough to swallow every point,
//! then add the points one at a time. Adding a point means deleting every
//! triangle whose circumcircle contains it, which leaves a hole, and filling
//! that hole with triangles joining the point to each edge of the hole's
//! boundary. Interior edges of the hole appear twice, once from each side, so
//! the boundary is found by throwing away every edge that has a twin.
//!
//! The one thing that keeps it from being quadratic is the completeness flag:
//! once a triangle's circumcircle lies entirely to the left of the point being
//! added, no later point can be inside it either, because the points arrive
//! sorted by x. That triangle is never examined again.

/// One input point. `z` is carried through untouched, which is what the callers
/// use to remember what the point was for.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Xyz {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A triangle, as three indices into the point array.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ITriangle {
    pub p1: usize,
    pub p2: usize,
    pub p3: usize,
}

impl ITriangle {
    /// The three corners in order, which upstream reaches by taking the address
    /// of `p1` and indexing off it.
    pub fn corners(&self) -> [usize; 3] {
        [self.p1, self.p2, self.p3]
    }
}

const EPSILON: f64 = 0.000_001;

/// The circle through three points: its centre, its squared radius, and whether
/// `(xp, yp)` is inside it. A point on the edge counts as inside. `None` when
/// the three points are too nearly coincident to have a circle at all.
///
/// Upstream returns the inside flag and fills the centre and radius through
/// pointers, because the caller wants both: the flag says whether to delete the
/// triangle, and the geometry says whether to stop looking at it.
fn circumcircle(xp: f64, yp: f64, a: Xyz, b: Xyz, c: Xyz) -> Option<(f64, f64, f64, bool)> {
    let fabsy1y2 = (a.y - b.y).abs();
    let fabsy2y3 = (b.y - c.y).abs();

    // Coincident points.
    if fabsy1y2 < EPSILON && fabsy2y3 < EPSILON {
        return None;
    }

    let (xc, yc);
    if fabsy1y2 < EPSILON {
        let m2 = -(c.x - b.x) / (c.y - b.y);
        let mx2 = (b.x + c.x) / 2.0;
        let my2 = (b.y + c.y) / 2.0;
        xc = (b.x + a.x) / 2.0;
        yc = m2 * (xc - mx2) + my2;
    } else if fabsy2y3 < EPSILON {
        let m1 = -(b.x - a.x) / (b.y - a.y);
        let mx1 = (a.x + b.x) / 2.0;
        let my1 = (a.y + b.y) / 2.0;
        xc = (c.x + b.x) / 2.0;
        yc = m1 * (xc - mx1) + my1;
    } else {
        let m1 = -(b.x - a.x) / (b.y - a.y);
        let m2 = -(c.x - b.x) / (c.y - b.y);
        let mx1 = (a.x + b.x) / 2.0;
        let mx2 = (b.x + c.x) / 2.0;
        let my1 = (a.y + b.y) / 2.0;
        let my2 = (b.y + c.y) / 2.0;
        xc = (m1 * mx1 - m2 * mx2 + my2 - my1) / (m1 - m2);
        yc = if fabsy1y2 > fabsy2y3 {
            m1 * (xc - mx1) + my1
        } else {
            m2 * (xc - mx2) + my2
        };
    }

    let (dx, dy) = (b.x - xc, b.y - yc);
    let rsqr = dx * dx + dy * dy;

    let (dx, dy) = (xp - xc, yp - yc);
    let drsqr = dx * dx + dy * dy;

    // The original test was `drsqr <= rsqr`; the epsilon is Chuck Morris's.
    Some((xc, yc, rsqr, drsqr - rsqr <= EPSILON))
}

/// Triangulate `points`, which must be sorted by increasing x.
///
/// Three more points are appended for the enclosing supertriangle and then
/// removed again, so `points` comes back as it went in. Triangles touching the
/// supertriangle are dropped, which is what leaves the convex hull.
pub fn delaunay(points: &mut Vec<Xyz>) -> Vec<ITriangle> {
    let nv = points.len();
    if nv < 3 {
        return Vec::new();
    }

    let trimax = 4 * nv;
    let mut complete: Vec<bool> = Vec::with_capacity(trimax);
    let mut edges: Vec<(i64, i64)> = Vec::new();

    // The bounds, so the supertriangle can be made to swallow everything.
    let mut xmin = points[0].x;
    let mut ymin = points[0].y;
    let mut xmax = xmin;
    let mut ymax = ymin;
    for p in points.iter().skip(1) {
        xmin = xmin.min(p.x);
        xmax = xmax.max(p.x);
        ymin = ymin.min(p.y);
        ymax = ymax.max(p.y);
    }
    let dx = xmax - xmin;
    let dy = ymax - ymin;
    let dmax = dx.max(dy);
    let xmid = (xmax + xmin) / 2.0;
    let ymid = (ymax + ymin) / 2.0;

    points.push(Xyz {
        x: xmid - 20.0 * dmax,
        y: ymid - dmax,
        z: 0.0,
    });
    points.push(Xyz {
        x: xmid,
        y: ymid + 20.0 * dmax,
        z: 0.0,
    });
    points.push(Xyz {
        x: xmid + 20.0 * dmax,
        y: ymid - dmax,
        z: 0.0,
    });

    let mut v: Vec<ITriangle> = vec![ITriangle {
        p1: nv,
        p2: nv + 1,
        p3: nv + 2,
    }];
    complete.push(false);

    for i in 0..nv {
        let xp = points[i].x;
        let yp = points[i].y;
        edges.clear();

        // Every triangle whose circumcircle holds this point gives up its three
        // edges and is deleted.
        let mut j = 0;
        while j < v.len() {
            if complete[j] {
                j += 1;
                continue;
            }
            let (a, b, c) = (points[v[j].p1], points[v[j].p2], points[v[j].p3]);
            let mut inside = false;
            if let Some((xc, _yc, rsqr, hit)) = circumcircle(xp, yp, a, b, c) {
                inside = hit;
                // Once the circle lies wholly to the left of the point, and the
                // points arrive sorted by x, nothing later can be inside it
                // either. That triangle is never looked at again.
                if xc < xp && (xp - xc) * (xp - xc) > rsqr {
                    complete[j] = true;
                }
            }

            if inside {
                edges.push((v[j].p1 as i64, v[j].p2 as i64));
                edges.push((v[j].p2 as i64, v[j].p3 as i64));
                edges.push((v[j].p3 as i64, v[j].p1 as i64));
                let last = v.len() - 1;
                v[j] = v[last];
                complete[j] = complete[last];
                v.pop();
                complete.pop();
                continue; // Re-examine this slot, which now holds another one.
            }
            j += 1;
        }

        // Tag the edges that appear twice: they are interior to the hole, so
        // whatever survives is its boundary.
        for j in 0..edges.len().saturating_sub(1) {
            for k in (j + 1)..edges.len() {
                let same_reversed = edges[j].0 == edges[k].1 && edges[j].1 == edges[k].0;
                // Shouldn't be needed if every triangle is wound the same way.
                let same = edges[j].0 == edges[k].0 && edges[j].1 == edges[k].1;
                if same_reversed || same {
                    edges[j] = (-1, -1);
                    edges[k] = (-1, -1);
                }
            }
        }

        for e in &edges {
            if e.0 < 0 || e.1 < 0 {
                continue;
            }
            if v.len() >= trimax {
                break;
            }
            v.push(ITriangle {
                p1: e.0 as usize,
                p2: e.1 as usize,
                p3: i,
            });
            complete.push(false);
        }
    }

    // Drop everything still touching the supertriangle.
    let mut i = 0;
    while i < v.len() {
        if v[i].p1 >= nv || v[i].p2 >= nv || v[i].p3 >= nv {
            let last = v.len() - 1;
            v[i] = v[last];
            v.pop();
            continue;
        }
        i += 1;
    }

    points.truncate(nv);
    v
}

/// `delaunay_xyzcompare`: sort by increasing x, which the algorithm requires.
pub fn sort_by_x(points: &mut [Xyz]) {
    points.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_becomes_two_triangles() {
        let mut p = vec![
            Xyz {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Xyz {
                x: 0.0,
                y: 10.0,
                z: 0.0,
            },
            Xyz {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            Xyz {
                x: 10.0,
                y: 10.0,
                z: 0.0,
            },
        ];
        sort_by_x(&mut p);
        let tris = delaunay(&mut p);
        assert_eq!(p.len(), 4, "the supertriangle points were left behind");
        assert_eq!(tris.len(), 2, "a square is two triangles");
        for t in &tris {
            for c in t.corners() {
                assert!(c < 4, "a corner escaped the point list");
            }
        }
    }

    /// The whole point of the thing: every triangle's circumcircle has to be
    /// empty of other points, which is what makes a triangulation Delaunay.
    #[test]
    fn no_point_lies_inside_a_circumcircle() {
        crate::runtime::rand::ya_rand_init(11);
        let mut p: Vec<Xyz> = (0..60)
            .map(|_| Xyz {
                x: crate::runtime::frand(200.0),
                y: crate::runtime::frand(200.0),
                z: 0.0,
            })
            .collect();
        sort_by_x(&mut p);
        let tris = delaunay(&mut p);
        assert!(tris.len() > 40, "far too few triangles: {}", tris.len());

        for t in &tris {
            let [a, b, c] = [p[t.p1], p[t.p2], p[t.p3]];
            let Some((xc, yc, r, _)) = circumcircle(0.0, 0.0, a, b, c) else {
                continue;
            };
            for (i, q) in p.iter().enumerate() {
                if i == t.p1 || i == t.p2 || i == t.p3 {
                    continue;
                }
                let d = (q.x - xc) * (q.x - xc) + (q.y - yc) * (q.y - yc);
                assert!(
                    d >= r - 0.01,
                    "point {i} is inside a triangle's circumcircle"
                );
            }
        }
    }
}
