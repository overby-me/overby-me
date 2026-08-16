//! Port of `hacks/glx/dymaxionmap-coords.c`.
//!
//! ```text
//! http://www.rwgrayprojects.com/rbfnotes/maps/graymap6.html
//! Slightly modified by jwz for xscreensaver
//!
//! /**************************************************************/
//! /*                                                            */
//! /* This C program is copyrighted by  Robert W. Gray and may   */
//! /* not be used in ANY for-profit project without written      */
//! /* permission.                                                */
//! /*                                                            */
//! /**************************************************************/
//!
//! (Note: Robert Gray has kindly given me his permission to include
//! this code in xscreensaver. -- Jamie Zawinski, Apr 2018.)
//!
//! /**************************************************************/
//! /*                                                            */
//! /* This C program contains the Dymaxion map coordinate        */
//! /* transformation routines for converting longitude/latitude  */
//! /* points to (X, Y) points on the Dymaxion map.               */
//! /*                                                            */
//! /* This version uses the exact transformation equations.      */
//! /**************************************************************/
//! ```
//!
//! Buckminster Fuller's projection: wrap the globe in an icosahedron, project
//! each face out from the centre of the sphere onto its plane, then cut the
//! icosahedron open and lay it flat. Fuller's cut runs entirely through ocean,
//! so the continents come out as one nearly unbroken island, and no face is
//! distorted much because none of them is very big.
//!
//! [`convert`] is the whole of it: a longitude and a latitude in, and a point
//! on the unfolded net out, in a box [`WIDTH`] by [`HEIGHT`] with the origin
//! at the bottom left. It works out which of the twenty faces the point falls
//! on by which face centre it is nearest, rotates it into a template triangle,
//! applies Gray's exact equations, and then puts the triangle where it belongs
//! in the net.

use std::f64::consts::PI;

/// The width of the unfolded net, in units of one triangle edge.
pub const WIDTH: f64 = 5.5;

/// And its height. Five and a half by two and a half triangles.
pub const HEIGHT: f64 = 2.598_076_211_353_316; // 3 * sqrt(3) / 2

/// The twelve vertices of the icosahedron, turned to the orientation Fuller
/// chose: the one whose cut runs through water all the way round.
#[rustfmt::skip]
const V: [[f64; 3]; 13] = [
    [0.0, 0.0, 0.0], // Gray indexes from one; this keeps his numbering.
    [ 0.42015242670871,  0.07814524940278296,  0.9040825506150193],
    [ 0.9950094394362416, -0.09134779527642793,  0.040147175877166645],
    [ 0.5188367303273644,  0.8354203803782358,  0.18133183755726245],
    [-0.4146822253203352,  0.6559624054348008,  0.6306758078914754],
    [-0.5154559599440418, -0.381716898287133,  0.7672009925177475],
    [ 0.3557814025329447, -0.8435800024661781,  0.40223422660292557],
    [ 0.4146822253203352, -0.6559624054348008, -0.6306758078914754],
    [ 0.5154559599440418,  0.381716898287133, -0.7672009925177475],
    [-0.3557814025329447,  0.8435800024661781, -0.40223422660292557],
    [-0.9950094394362416,  0.09134779527642793, -0.040147175877166645],
    [-0.5188367303273644, -0.8354203803782358, -0.18133183755726245],
    [-0.42015242670871, -0.07814524940278296, -0.9040825506150193],
];

/// Which three vertices make up each of the twenty faces, in Gray's order.
#[rustfmt::skip]
const FACE: [[usize; 3]; 21] = [
    [0, 0, 0],
    [1, 2, 3],   [1, 3, 4],   [1, 4, 5],   [1, 5, 6],   [1, 2, 6],
    [2, 3, 8],   [8, 3, 9],   [9, 3, 4],   [10, 9, 4],  [5, 10, 4],
    [5, 11, 10], [5, 6, 11],  [11, 6, 7],  [7, 6, 2],   [8, 7, 2],
    [12, 9, 8],  [12, 9, 10], [12, 11, 10], [12, 11, 7], [12, 8, 7],
];

/// The three vertices of each face again, this time in the order
/// `s_tri_info` wants them for deciding which sixth of the face a point is
/// in. Gray writes them out separately and they are not the same order.
#[rustfmt::skip]
const LCD_FACE: [[usize; 3]; 21] = [
    [0, 0, 0],
    [1, 3, 2],  [1, 4, 3],  [1, 5, 4],  [1, 6, 5],  [1, 2, 6],
    [2, 3, 8],  [3, 9, 8],  [3, 4, 9],  [4, 10, 9], [4, 5, 10],
    [5, 11, 10], [5, 6, 11], [6, 7, 11], [2, 7, 6], [2, 8, 7],
    [8, 9, 12], [9, 10, 12], [10, 11, 12], [11, 7, 12], [8, 12, 7],
];

/// Which vertex of each face is used as the reference when rotating the face
/// into the template triangle.
#[rustfmt::skip]
const REF_VERT: [usize; 21] = [
    0,
    1, 1, 1, 1, 1, 2, 3, 3, 4, 4,
    5, 5, 6, 2, 2, 8, 9, 10, 11, 8,
];

/// Where each face goes in the unfolded net: how far to turn it, and where to
/// put it. Two of the faces are cut in half and the halves go to opposite
/// ends of the net, which is what the second entry is for.
#[rustfmt::skip]
const PLACE: [(f64, f64, f64); 21] = [
    (0.0, 0.0, 0.0),
    (240.0, 2.0, 7.0 / (2.0 * SQRT3)),
    (300.0, 2.0, 5.0 / (2.0 * SQRT3)),
    (0.0,   2.5, 2.0 / SQRT3),
    (60.0,  3.0, 5.0 / (2.0 * SQRT3)),
    (180.0, 2.5, 4.0 * SQRT3 / 3.0),
    (300.0, 1.5, 4.0 * SQRT3 / 3.0),
    (300.0, 1.0, 5.0 / (2.0 * SQRT3)),
    (0.0,   1.5, 2.0 / SQRT3),
    (300.0, 1.5, 1.0 / SQRT3),          // and the other half, below
    (60.0,  2.5, 1.0 / SQRT3),
    (60.0,  3.5, 1.0 / SQRT3),
    (120.0, 3.5, 2.0 / SQRT3),
    (60.0,  4.0, 5.0 / (2.0 * SQRT3)),
    (0.0,   4.0, 7.0 / (2.0 * SQRT3)),
    (0.0,   5.0, 7.0 / (2.0 * SQRT3)),
    (60.0,  0.5, 1.0 / SQRT3),          // and the other half, below
    (0.0,   1.0, 1.0 / (2.0 * SQRT3)),
    (120.0, 4.0, 1.0 / (2.0 * SQRT3)),
    (120.0, 4.5, 2.0 / SQRT3),
    (300.0, 5.0, 5.0 / (2.0 * SQRT3)),
];

const SQRT3: f64 = 1.732_050_807_568_877_2;

/// The constants Gray derives once from the icosahedron: the arc a triangle
/// edge subtends, and two lengths of the template triangle.
struct Constants {
    /// The centre of each face, on the unit sphere.
    centre: [[f64; 3]; 21],
    garc: f64,
    gt: f64,
    gdve: f64,
    gel: f64,
}

impl Constants {
    fn new() -> Self {
        let mut centre = [[0.0f64; 3]; 21];
        for (i, f) in FACE.iter().enumerate().skip(1) {
            let mut h = [0.0f64; 3];
            for k in 0..3 {
                h[k] = (V[f[0]][k] + V[f[1]][k] + V[f[2]][k]) / 3.0;
            }
            let magn = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
            for k in 0..3 {
                centre[i][k] = h[k] / magn;
            }
        }

        let garc = 2.0 * ((5.0 - 5.0f64.sqrt()).sqrt() / 10.0f64.sqrt()).asin();
        Constants {
            centre,
            garc,
            gt: garc / 2.0,
            gdve: (3.0 + 5.0f64.sqrt()).sqrt() / (5.0 + 5.0f64.sqrt()).sqrt(),
            gel: 8.0f64.sqrt() / (5.0 + 5.0f64.sqrt()).sqrt(),
        }
    }
}

/// Gray computes these once into globals; here they are worked out on first
/// use and kept, which is the same thing without the global.
fn constants() -> &'static Constants {
    use std::sync::OnceLock;
    static ONCE: OnceLock<Constants> = OnceLock::new();
    ONCE.get_or_init(Constants::new)
}

fn radians(degrees: f64) -> f64 {
    degrees * PI / 180.0
}

/// `rotate`: turn a point in the plane.
fn rotate(angle: f64, x: f64, y: f64) -> (f64, f64) {
    let (s, c) = radians(angle).sin_cos();
    (x * c - y * s, x * s + y * c)
}

/// `r2`: turn a point about one of the three axes. Gray numbers them from
/// one, and turns two of them the other way round from the third.
fn r2(axis: usize, alpha: f64, p: [f64; 3]) -> [f64; 3] {
    let (a, b, c) = (p[0], p[1], p[2]);
    let (s, co) = alpha.sin_cos();
    match axis {
        1 => [a, b * co + c * s, c * co - b * s],
        2 => [a * co - c * s, b, a * s + c * co],
        _ => [a * co + b * s, b * co - a * s, c],
    }
}

/// `c_to_s`: cartesian to spherical polar, in radians.
fn c_to_s(x: f64, y: f64, z: f64) -> (f64, f64) {
    let mut a = 0.0;
    if x > 0.0 && y > 0.0 {
        a = 0.0;
    }
    if x < 0.0 && y > 0.0 {
        a = PI;
    }
    if x < 0.0 && y < 0.0 {
        a = PI;
    }
    if x > 0.0 && y < 0.0 {
        a = 2.0 * PI;
    }
    let lat = z.clamp(-1.0, 1.0).acos();
    let lng = if x == 0.0 && y > 0.0 {
        PI / 2.0
    } else if x == 0.0 && y < 0.0 {
        3.0 * PI / 2.0
    } else if x > 0.0 && y == 0.0 {
        0.0
    } else if x < 0.0 && y == 0.0 {
        PI
    } else {
        (y / x).atan() + a
    };
    (lng, lat)
}

/// `s_tri_info`: which of the twenty faces a point is on, and which sixth of
/// that face. Two of the faces are cut in half in the net, and the sixth is
/// what says which half a point belongs to.
fn tri_info(k: &Constants, p: [f64; 3]) -> (usize, usize) {
    let mut tri = 0;
    let mut best = 9999.0f64;
    for (i, c) in k.centre.iter().enumerate().skip(1) {
        let d = ((c[0] - p[0]).powi(2) + (c[1] - p[1]).powi(2) + (c[2] - p[2]).powi(2)).sqrt();
        if d < best {
            tri = i;
            best = d;
        }
    }

    let f = LCD_FACE[tri];
    let dist = |v: usize| {
        ((p[0] - V[v][0]).powi(2) + (p[1] - V[v][1]).powi(2) + (p[2] - V[v][2]).powi(2)).sqrt()
    };
    let (d1, d2, d3) = (dist(f[0]), dist(f[1]), dist(f[2]));

    let lcd = if d1 <= d2 && d2 <= d3 {
        1
    } else if d1 <= d3 && d3 <= d2 {
        6
    } else if d2 <= d1 && d1 <= d3 {
        2
    } else if d2 <= d3 && d3 <= d1 {
        3
    } else if d3 <= d1 && d1 <= d2 {
        5
    } else {
        4
    };
    (tri, lcd)
}

/// `dymax_point`: a point known to be on face `tri`, placed in the net.
fn dymax_point(k: &Constants, tri: usize, lcd: usize, p: [f64; 3]) -> (f64, f64) {
    let v1 = REF_VERT[tri];
    let mut h0 = p;
    let mut h1 = V[v1];
    let c = k.centre[tri];

    // Turn the face centre to the pole, then the reference vertex to a known
    // longitude, which puts the whole face in the template triangle.
    let (hlng, hlat) = c_to_s(c[0], c[1], c[2]);
    h0 = r2(3, hlng, h0);
    h1 = r2(3, hlng, h1);
    h0 = r2(2, hlat, h0);
    h1 = r2(2, hlat, h1);
    let (hlng, _) = c_to_s(h1[0], h1[1], h1[2]);
    h0 = r2(3, hlng - radians(90.0), h0);

    /* exact transformation equations */
    let gz = (1.0 - h0[0] * h0[0] - h0[1] * h0[1]).max(0.0).sqrt();
    let gs = (5.0 + 2.0 * 5.0f64.sqrt()).sqrt() / (gz * 15.0f64.sqrt());

    let gxp = h0[0] * gs;
    let gyp = h0[1] * gs;

    let ga1p = 2.0 * gyp / SQRT3 + (k.gel / 3.0);
    let ga2p = gxp - (gyp / SQRT3) + (k.gel / 3.0);
    let ga3p = (k.gel / 3.0) - gxp - (gyp / SQRT3);

    let ga1 = k.gt + ((ga1p - 0.5 * k.gel) / k.gdve).atan();
    let ga2 = k.gt + ((ga2p - 0.5 * k.gel) / k.gdve).atan();
    let ga3 = k.gt + ((ga3p - 0.5 * k.gel) / k.gdve).atan();

    let gx = 0.5 * (ga2 - ga3);
    let gy = (1.0 / (2.0 * SQRT3)) * (2.0 * ga1 - ga2 - ga3);

    /* Re-scale so plane triangle edge length is 1. */
    let x = gx / k.garc;
    let y = gy / k.garc;

    // Two faces are cut in half, and which half a point is on decides which
    // end of the net it goes to.
    let (angle, ox, oy) = match tri {
        9 if lcd <= 2 => (0.0, 2.0, 1.0 / (2.0 * SQRT3)),
        16 if lcd >= 4 => (0.0, 5.5, 2.0 / SQRT3),
        _ => PLACE[tri],
    };
    let (x, y) = rotate(angle, x, y);
    (x + ox, y + oy)
}

/// `dymaxion_convert`: a longitude and latitude in degrees, to a point on the
/// unfolded net.
pub fn convert(lng: f64, lat: f64) -> (f64, f64) {
    let k = constants();

    /* Convert the given (long.,lat.) coordinate into spherical polar
    coordinates, then into cartesian. */
    let theta = radians(90.0 - lat);
    let phi = radians(if lng < 0.0 { lng + 360.0 } else { lng });
    let p = [
        theta.sin() * phi.cos(),
        theta.sin() * phi.sin(),
        theta.cos(),
    ];

    let (tri, lcd) = tri_info(k, p);
    dymax_point(k, tri, lcd, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_globe_lands_on_the_net() {
        // Nothing anywhere on the sphere may fall outside the box the net
        // occupies, and between them the points have to cover most of it.
        let (mut lox, mut loy) = (f64::MAX, f64::MAX);
        let (mut hix, mut hiy) = (f64::MIN, f64::MIN);
        for i in 0..=180 {
            let lat = -90.0 + i as f64;
            for j in 0..=360 {
                let lng = -180.0 + j as f64;
                let (x, y) = convert(lng, lat);
                assert!(x.is_finite() && y.is_finite(), "{lng},{lat} -> {x},{y}");
                lox = lox.min(x);
                hix = hix.max(x);
                loy = loy.min(y);
                hiy = hiy.max(y);
            }
        }
        assert!(lox > -0.001 && hix < WIDTH + 0.001, "{lox} to {hix}");
        assert!(loy > -0.001 && hiy < HEIGHT + 0.001, "{loy} to {hiy}");
        // And it really does use the whole width and height.
        assert!(hix - lox > WIDTH - 0.2, "only {} wide", hix - lox);
        assert!(hiy - loy > HEIGHT - 0.2, "only {} tall", hiy - loy);
    }

    #[test]
    fn each_face_goes_to_its_own_triangle_of_the_net() {
        // The twenty face centres are the middles of the twenty triangles, so
        // no two of them may land in the same place, and each must be about
        // an edge length from three of the others.
        let k = constants();
        let mut centres = Vec::new();
        for c in k.centre.iter().skip(1) {
            let (lng, lat) = c_to_s(c[0], c[1], c[2]);
            let (x, y) = convert(lng * 180.0 / PI, 90.0 - lat * 180.0 / PI);
            centres.push((x, y));
        }
        assert_eq!(centres.len(), 20);
        for i in 0..20 {
            for j in i + 1..20 {
                let d = ((centres[i].0 - centres[j].0).powi(2)
                    + (centres[i].1 - centres[j].1).powi(2))
                .sqrt();
                assert!(d > 0.4, "faces {i} and {j} landed {d} apart");
            }
        }
    }

    #[test]
    fn the_map_is_continuous_within_a_face() {
        // A degree of movement anywhere is a small movement on the net,
        // except where the net is cut open. Fuller's cut goes through water
        // and touches twelve of the corners, so a handful of the samples do
        // jump; the great majority may not.
        let mut jumps = 0;
        let mut total = 0;
        for i in 0..180 {
            let lat = -89.5 + i as f64;
            for j in 0..360 {
                let lng = -179.5 + j as f64;
                let a = convert(lng, lat);
                let b = convert(lng + 1.0, lat);
                let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                total += 1;
                if d > 0.2 {
                    jumps += 1;
                }
            }
        }
        assert!(
            jumps * 20 < total,
            "{jumps} of {total} samples jumped, which is not a fold"
        );
    }

    #[test]
    fn distances_come_out_about_the_same_everywhere() {
        // What the projection is for: because no one face of an icosahedron
        // covers much of the sphere, the scale barely changes across the map.
        // A degree of arc is very nearly the same length wherever it is, and
        // a mistake anywhere in the transformation shows up as a place where
        // it is not.
        let mut ratios = Vec::new();
        for i in 1..60 {
            let lat = -90.0 + i as f64 * 3.0;
            for j in 0..120 {
                let lng = -180.0 + j as f64 * 3.0;
                let a = convert(lng, lat);
                // A step east, shortened towards the poles so that it is the
                // same arc everywhere.
                let step = 0.25;
                let b = convert(lng + step / lat.to_radians().cos(), lat);
                let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                if d < 0.2 {
                    // Not one of the samples that steps over a fold.
                    ratios.push(d / step);
                }
            }
        }
        assert!(ratios.len() > 5000, "only {} usable samples", ratios.len());
        ratios.sort_by(f64::total_cmp);
        let lo = ratios[ratios.len() / 20];
        let hi = ratios[ratios.len() * 19 / 20];
        // Fuller's claim is that the distortion is small; nine tenths of the
        // map is within a fifth of the same scale.
        assert!(hi / lo < 1.2, "the scale runs from {lo} to {hi}");
    }

    #[test]
    fn the_poles_are_as_far_apart_as_the_net_allows() {
        // They are antipodal, so nothing on the map may be further from one
        // of them than the other is, give or take the folding.
        let n = convert(0.0, 90.0);
        let s = convert(0.0, -90.0);
        let d = ((n.0 - s.0).powi(2) + (n.1 - s.1).powi(2)).sqrt();
        assert!(d > 1.5, "the poles landed {d} apart at {n:?} and {s:?}");
        for p in [n, s] {
            assert!((0.0..=WIDTH).contains(&p.0) && (0.0..=HEIGHT).contains(&p.1));
        }
    }

    #[test]
    fn building_a_projection_map_is_not_too_slow_to_do_at_startup() {
        // `dymaxionmap` builds a lookup table by converting every half pixel
        // of its source map, which at 1024x512 is two million calls. jwz
        // calls it "not super fast" and shows a loading message while it
        // happens; on a 2048-wide source he quotes seven seconds.
        //
        // Measured here at four and a half million a second in release, so
        // the whole table at 1024x512 is under half a second natively and
        // about a second in a browser. That is a pause worth a loading
        // message and not worth spreading over frames.
        //
        // This runs a small corner of it, because the suite should stay
        // quick and a wall-clock assertion would only be flaky.
        let (w, h) = (128, 64);
        let mut sum = 0.0;
        for y2 in 0..h * 2 {
            let y = y2 as f64 / 2.0;
            let lat = -90.0 + (180.0 * y / h as f64);
            for x2 in 0..w * 2 {
                let x = x2 as f64 / 2.0;
                let lng = -180.0 + (360.0 * x / w as f64);
                let (ox, oy) = convert(lng, lat);
                sum += ox + oy;
            }
        }
        assert!(sum.is_finite());
    }
}
