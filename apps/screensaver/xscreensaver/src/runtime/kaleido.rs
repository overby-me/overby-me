//! Port of the `kaleido` half of `hacks/glx/polyhedra.c`.
//!
//! ```text
//! kaleido
//!
//!    Kaleidoscopic construction of uniform polyhedra
//!    Copyright (c) 1991-2002  Dr. Zvi Har'El <rl@math.technion.ac.il>
//!
//!    Redistribution and use in source and binary forms, with or without
//!    modification, are permitted provided that the following conditions
//!    are met:
//!
//!    1. Redistributions of source code must retain the above copyright
//!       notice, this list of conditions and the following disclaimer.
//!
//!    2. Redistributions in binary form must reproduce the above copyright
//!       notice, this list of conditions and the following disclaimer in
//!       the documentation and/or other materials provided with the
//!       distribution.
//!
//!    3. The end-user documentation included with the redistribution,
//!       if any, must include the following acknowledgment:
//!        "This product includes software developed by
//!         Dr. Zvi Har'El (http://www.math.technion.ac.il/~rl/)."
//!       Alternately, this acknowledgment may appear in the software itself,
//!       if and wherever such third-party acknowledgments normally appear.
//!
//!    This software is provided 'as-is', without any express or implied
//!    warranty.  In no event will the author be held liable for any
//!    damages arising from the use of this software.
//! ```
//!
//! The eighty uniform polyhedra, worked out rather than modelled. Each one is
//! written as a *Wythoff symbol*: three numbers and a bar, saying where the
//! mirrors of a kaleidoscope stand and which of them a point sits on. `2 5|2`
//! is the pentagonal prism; `|2 3 5` is the snub dodecahedron; the numbers can
//! be fractions, and `5/2` means a pentagram, which is how the star
//! polyhedra get in.
//!
//! The construction is spherical trigonometry. The three numbers give a
//! triangle on the sphere whose angles are `pi/p`, `pi/q`, `pi/r`; reflecting
//! it in its own sides tiles the sphere with `g` copies of it, and the orbit
//! of one point under those reflections is the polyhedron's vertices. Which
//! point depends on where the bar is. `moebius` finds the triangle, its
//! symmetry group and its density; `decompose` splits it into the right
//! triangles that a vertex figure is made of; `newton` solves for the angles
//! those triangles must have, by Newton's method, since there is no closed
//! form; `vertices` then walks the vertex graph outwards from the north pole,
//! rotating each vertex's neighbours into place, and `faces` finds the faces
//! as the poles of the planes through them.
//!
//! Two of the eighty do not come out of the machinery and are patched up
//! afterwards by `exceptions`: the ones whose Wythoff symbol has an even
//! denominator, where a face has to be removed and two retrograde ones added,
//! and the great dirhombicosidodecahedron, which has no Wythoff symbol at all
//! and is built from a related solid by hand.
//!
//! Upstream also computes an edge list; nothing asks for one, so that pass is
//! not here. Its name-guessing for symbols that are not in the table is not
//! here either, since everything is asked for by table index.

use std::f64::consts::PI;

const DBL_EPSILON: f64 = 2.220_446_049_250_313e-16;
const BIG_EPSILON: f64 = 3e-2;

/// One row of the table: a Wythoff symbol and the names it goes by.
pub struct Uniform {
    pub wythoff: &'static str,
    pub name: &'static str,
    pub dual: &'static str,
    pub group: &'static str,
    pub class: &'static str,
    pub dual_class: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vector {
    pub fn new(x: f64, y: f64, z: f64) -> Vector {
        Vector { x, y, z }
    }

    pub fn dot(self, b: Vector) -> f64 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }

    pub fn scale(self, k: f64) -> Vector {
        Vector::new(k * self.x, k * self.y, k * self.z)
    }

    pub fn diff(self, b: Vector) -> Vector {
        Vector::new(self.x - b.x, self.y - b.y, self.z - b.z)
    }

    pub fn sum(self, b: Vector) -> Vector {
        Vector::new(self.x + b.x, self.y + b.y, self.z + b.z)
    }

    pub fn cross(self, b: Vector) -> Vector {
        Vector::new(
            self.y * b.z - self.z * b.y,
            self.z * b.x - self.x * b.z,
            self.x * b.y - self.y * b.x,
        )
    }

    fn sum3(a: Vector, b: Vector, c: Vector) -> Vector {
        a.sum(b).sum(c)
    }

    fn same(self, b: Vector, epsilon: f64) -> bool {
        (self.x - b.x).abs() < epsilon
            && (self.y - b.y).abs() < epsilon
            && (self.z - b.z).abs() < epsilon
    }
}

pub fn rotate(vertex: Vector, axis: Vector, angle: f64) -> Vector {
    let p = axis.scale(axis.dot(vertex));
    Vector::sum3(
        p,
        vertex.diff(p).scale(angle.cos()),
        axis.cross(vertex).scale(angle.sin()),
    )
}

/// The polar reciprocal of the plane through `a`, `b` and `c`.
///
/// If the plane does not contain the origin, return `p` with
/// `dot(p,a) = dot(p,b) = dot(p,c) = r`; otherwise the unit normal.
fn pole(r: f64, a: Vector, b: Vector, c: Vector) -> Vector {
    let p = b.diff(a).cross(c.diff(a));
    let k = p.dot(a);
    if k.abs() < 1e-6 {
        p.scale(1.0 / p.dot(p).sqrt())
    } else {
        p.scale(r / k)
    }
}

/// The mathematical modulus function.
fn imod(i: i64, j: i64) -> i64 {
    let i = i % j;
    if i >= 0 {
        i
    } else if j < 0 {
        i - j
    } else {
        i + j
    }
}

#[derive(Clone, Copy, Debug)]
struct Fraction {
    n: i64,
    d: i64,
}

/// Find the numerator and the denominator using the Euclidean algorithm.
fn frac(x: f64) -> Fraction {
    let zero = Fraction { n: 0, d: 1 };
    let inf = Fraction { n: 1, d: 0 };
    let mut r = zero;
    let mut out = inf;
    let mut s = x;
    loop {
        if s.abs() > i32::MAX as f64 {
            return out;
        }
        let f = s.floor() as i64;
        let r0 = r;
        r = out;
        out = Fraction {
            n: out.n * f + r0.n,
            d: out.d * f + r0.d,
        };
        if x == out.n as f64 / out.d as f64 {
            return out;
        }
        s = 1.0 / (s - f as f64);
    }
}

fn numerator(x: f64) -> i64 {
    frac(x).n
}

fn denominator(x: f64) -> i64 {
    frac(x).d
}

/// `n/d` becomes `n/(n-d)`: the retrograde of a star polygon.
fn compl(x: f64) -> f64 {
    let f = frac(x);
    f.n as f64 / (f.n - f.d) as f64
}

/// A fraction written out, which is how a Wythoff symbol and a vertex
/// configuration are printed.
fn sprintfrac(x: f64) -> String {
    let f = frac(x);
    if f.d == 0 {
        "infinity".to_string()
    } else if f.d == 1 {
        format!("{}", f.n)
    } else {
        format!("{}/{}", f.n, f.d)
    }
}

/// Everything the construction works out about one polyhedron.
pub struct Polyhedron {
    /// Index into [`UNIFORM`].
    pub index: usize,
    /// Number of face types, at most five.
    pub n_types: usize,
    /// Vertex valency.
    pub m: usize,
    /// Vertex, edge and face counts.
    pub nv: usize,
    pub ne: usize,
    pub nf: usize,
    /// Density: how many times the faces wrap the centre.
    pub density: i32,
    /// Euler characteristic, `V - E + F`.
    pub chi: i32,
    /// Order of the symmetry group, and its type: 2 dihedral, 3 tetrahedral,
    /// 4 octahedral, 5 icosahedral.
    pub g: i32,
    pub k: i32,
    /// Has equatorial faces, so some of its planes pass through the centre.
    pub hemi: bool,
    pub onesided: bool,
    /// Which face the `p q r|` postprocessing removed, or -1.
    pub even: i32,
    /// Face counts by type.
    pub fi: Vec<i32>,
    /// Vertex configuration: which face type each step round a vertex is.
    pub rot: Vec<usize>,
    /// Snub triangle configuration.
    pub snub: Vec<i32>,
    firstrot: Vec<usize>,
    /// Face types, one per face.
    pub ftype: Vec<usize>,
    /// Vertex-face incidence, `M` by `V`.
    pub incid: Vec<Vec<i32>>,
    /// Vertex-vertex adjacency, `M` by `V`.
    pub adj: Vec<Vec<i32>>,
    /// p, q and r, with the bar as a zero.
    pub p: [f64; 4],
    /// Smallest nonzero inradius.
    pub minr: f64,
    /// Sides of a face of each type, and faces at a vertex of each type.
    pub n: Vec<f64>,
    pub mm: Vec<f64>,
    /// Fundamental angles, in radians.
    pub gamma: Vec<f64>,
    /// The Wythoff symbol and the vertex configuration, written out.
    pub polyform: String,
    pub config: String,
    /// Vertex and face coordinates.
    pub v: Vec<Vector>,
    pub f: Vec<Vector>,
}

impl Polyhedron {
    fn new() -> Polyhedron {
        Polyhedron {
            index: 0,
            n_types: 0,
            m: 0,
            nv: 0,
            ne: 0,
            nf: 0,
            density: 0,
            chi: 0,
            g: 0,
            k: 2,
            hemi: false,
            onesided: false,
            even: -1,
            fi: Vec::new(),
            rot: Vec::new(),
            snub: Vec::new(),
            firstrot: Vec::new(),
            ftype: Vec::new(),
            incid: Vec::new(),
            adj: Vec::new(),
            p: [0.0; 4],
            minr: 0.0,
            n: Vec::new(),
            mm: Vec::new(),
            gamma: Vec::new(),
            polyform: String::new(),
            config: String::new(),
            v: Vec::new(),
            f: Vec::new(),
        }
    }

    pub fn name(&self) -> &'static str {
        UNIFORM[self.index].name
    }
    pub fn dual_name(&self) -> &'static str {
        UNIFORM[self.index].dual
    }
    pub fn group(&self) -> &'static str {
        UNIFORM[self.index].group
    }
    pub fn class(&self) -> &'static str {
        UNIFORM[self.index].class
    }
    pub fn dual_class(&self) -> &'static str {
        UNIFORM[self.index].dual_class
    }
}

/// The last entry, `|3/2 5/3 3 5/2`, which is not Wythoffian and is special
/// cased in three places.
fn last_uniform() -> usize {
    UNIFORM.len()
}

/// Unpack a Wythoff symbol: three fractions and a bar in some order. A bar is
/// stored as a zero, so which of the four slots is zero says which of the four
/// forms this is.
fn unpacksym(index: usize, p: &mut Polyhedron) -> bool {
    let sym = UNIFORM[index].wythoff;
    p.index = index;
    let mut i = 0;
    let mut bars = 0;
    let b = sym.as_bytes();
    let mut at = 0;
    loop {
        while at < b.len() && b[at].is_ascii_whitespace() {
            at += 1;
        }
        if at >= b.len() {
            return i == 4 && (bars > 0 || p.index == last_uniform() - 1);
        }
        if i == 4 {
            return false;
        }
        if b[at] == b'|' {
            at += 1;
            bars += 1;
            if bars > 1 {
                return false;
            }
            p.p[i] = 0.0;
            i += 1;
            continue;
        }
        if !b[at].is_ascii_digit() {
            return false;
        }
        let mut n: i64 = 0;
        while at < b.len() && b[at].is_ascii_digit() {
            n = n * 10 + (b[at] - b'0') as i64;
            at += 1;
        }
        while at < b.len() && b[at].is_ascii_whitespace() {
            at += 1;
        }
        if at >= b.len() || b[at] != b'/' {
            p.p[i] = n as f64;
            if p.p[i] <= 1.0 {
                return false;
            }
            i += 1;
            continue;
        }
        at += 1;
        while at < b.len() && b[at].is_ascii_whitespace() {
            at += 1;
        }
        if at >= b.len() || !b[at].is_ascii_digit() {
            return false;
        }
        let mut d: i64 = 0;
        while at < b.len() && b[at].is_ascii_digit() {
            d = d * 10 + (b[at] - b'0') as i64;
            at += 1;
        }
        if d == 0 {
            return false;
        }
        p.p[i] = n as f64 / d as f64;
        if p.p[i] <= 1.0 {
            return false;
        }
        i += 1;
    }
}

/// Find the Moebius triangle of the Schwarz triangle, the order `g` of its
/// symmetry group, its Euler characteristic and its covering density.
fn moebius(p: &mut Polyhedron) -> bool {
    let mut twos = 0;
    p.k = 2;
    p.polyform = if p.index == last_uniform() - 1 {
        "|".to_string()
    } else {
        String::new()
    };

    for j in 0..4 {
        if p.p[j] != 0.0 {
            let s = sprintfrac(p.p[j]);
            if j > 0 && p.p[j - 1] != 0.0 {
                p.polyform.push(' ');
            }
            p.polyform.push_str(&s);
            if p.p[j] != 2.0 {
                let k = numerator(p.p[j]) as i32;
                if k > p.k {
                    if p.k == 4 {
                        break;
                    }
                    p.k = k;
                } else if k < p.k && k == 4 {
                    break;
                }
            } else {
                twos += 1;
            }
        } else {
            p.polyform.push('|');
        }
    }

    if twos >= 2 {
        /* dihedral */
        p.g = 4 * p.k;
        p.k = 2;
    } else {
        if p.k > 5 {
            return false;
        }
        p.g = 24 * p.k / (6 - p.k);
    }

    if p.index != last_uniform() - 1 {
        p.density = -p.g;
        p.chi = -p.g;
        for j in 0..4 {
            if p.p[j] != 0.0 {
                let i = p.g / numerator(p.p[j]) as i32;
                p.chi += i;
                p.density += i * denominator(p.p[j]) as i32;
            }
        }
        p.chi /= 2;
        p.density /= 4;
        if p.density <= 0 {
            return false;
        }
    }
    true
}

/// Split the Schwarz triangle into `N` right triangles and find the vertex
/// count and valency.
fn decompose(p: &mut Polyhedron) -> bool {
    if p.p[1] == 0.0 {
        /* p|q r */
        p.n_types = 2;
        p.m = 2 * numerator(p.p[0]) as usize;
        p.nv = (p.g as usize) / p.m;
        p.n = vec![0.0; p.n_types];
        p.mm = vec![0.0; p.n_types];
        p.rot = Vec::with_capacity(p.m);
        for j in 0..2 {
            p.n[j] = p.p[j + 2];
            p.mm[j] = p.p[0];
        }
        for _ in 0..p.m / 2 {
            p.rot.push(0);
            p.rot.push(1);
        }
    } else if p.p[2] == 0.0 {
        /* p q|r */
        p.n_types = 3;
        p.m = 4;
        p.nv = p.g as usize / 2;
        p.n = vec![0.0; p.n_types];
        p.mm = vec![0.0; p.n_types];
        p.rot = Vec::with_capacity(p.m);
        p.n[0] = 2.0 * p.p[3];
        p.mm[0] = 2.0;
        for j in 1..3 {
            p.n[j] = p.p[j - 1];
            p.mm[j] = 1.0;
            p.rot.push(0);
            p.rot.push(j);
        }
        if (p.p[0] - compl(p.p[1])).abs() < DBL_EPSILON {
            /* p = q' */
            p.hemi = true;
            p.density = 0;
            if p.p[0] != 2.0 && !(p.p[3] == 3.0 && (p.p[0] == 3.0 || p.p[1] == 3.0)) {
                p.onesided = true;
                p.nv /= 2;
                p.chi /= 2;
            }
        }
    } else if p.p[3] == 0.0 {
        /* p q r| */
        p.n_types = 3;
        p.m = 3;
        p.nv = p.g as usize;
        p.n = vec![0.0; p.n_types];
        p.mm = vec![0.0; p.n_types];
        p.rot = Vec::with_capacity(p.m);
        for j in 0..3 {
            if denominator(p.p[j]) % 2 == 0 {
                // What happens if there is more than one even denominator?
                if p.p[(j + 1) % 3] != p.p[(j + 2) % 3] {
                    /* needs postprocessing */
                    p.even = j as i32; /* memorize the removed face */
                    p.chi -= p.g / numerator(p.p[j]) as i32 / 2;
                    p.onesided = true;
                    p.density = 0;
                } else {
                    /* for p = q we get a double 2 2r|p */
                    p.density /= 2;
                }
                p.nv /= 2;
            }
            p.n[j] = 2.0 * p.p[j];
            p.mm[j] = 1.0;
            p.rot.push(j);
        }
    } else {
        /* |p q r - snub polyhedron */
        p.n_types = 4;
        p.m = 6;
        /* Only "white" triangles carry a vertex */
        p.nv = p.g as usize / 2;
        p.n = vec![0.0; p.n_types];
        p.mm = vec![0.0; p.n_types];
        p.rot = Vec::with_capacity(p.m);
        p.snub = Vec::with_capacity(p.m);
        p.mm[0] = 3.0;
        p.n[0] = 3.0;
        for j in 1..4 {
            p.n[j] = p.p[j];
            p.mm[j] = 1.0;
            p.rot.push(0);
            p.rot.push(j);
            p.snub.push(1);
            p.snub.push(0);
        }
    }

    // Sort the fundamental triangles by decreasing n[i], pushing the trivial
    // ones (n[i] = 2) to the end.
    let mut jj = p.n_types as i32 - 1;
    while jj != 0 {
        let last = jj;
        jj = 0;
        for j in 0..last as usize {
            if (p.n[j] < p.n[j + 1] || p.n[j] == 2.0) && p.n[j + 1] != 2.0 {
                p.n.swap(j, j + 1);
                p.mm.swap(j, j + 1);
                for r in &mut p.rot {
                    if *r == j {
                        *r = j + 1;
                    } else if *r == j + 1 {
                        *r = j;
                    }
                }
                if p.even != -1 {
                    if p.even == j as i32 {
                        p.even = j as i32 + 1;
                    } else if p.even == j as i32 + 1 {
                        p.even = j as i32;
                    }
                }
                jj = j as i32;
            }
        }
    }

    /* Get rid of repeated triangles. */
    let mut big_j = 0;
    while big_j < p.n_types && p.n[big_j] != 2.0 {
        let mut j = big_j + 1;
        while j < p.n_types && p.n[j] == p.n[big_j] {
            p.mm[big_j] += p.mm[j];
            j += 1;
        }
        let k = j - big_j - 1;
        if k != 0 {
            for i in j..p.n_types {
                p.n[i - k] = p.n[i];
                p.mm[i - k] = p.mm[i];
            }
            p.n_types -= k;
            for r in &mut p.rot {
                if *r >= j {
                    *r -= k;
                } else if *r > big_j {
                    *r = big_j;
                }
            }
            if p.even >= j as i32 {
                p.even -= k as i32;
            }
        }
        big_j += 1;
    }

    /* Get rid of trivial triangles. */
    if big_j == 0 {
        big_j = 1; /* hosohedron */
    }
    if big_j < p.n_types {
        p.n_types = big_j;
        let mut i = 0;
        while i < p.m {
            if p.rot[i] >= p.n_types {
                p.rot.remove(i);
                if !p.snub.is_empty() {
                    p.snub.remove(i);
                }
                p.m -= 1;
            } else {
                i += 1;
            }
        }
    }

    p.n.truncate(p.n_types);
    p.mm.truncate(p.n_types);
    p.rot.truncate(p.m);
    if !p.snub.is_empty() {
        p.snub.truncate(p.m);
    }
    true
}

/// Solve the fundamental right spherical triangles by Newton's method.
fn newton(p: &mut Polyhedron) -> bool {
    p.gamma = vec![0.0; p.n_types];
    if p.n_types == 1 {
        p.gamma[0] = PI / p.mm[0];
        return true;
    }
    for j in 0..p.n_types {
        p.gamma[j] = PI / 2.0 - PI / p.n[j];
    }
    for _ in 0..1000 {
        let mut delta = PI;
        let mut sigma = 0.0;
        for j in 0..p.n_types {
            delta -= p.mm[j] * p.gamma[j];
        }
        if delta.abs() < 11.0 * DBL_EPSILON {
            return true;
        }
        for j in 0..p.n_types {
            sigma += p.mm[j] * p.gamma[j].tan();
        }
        p.gamma[0] += delta * p.gamma[0].tan() / sigma;
        if p.gamma[0] < 0.0 || p.gamma[0] > PI {
            return false;
        }
        let cosa = (PI / p.n[0]).cos() / p.gamma[0].sin();
        for j in 1..p.n_types {
            p.gamma[j] = ((PI / p.n[j]).cos() / cosa).asin();
            if !p.gamma[j].is_finite() {
                return false;
            }
        }
    }
    // Upstream loops until it converges. It always does, but a loop with no
    // bound in a browser is a hang, so this one gives up instead.
    false
}

/// Postprocess the two polyhedra the machinery does not produce directly.
fn exceptions(p: &mut Polyhedron) {
    // `p q r|` where r has an even denominator: remove the {2r} and add a
    // retrograde {2p} and {2q}.
    if p.even != -1 {
        p.m = 4;
        p.n_types = 4;
        p.n.resize(4, 0.0);
        p.mm.resize(4, 0.0);
        p.gamma.resize(4, 0.0);
        p.rot.resize(4, 0);
        for j in (p.even + 1) as usize..3 {
            p.n[j - 1] = p.n[j];
            p.gamma[j - 1] = p.gamma[j];
        }
        p.n[2] = compl(p.n[1]);
        p.gamma[2] = -p.gamma[1];
        p.n[3] = compl(p.n[0]);
        p.mm[3] = 1.0;
        p.gamma[3] = -p.gamma[0];
        p.rot[0] = 0;
        p.rot[1] = 1;
        p.rot[2] = 3;
        p.rot[3] = 2;
    }

    // The last one, |3/2 5/3 3 5/2: take a |5/3 3 5/2, replace the three snub
    // triangles by four equatorial squares and add the missing {3/2}.
    if p.index == last_uniform() - 1 {
        p.n_types = 5;
        p.m = 8;
        p.n.resize(5, 0.0);
        p.mm.resize(5, 0.0);
        p.gamma.resize(5, 0.0);
        p.rot.resize(8, 0);
        p.snub.resize(8, 0);
        p.hemi = true;
        p.density = 0;
        for j in (1..4).rev() {
            p.mm[j] = 1.0;
            p.n[j] = p.n[j - 1];
            p.gamma[j] = p.gamma[j - 1];
        }
        p.mm[0] = 4.0;
        p.n[0] = 4.0;
        p.gamma[0] = PI / 2.0;
        p.mm[4] = 1.0;
        p.n[4] = compl(p.n[1]);
        p.gamma[4] = -p.gamma[1];
        let mut j = 1;
        while j < 6 {
            p.rot[j] += 1;
            j += 2;
        }
        p.rot[6] = 0;
        p.rot[7] = 4;
        p.snub[6] = 1;
        p.snub[7] = 0;
    }
}

/// Count edges and faces, and update the density and characteristic where the
/// polyhedron's differ from its Schwarz triangle's.
fn count(p: &mut Polyhedron) {
    p.fi = vec![0; p.n_types];
    for j in 0..p.n_types {
        let temp = p.nv as i32 * numerator(p.mm[j]) as i32;
        p.ne += temp as usize;
        p.fi[j] = temp / numerator(p.n[j]) as i32;
        p.nf += p.fi[j] as usize;
    }
    p.ne /= 2;
    if p.density != 0 && p.gamma[0] > PI / 2.0 {
        p.density = p.fi[0] - p.density;
    }
    if p.index == last_uniform() - 1 {
        p.chi = p.nv as i32 - p.ne as i32 + p.nf as i32;
    }
}

/// Write out the vertex configuration.
fn configuration(p: &mut Polyhedron) {
    let mut out = String::new();
    for j in 0..p.m {
        if j > 0 {
            out.push_str(", ");
        }
        out.push_str(&sprintfrac(p.n[p.rot[j]]));
    }
    let d = denominator(p.mm[0]);
    if d != 1 {
        out.push_str(&format!("/{d}"));
    }
    p.config = out;
}

/// Compute the vertices and the vertex adjacency lists, by breadth-first
/// search out from the north pole: each vertex's neighbours are its parent's
/// rotated by a cyclic sequence of the fundamental angles.
fn vertices(p: &mut Polyhedron) -> bool {
    let mut new_v = 2;
    p.v = vec![Vector::default(); p.nv];
    p.adj = vec![vec![0i32; p.nv]; p.m];
    p.firstrot = vec![0usize; p.nv];

    let cosa = (PI / p.n[0]).cos() / p.gamma[0].sin();
    p.v[0] = Vector::new(0.0, 0.0, 1.0);
    p.firstrot[0] = 0;
    p.adj[0][0] = 1;
    p.v[1] = Vector::new(
        2.0 * cosa * (1.0 - cosa * cosa).sqrt(),
        0.0,
        2.0 * cosa * cosa - 1.0,
    );
    if p.snub.is_empty() {
        p.firstrot[1] = 0;
        p.adj[0][1] = -1; /* start the other side */
        p.adj[p.m - 1][1] = 0;
    } else {
        p.firstrot[1] = if p.snub[p.m - 1] != 0 { 0 } else { p.m - 1 };
        p.adj[0][1] = 0;
    }

    let mut i = 0;
    while i < new_v {
        let (one, start, limit): (i32, i32, i32) = if p.adj[0][i] == -1 {
            (-1, p.m as i32 - 2, -1)
        } else {
            (1, 1, p.m as i32)
        };
        let mut k = p.firstrot[i];
        let mut j = start;
        while j != limit {
            let from = p.adj[(j - one) as usize][i];
            let temp = rotate(
                p.v[from as usize],
                p.v[i],
                one as f64 * 2.0 * p.gamma[p.rot[k]],
            );
            let mut big_j = 0;
            while big_j < new_v && !p.v[big_j].same(temp, BIG_EPSILON) {
                big_j += 1;
            }
            p.adj[j as usize][i] = big_j as i32;
            let last = k;
            k += 1;
            if k == p.m {
                k = 0;
            }
            if big_j == new_v {
                /* new vertex */
                if new_v == p.nv {
                    return false;
                }
                p.v[new_v] = temp;
                new_v += 1;
                if p.snub.is_empty() {
                    p.firstrot[big_j] = k;
                    if one > 0 {
                        p.adj[0][big_j] = -1;
                        p.adj[p.m - 1][big_j] = i as i32;
                    } else {
                        p.adj[0][big_j] = i as i32;
                    }
                } else {
                    p.firstrot[big_j] = if p.snub[last] == 0 {
                        last
                    } else if p.snub[k] == 0 {
                        (k + 1) % p.m
                    } else {
                        k
                    };
                    p.adj[0][big_j] = i as i32;
                }
            }
            j += one;
        }
        i += 1;
    }
    true
}

/// Compute the faces, which are the poles of the planes through the vertices
/// around them, and the vertex-face incidence.
fn faces(p: &mut Polyhedron) -> bool {
    let mut new_f = 0;
    p.f = vec![Vector::default(); p.nf];
    p.ftype = vec![0usize; p.nf];
    p.incid = vec![vec![-1i32; p.nv]; p.m];
    let h = if p.hemi { 1 } else { 0 };
    p.minr = 1.0 / ((PI / p.n[h]).tan() * p.gamma[h].tan()).abs();

    for i in 0..p.nv {
        for j in 0..p.m {
            if p.incid[j][i] != -1 {
                continue;
            }
            p.incid[j][i] = new_f as i32;
            if new_f == p.nf {
                return false;
            }
            p.f[new_f] = pole(
                p.minr,
                p.v[i],
                p.v[p.adj[j][i] as usize],
                p.v[p.adj[imod(j as i64 + 1, p.m as i64) as usize][i] as usize],
            );
            let off = if p.adj[0][i] < p.adj[p.m - 1][i] {
                j as i64
            } else {
                -(j as i64) - 2
            };
            p.ftype[new_f] = p.rot[imod(p.firstrot[i] as i64 + off, p.m as i64) as usize];

            /* papillon edge type */
            let pap = if p.onesided {
                (p.firstrot[i] + j) % 2
            } else {
                0
            };
            let mut i0 = i;
            let mut big_j = j;
            loop {
                let k = i0;
                i0 = p.adj[big_j][k] as usize;
                if i0 == i {
                    break;
                }
                big_j = 0;
                while big_j < p.m && p.adj[big_j][i0] != k as i32 {
                    big_j += 1;
                }
                if big_j == p.m {
                    return false;
                }
                if p.onesided && (big_j + p.firstrot[i0]) % 2 == pap {
                    p.incid[big_j][i0] = new_f as i32;
                    big_j += 1;
                    if big_j >= p.m {
                        big_j = 0;
                    }
                } else {
                    if big_j == 0 {
                        big_j = p.m - 1;
                    } else {
                        big_j -= 1;
                    }
                    p.incid[big_j][i0] = new_f as i32;
                }
            }
            new_f += 1;
        }
    }
    true
}

/// Construct one of the eighty, by its index into [`UNIFORM`].
pub fn kaleido(index: usize) -> Option<Polyhedron> {
    if index >= UNIFORM.len() {
        return None;
    }
    let mut p = Polyhedron::new();
    if !unpacksym(index, &mut p) {
        return None;
    }
    if !moebius(&mut p) {
        return None;
    }
    if !decompose(&mut p) {
        return None;
    }
    if !newton(&mut p) {
        return None;
    }
    exceptions(&mut p);
    count(&mut p);
    configuration(&mut p);
    if !vertices(&mut p) {
        return None;
    }
    if !faces(&mut p) {
        return None;
    }
    Some(p)
}
/// Upstream's `uniform[]`: the eighty uniform polyhedra, their Wythoff
/// symbols and the names they go by. The Coxeter and Wenninger numbers
/// upstream carries alongside are not used by anything and are not here.
pub const UNIFORM: &[Uniform] = &[
    Uniform {
        wythoff: "2 5|2",
        name: "Pentagonal Prism",
        dual: "Pentagonal Dipyramid",
        group: "Dihedral (D[1/5])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|2 2 5",
        name: "Pentagonal Antiprism",
        dual: "Pentagonal Deltohedron",
        group: "Dihedral (D[1/5])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "2 5/2|2",
        name: "Pentagrammic Prism",
        dual: "Pentagrammic Dipyramid",
        group: "Dihedral (D[2/5])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|2 2 5/2",
        name: "Pentagrammic Antiprism",
        dual: "Pentagrammic Deltohedron",
        group: "Dihedral (D[2/5])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|2 2 5/3",
        name: "Pentagrammic Crossed Antiprism",
        dual: "Pentagrammic Concave Deltohedron",
        group: "Dihedral (D[3/5])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3|2 3",
        name: "Tetrahedron",
        dual: "Tetrahedron",
        group: "Tetrahedral (T[1])",
        class: "Platonic Solid",
        dual_class: "Platonic Solid",
    },
    Uniform {
        wythoff: "2 3|3",
        name: "Truncated Tetrahedron",
        dual: "Triakistetrahedron",
        group: "Tetrahedral (T[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "3/2 3|3",
        name: "Octahemioctahedron",
        dual: "Octahemioctacron",
        group: "Tetrahedral (T[2])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 3|2",
        name: "Tetrahemihexahedron",
        dual: "Tetrahemihexacron",
        group: "Tetrahedral (T[3])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "4|2 3",
        name: "Octahedron",
        dual: "Cube",
        group: "Octahedral (O[1])",
        class: "Platonic Solid",
        dual_class: "Platonic Solid",
    },
    Uniform {
        wythoff: "3|2 4",
        name: "Cube",
        dual: "Octahedron",
        group: "Octahedral (O[1])",
        class: "Platonic Solid",
        dual_class: "Platonic Solid",
    },
    Uniform {
        wythoff: "2|3 4",
        name: "Cuboctahedron",
        dual: "Rhombic Dodecahedron",
        group: "Octahedral (O[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "2 4|3",
        name: "Truncated Octahedron",
        dual: "Tetrakishexahedron",
        group: "Octahedral (O[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "2 3|4",
        name: "Truncated Cube",
        dual: "Triakisoctahedron",
        group: "Octahedral (O[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "3 4|2",
        name: "Rhombicuboctahedron",
        dual: "Deltoidal Icositetrahedron",
        group: "Octahedral (O[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "2 3 4|",
        name: "Truncated Cuboctahedron",
        dual: "Disdyakisdodecahedron",
        group: "Octahedral (O[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "|2 3 4",
        name: "Snub Cube",
        dual: "Pentagonal Icositetrahedron",
        group: "Octahedral (O[1]), Chiral",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "3/2 4|4",
        name: "Small Cubicuboctahedron",
        dual: "Small Hexacronic Icositetrahedron",
        group: "Octahedral (O[2b])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3 4|4/3",
        name: "Great Cubicuboctahedron",
        dual: "Great Hexacronic Icositetrahedron",
        group: "Octahedral (O[4])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "4/3 4|3",
        name: "Cubohemioctahedron",
        dual: "Hexahemioctacron",
        group: "Octahedral (O[4])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "4/3 3 4|",
        name: "Cubitruncated Cuboctahedron",
        dual: "Tetradyakishexahedron",
        group: "Octahedral (O[4])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 4|2",
        name: "Great Rhombicuboctahedron",
        dual: "Great Deltoidal Icositetrahedron",
        group: "Octahedral (O[5])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 2 4|",
        name: "Small Rhombihexahedron",
        dual: "Small Rhombihexacron",
        group: "Octahedral (O[5])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "2 3|4/3",
        name: "Stellated Truncated Hexahedron",
        dual: "Great Triakisoctahedron",
        group: "Octahedral (O[7])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "4/3 2 3|",
        name: "Great Truncated Cuboctahedron",
        dual: "Great Disdyakisdodecahedron",
        group: "Octahedral (O[7])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "4/3 3/2 2|",
        name: "Great Rhombihexahedron",
        dual: "Great Rhombihexacron",
        group: "Octahedral (O[11])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5|2 3",
        name: "Icosahedron",
        dual: "Dodecahedron",
        group: "Icosahedral (I[1])",
        class: "Platonic Solid",
        dual_class: "Platonic Solid",
    },
    Uniform {
        wythoff: "3|2 5",
        name: "Dodecahedron",
        dual: "Icosahedron",
        group: "Icosahedral (I[1])",
        class: "Platonic Solid",
        dual_class: "Platonic Solid",
    },
    Uniform {
        wythoff: "2|3 5",
        name: "Icosidodecahedron",
        dual: "Rhombic Triacontahedron",
        group: "Icosahedral (I[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "2 5|3",
        name: "Truncated Icosahedron",
        dual: "Pentakisdodecahedron",
        group: "Icosahedral (I[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "2 3|5",
        name: "Truncated Dodecahedron",
        dual: "Triakisicosahedron",
        group: "Icosahedral (I[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "3 5|2",
        name: "Rhombicosidodecahedron",
        dual: "Deltoidal Hexecontahedron",
        group: "Icosahedral (I[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "2 3 5|",
        name: "Truncated Icosidodecahedron",
        dual: "Disdyakistriacontahedron",
        group: "Icosahedral (I[1])",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "|2 3 5",
        name: "Snub Dodecahedron",
        dual: "Pentagonal Hexecontahedron",
        group: "Icosahedral (I[1]), Chiral",
        class: "Archimedean Solid",
        dual_class: "Catalan Solid",
    },
    Uniform {
        wythoff: "3|5/2 3",
        name: "Small Ditrigonal Icosidodecahedron",
        dual: "Small Triambic Icosahedron",
        group: "Icosahedral (I[2a])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/2 3|3",
        name: "Small Icosicosidodecahedron",
        dual: "Small Icosacronic Hexecontahedron",
        group: "Icosahedral (I[2a])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|5/2 3 3",
        name: "Small Snub Icosicosidodecahedron",
        dual: "Small Hexagonal Hexecontahedron",
        group: "Icosahedral (I[2a])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 5|5",
        name: "Small Dodecicosidodecahedron",
        dual: "Small Dodecacronic Hexecontahedron",
        group: "Icosahedral (I[2b])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5|2 5/2",
        name: "Small Stellated Dodecahedron",
        dual: "Great Dodecahedron",
        group: "Icosahedral (I[3])",
        class: "Kepler-Poinsot Solid",
        dual_class: "Kepler-Poinsot Solid",
    },
    Uniform {
        wythoff: "5/2|2 5",
        name: "Great Dodecahedron",
        dual: "Small Stellated Dodecahedron",
        group: "Icosahedral (I[3])",
        class: "Kepler-Poinsot Solid",
        dual_class: "Kepler-Poinsot Solid",
    },
    Uniform {
        wythoff: "2|5/2 5",
        name: "Great Dodecadodecahedron",
        dual: "Medial Rhombic Triacontahedron",
        group: "Icosahedral (I[3])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "2 5/2|5",
        name: "Truncated Great Dodecahedron",
        dual: "Small Stellapentakisdodecahedron",
        group: "Icosahedral (I[3])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/2 5|2",
        name: "Rhombidodecadodecahedron",
        dual: "Medial Deltoidal Hexecontahedron",
        group: "Icosahedral (I[3])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "2 5/2 5|",
        name: "Small Rhombidodecahedron",
        dual: "Small Rhombidodecacron",
        group: "Icosahedral (I[3])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|2 5/2 5",
        name: "Snub Dodecadodecahedron",
        dual: "Medial Pentagonal Hexecontahedron",
        group: "Icosahedral (I[3])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3|5/3 5",
        name: "Ditrigonal Dodecadodecahedron",
        dual: "Medial Triambic Icosahedron",
        group: "Icosahedral (I[4])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3 5|5/3",
        name: "Great Ditrigonal Dodecicosidodecahedron",
        dual: "Great Ditrigonal Dodecacronic Hexecontahedron",
        group: "Icosahedral (I[4])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/3 3|5",
        name: "Small Ditrigonal Dodecicosidodecahedron",
        dual: "Small Ditrigonal Dodecacronic Hexecontahedron",
        group: "Icosahedral (I[4])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/3 5|3",
        name: "Icosidodecadodecahedron",
        dual: "Medial Icosacronic Hexecontahedron",
        group: "Icosahedral (I[4])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/3 3 5|",
        name: "Icositruncated Dodecadodecahedron",
        dual: "Tridyakisicosahedron",
        group: "Icosahedral (I[4])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|5/3 3 5",
        name: "Snub Icosidodecadodecahedron",
        dual: "Medial Hexagonal Hexecontahedron",
        group: "Icosahedral (I[4])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2|3 5",
        name: "Great Ditrigonal Icosidodecahedron",
        dual: "Great Triambic Icosahedron",
        group: "Icosahedral (I[6b])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 5|3",
        name: "Great Icosicosidodecahedron",
        dual: "Great Icosacronic Hexecontahedron",
        group: "Icosahedral (I[6b])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 3|5",
        name: "Small Icosihemidodecahedron",
        dual: "Small Icosihemidodecacron",
        group: "Icosahedral (I[6b])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 3 5|",
        name: "Small Dodecicosahedron",
        dual: "Small Dodecicosacron",
        group: "Icosahedral (I[6b])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/4 5|5",
        name: "Small Dodecahemidodecahedron",
        dual: "Small Dodecahemidodecacron",
        group: "Icosahedral (I[6c])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3|2 5/2",
        name: "Great Stellated Dodecahedron",
        dual: "Great Icosahedron",
        group: "Icosahedral (I[7])",
        class: "Kepler-Poinsot Solid",
        dual_class: "Kepler-Poinsot Solid",
    },
    Uniform {
        wythoff: "5/2|2 3",
        name: "Great Icosahedron",
        dual: "Great Stellated Dodecahedron",
        group: "Icosahedral (I[7])",
        class: "Kepler-Poinsot Solid",
        dual_class: "Kepler-Poinsot Solid",
    },
    Uniform {
        wythoff: "2|5/2 3",
        name: "Great Icosidodecahedron",
        dual: "Great Rhombic Triacontahedron",
        group: "Icosahedral (I[7])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "2 5/2|3",
        name: "Great Truncated Icosahedron",
        dual: "Great Stellapentakisdodecahedron",
        group: "Icosahedral (I[7])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "2 5/2 3|",
        name: "Rhombicosahedron",
        dual: "Rhombicosacron",
        group: "Icosahedral (I[7])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|2 5/2 3",
        name: "Great Snub Icosidodecahedron",
        dual: "Great Pentagonal Hexecontahedron",
        group: "Icosahedral (I[7])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "2 5|5/3",
        name: "Small Stellated Truncated Dodecahedron",
        dual: "Great Pentakisdodecahedron",
        group: "Icosahedral (I[9])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/3 2 5|",
        name: "Truncated Dodecadodecahedron",
        dual: "Medial Disdyakistriacontahedron",
        group: "Icosahedral (I[9])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|5/3 2 5",
        name: "Inverted Snub Dodecadodecahedron",
        dual: "Medial Inverted Pentagonal Hexecontahedron",
        group: "Icosahedral (I[9])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/2 3|5/3",
        name: "Great Dodecicosidodecahedron",
        dual: "Great Dodecacronic Hexecontahedron",
        group: "Icosahedral (I[10a])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/3 5/2|3",
        name: "Small Dodecahemicosahedron",
        dual: "Small Dodecahemicosacron",
        group: "Icosahedral (I[10a])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/3 5/2 3|",
        name: "Great Dodecicosahedron",
        dual: "Great Dodecicosacron",
        group: "Icosahedral (I[10a])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|5/3 5/2 3",
        name: "Great Snub Dodecicosidodecahedron",
        dual: "Great Hexagonal Hexecontahedron",
        group: "Icosahedral (I[10a])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/4 5|3",
        name: "Great Dodecahemicosahedron",
        dual: "Great Dodecahemicosacron",
        group: "Icosahedral (I[10b])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "2 3|5/3",
        name: "Great Stellated Truncated Dodecahedron",
        dual: "Great Triakisicosahedron",
        group: "Icosahedral (I[13])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/3 3|2",
        name: "Great Rhombicosidodecahedron",
        dual: "Great Deltoidal Hexecontahedron",
        group: "Icosahedral (I[13])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/3 2 3|",
        name: "Great Truncated Icosidodecahedron",
        dual: "Great Disdyakistriacontahedron",
        group: "Icosahedral (I[13])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|5/3 2 3",
        name: "Great Inverted Snub Icosidodecahedron",
        dual: "Great Inverted Pentagonal Hexecontahedron",
        group: "Icosahedral (I[13])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "5/3 5/2|5/3",
        name: "Great Dodecahemidodecahedron",
        dual: "Great Dodecahemidodecacron",
        group: "Icosahedral (I[18a])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 3|5/3",
        name: "Great Icosihemidodecahedron",
        dual: "Great Icosihemidodecacron",
        group: "Icosahedral (I[18b])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|3/2 3/2 5/2",
        name: "Small Retrosnub Icosicosidodecahedron",
        dual: "Small Hexagrammic Hexecontahedron",
        group: "Icosahedral (I[22])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 5/3 2|",
        name: "Great Rhombidodecahedron",
        dual: "Great Rhombidodecacron",
        group: "Icosahedral (I[23])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "|3/2 5/3 2",
        name: "Great Retrosnub Icosidodecahedron",
        dual: "Great Pentagrammic Hexecontahedron",
        group: "Icosahedral (I[23])",
        class: "",
        dual_class: "",
    },
    Uniform {
        wythoff: "3/2 5/3 3 5/2",
        name: "Great Dirhombicosidodecahedron",
        dual: "Great Dirhombicosidodecacron",
        group: "Non-Wythoffian",
        class: "",
        dual_class: "",
    },
];

/// One face of a drawable shape: which points it runs through, and which of
/// the polyhedron's face types it belongs to.
pub struct Face {
    pub color: usize,
    pub points: Vec<usize>,
}

/// A polyhedron ready to draw: points, and faces that index them.
pub struct Shape {
    /// One-based index into [`UNIFORM`], which is what upstream shows.
    pub number: usize,
    pub wythoff: String,
    pub name: String,
    pub dual: String,
    pub config: String,
    pub group: String,
    pub class: String,
    pub logical_faces: usize,
    pub logical_vertices: usize,
    pub nedges: usize,
    pub density: i32,
    pub chi: i32,
    pub points: Vec<Vector>,
    pub faces: Vec<Face>,
}

/// Turn a constructed polyhedron into something drawable, either the solid
/// itself or its dual.
///
/// A face of a uniform polyhedron can be a star, and a star is not a simple
/// polygon, so it cannot be filled by fanning it out from one of its own
/// corners. Each such face gets an *auxiliary* point instead and is
/// triangulated from there: the circumcentre for a regular star face, the
/// incentre for the pentagrams and hexagrams of the denser duals, and the
/// self-intersection of the crossed parallelogram for the duals whose Wythoff
/// symbol has an even denominator. The hemi-duals get six each, because their
/// faces run off to infinity and have to be cut short.
///
/// Upstream also turns the whole thing by a fixed azimuth and elevation
/// before handing it over. The angle it turns by is zero, so the rotation is
/// the identity and is not here.
fn construct(p: &Polyhedron, star: bool) -> Shape {
    let (v, f) = if star { (&p.f, &p.v) } else { (&p.v, &p.f) };
    let nv = v.len();
    let nf = f.len();

    let mut result = Shape {
        number: p.index + 1,
        wythoff: p.polyform.clone(),
        name: if star { p.dual_name() } else { p.name() }.to_string(),
        dual: if star { p.name() } else { p.dual_name() }.to_string(),
        config: p.config.clone(),
        group: p.group().to_string(),
        class: if star { p.dual_class() } else { p.class() }.to_string(),
        logical_faces: nf,
        logical_vertices: nv,
        nedges: p.ne,
        density: p.density,
        chi: p.chi,
        points: v.clone(),
        faces: Vec::new(),
    };

    let mut hit: Vec<bool> = Vec::new();
    let last = p.index == last_uniform() - 1;

    for i in 0..nf {
        // `ftype` is indexed by a face of the solid, so it may only be
        // consulted when this is the solid.
        let simple_star = !star && {
            let d = frac(p.n[p.ftype[i]]);
            d.d != 1 && d.d != d.n - 1
        };
        let dense_dual = star && p.k == 5 && (p.density > 30 || denominator(p.mm[0]) != 1);
        if simple_star || dense_dual {
            /* find the center of the face */
            let h = if !star && p.hemi && p.ftype[i] == 0 {
                0.0
            } else {
                p.minr / f[i].dot(f[i])
            };
            result.points.push(f[i].scale(h));
        } else if star && p.even != -1 {
            // The self-intersection of a crossed parallelogram. `hit` says
            // whether v0v1 crosses v2v3 rather than v0v3 crossing v1v2.
            let q = |j: usize| v[p.incid[j][i] as usize];
            let (v0, v1, v2, v3) = (q(0), q(1), q(2), q(3));
            let d0 = v0.diff(v2).dot(v0.diff(v2)).sqrt();
            let d1 = v1.diff(v3).dot(v1.diff(v3)).sqrt();
            let c0 = v0.sum(v2).scale(d1);
            let c1 = v1.sum(v3).scale(d0);
            let pt = c0.sum(c1).scale(0.5 / (d0 + d1));
            result.points.push(pt);
            let x = pt.diff(v2).cross(pt.diff(v3));
            hit.push(x.dot(x) < 1e-6);
        } else if star && p.hemi && !last {
            // The terminal points of the truncation and the
            // self-intersections:
            //
            //   v23       v0       v21
            //   |  \     /  \     /  |
            //   |   v0123    v0321   |
            //   |  /     \  /     \  |
            //   v01       v2       v03
            let j = usize::from(p.ftype[p.incid[0][i] as usize] == 0);
            let q = |k: usize| v[p.incid[k][i] as usize];
            let v0 = q(j); /* real vertex */
            let v1 = q(j + 1); /* ideal vertex (unit vector) */
            let v2 = q(j + 2); /* real */
            let v3 = q((j + 3) % 4); /* ideal */
            let (a, b) = crossings(v0, v1, v2, v3);
            for pt in truncations(v0, v2, a, b) {
                result.points.push(pt);
            }
        } else if star && last {
            // The last one has two crossed parallelograms in each face.
            let mut j = 0;
            while j < 8 && p.ftype[p.incid[j][i] as usize] != 3 {
                j += 1;
            }
            let q = |k: usize| v[p.incid[(j + k) % 8][i] as usize];
            let (v0, v1, v2, v3) = (q(0), q(1), q(2), q(3));
            let (v4, v5, v6, v7) = (q(4), q(5), q(6), q(7));
            let (a, b) = crossings(v0, v1, v2, v3);
            for pt in truncations(v0, v2, a, b) {
                result.points.push(pt);
            }
            let (a, b) = crossings(v4, v5, v6, v7);
            for pt in truncations(v4, v6, a, b) {
                result.points.push(pt);
            }
        }
    }

    /*
     * Face list. In the non-simple case the polygon is represented by its
     * triangulation, each triangle being two polyhedron vertices and one
     * auxiliary vertex.
     */
    let mut ii = nv;
    let mut facelets = 0;
    for i in 0..nf {
        if star {
            if p.k == 5 && (p.density > 30 || denominator(p.mm[0]) != 1) {
                for j in 0..p.m - 1 {
                    push3(&mut result, p.incid[j][i], p.incid[j + 1][i], ii as i32);
                    facelets += 1;
                }
                push3(&mut result, p.incid[p.m - 1][i], p.incid[0][i], ii as i32);
                ii += 1;
                facelets += 1;
            } else if p.even != -1 {
                if hit.get(i).copied().unwrap_or(false) {
                    push3(&mut result, p.incid[3][i], p.incid[0][i], ii as i32);
                    push3(&mut result, p.incid[1][i], p.incid[2][i], ii as i32);
                } else {
                    push3(&mut result, p.incid[0][i], p.incid[1][i], ii as i32);
                    push3(&mut result, p.incid[2][i], p.incid[3][i], ii as i32);
                }
                ii += 1;
                facelets += 2;
            } else if p.hemi && !last {
                let j = usize::from(p.ftype[p.incid[0][i] as usize] == 0);
                push3(&mut result, ii as i32, ii as i32 + 1, ii as i32 + 2);
                push4(
                    &mut result,
                    p.incid[j][i],
                    ii as i32 + 2,
                    p.incid[j + 2][i],
                    ii as i32 + 5,
                );
                push3(&mut result, ii as i32 + 3, ii as i32 + 4, ii as i32 + 5);
                ii += 6;
                facelets += 3;
            } else if last {
                let mut j = 0;
                while j < 8 && p.ftype[p.incid[j][i] as usize] != 3 {
                    j += 1;
                }
                // Two crossed parallelograms a face, each cut into the same
                // three pieces as a hemi-dual's.
                for (first, base) in [(0usize, 0i32), (4, 6)] {
                    let b = ii as i32 + base;
                    push3(&mut result, b, b + 1, b + 2);
                    push4(
                        &mut result,
                        p.incid[(j + first) % 8][i],
                        b + 2,
                        p.incid[(j + first + 2) % 8][i],
                        b + 5,
                    );
                    push3(&mut result, b + 3, b + 4, b + 5);
                }
                ii += 12;
                facelets += 6;
            } else {
                let points = (0..p.m).map(|j| p.incid[j][i] as usize).collect();
                result.faces.push(Face { color: 0, points });
                facelets += 1;
            }
        } else {
            let d = frac(p.n[p.ftype[i]]);
            let split = d.d != 1 && d.d != d.n - 1;
            /* find a vertex on this face, and which slot it is in */
            let (mut j, mut k) = (0usize, 0usize);
            'find: for jj in 0..nv {
                for kk in 0..p.m {
                    if p.incid[kk][jj] == i as i32 {
                        j = jj;
                        k = kk;
                        break 'find;
                    }
                }
            }
            /* walk the face's rim */
            let mut ring = vec![j];
            let mut ll = j;
            let mut l = p.adj[k][j] as usize;
            while l != j {
                k = 0;
                while k < p.m && p.incid[k][l] != i as i32 {
                    k += 1;
                }
                if p.adj[k][l] == ll as i32 {
                    k = imod(k as i64 + 1, p.m as i64) as usize;
                }
                ring.push(l);
                ll = l;
                l = p.adj[k][l] as usize;
            }
            if split {
                for w in 0..ring.len() {
                    push3(
                        &mut result,
                        ring[w] as i32,
                        ring[(w + 1) % ring.len()] as i32,
                        ii as i32,
                    );
                    facelets += 1;
                }
                ii += 1;
            } else {
                result.faces.push(Face {
                    color: 0,
                    points: ring,
                });
                facelets += 1;
            }
        }
    }

    /*
     * Face colour indices, for polyhedra with more than one face type. For a
     * non-simple face the index is repeated as many times as the
     * triangulation needed.
     */
    let mut ff = 0;
    if !star && p.n_types != 1 {
        for i in 0..nf {
            let d = frac(p.n[p.ftype[i]]);
            if d.d == 1 || d.d == d.n - 1 {
                result.faces[ff].color = p.ftype[i];
                ff += 1;
            } else {
                for _ in 0..d.n {
                    result.faces[ff].color = p.ftype[i];
                    ff += 1;
                }
            }
        }
    } else {
        for _ in 0..facelets {
            result.faces[ff].color = 0;
            ff += 1;
        }
    }

    result
}

/// Where the line `v0 + a v1` meets `v2 + b v3`, which is what an edge that
/// runs off to infinity crosses on its way.
fn crossing(v0: Vector, v1: Vector, v2: Vector, v3: Vector) -> Vector {
    let u = v1.cross(v3);
    v0.sum(v1.scale(v2.diff(v0).cross(v3).dot(u) / u.dot(u)))
}

fn crossings(v0: Vector, v1: Vector, v2: Vector, v3: Vector) -> (Vector, Vector) {
    (crossing(v0, v1, v2, v3), crossing(v0, v3, v2, v1))
}

/// The six points a hemi-dual's face is cut down to: the two crossings and
/// the four points its infinite edges are truncated at.
fn truncations(v0: Vector, v2: Vector, a: Vector, b: Vector) -> [Vector; 6] {
    /* truncation adjustment factor */
    let t = 1.5;
    [
        v0.sum(a.diff(v0).scale(t)),
        v2.sum(a.diff(v2).scale(t)),
        a,
        v0.sum(b.diff(v0).scale(t)),
        v2.sum(b.diff(v2).scale(t)),
        b,
    ]
}

fn push3(s: &mut Shape, x: i32, y: i32, z: i32) {
    s.faces.push(Face {
        color: 0,
        points: vec![x as usize, y as usize, z as usize],
    });
}

fn push4(s: &mut Shape, x: i32, y: i32, z: i32, w: i32) {
    s.faces.push(Face {
        color: 0,
        points: vec![x as usize, y as usize, z as usize, w as usize],
    });
}

/// How many drawable shapes there are: each of the eighty uniform polyhedra,
/// and each of their duals. Upstream's order, which is what its numbering and
/// its list of names both follow: a solid, then its own dual, then the next.
pub const SHAPES: usize = UNIFORM.len() * 2;

/// The name and the class of one shape, without building it. Naming one is
/// only a table lookup; building it is a Newton solve.
pub fn shape_names(n: usize) -> (&'static str, &'static str) {
    let u = &UNIFORM[n / 2];
    if n.is_multiple_of(2) {
        (u.name, u.class)
    } else {
        (u.dual, u.dual_class)
    }
}

/// Build one shape, ready to draw.
pub fn shape(n: usize) -> Option<Shape> {
    let p = kaleido(n / 2)?;
    Some(construct(&p, !n.is_multiple_of(2)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eighty_come_out() {
        assert_eq!(UNIFORM.len(), 80);
        for i in 0..UNIFORM.len() {
            let p = kaleido(i).unwrap_or_else(|| panic!("#{} {}", i + 1, UNIFORM[i].name));
            let what = format!("#{} {}", i + 1, p.name());

            // Euler's formula is the whole check: the construction counts the
            // faces and edges from the symmetry group, and then finds the
            // vertices and faces geometrically, and the two have to agree.
            assert_eq!(p.v.len(), p.nv, "{what}: vertices");
            assert_eq!(p.f.len(), p.nf, "{what}: faces");
            assert_eq!(
                p.nv as i32 - p.ne as i32 + p.nf as i32,
                p.chi,
                "{what}: V - E + F is not the Euler characteristic"
            );

            // Every vertex is on the unit sphere, which is what makes it
            // uniform, and no two are in the same place.
            for (j, v) in p.v.iter().enumerate() {
                let r = v.dot(*v).sqrt();
                assert!((r - 1.0).abs() < 1e-9, "{what}: vertex {j} at radius {r}");
            }

            // Every vertex has the same figure: M faces meet at each one, and
            // every slot of the incidence matrix names a face.
            for j in 0..p.m {
                for i2 in 0..p.nv {
                    let f = p.incid[j][i2];
                    assert!(
                        f >= 0 && (f as usize) < p.nf,
                        "{what}: vertex {i2} slot {j} is on face {f}"
                    );
                }
            }
            // And each face is named by as many vertex slots as it has sides.
            let mut hits = vec![0usize; p.nf];
            for row in &p.incid {
                for &f in row {
                    hits[f as usize] += 1;
                }
            }
            for (f, &n) in hits.iter().enumerate() {
                let sides = p.n[p.ftype[f]];
                assert_eq!(
                    n as i64,
                    numerator(sides),
                    "{what}: face {f} has {n} corners for a {sides}-gon"
                );
            }
        }
    }

    #[test]
    fn the_platonic_solids_are_what_they_should_be() {
        // The five, by name, with their vertex, edge and face counts and the
        // configuration the construction should write out.
        for (name, v, e, f, config) in [
            ("Tetrahedron", 4, 6, 4, "3, 3, 3"),
            ("Octahedron", 6, 12, 8, "3, 3, 3, 3"),
            ("Cube", 8, 12, 6, "4, 4, 4"),
            ("Icosahedron", 12, 30, 20, "3, 3, 3, 3, 3"),
            ("Dodecahedron", 20, 30, 12, "5, 5, 5"),
        ] {
            let i = UNIFORM
                .iter()
                .position(|u| u.name == name)
                .unwrap_or_else(|| panic!("no {name} in the table"));
            let p = kaleido(i).unwrap();
            assert_eq!((p.nv, p.ne, p.nf), (v, e, f), "{name}");
            assert_eq!(p.chi, 2, "{name} is not a sphere");
            assert_eq!(p.density, 1, "{name} wraps the centre more than once");
            assert_eq!(p.config, config, "{name}");
        }
    }

    #[test]
    fn the_kepler_poinsot_solids_wrap_the_centre() {
        // The four star polyhedra: the ones whose faces or vertex figures are
        // pentagrams, so they cover the centre more than once and are not
        // spheres.
        for (name, density, chi) in [
            ("Small Stellated Dodecahedron", 3, -6),
            ("Great Dodecahedron", 3, -6),
            ("Great Stellated Dodecahedron", 7, 2),
            ("Great Icosahedron", 7, 2),
        ] {
            let i = UNIFORM.iter().position(|u| u.name == name).unwrap();
            let p = kaleido(i).unwrap();
            assert_eq!(p.density, density, "{name}: density");
            assert_eq!(p.chi, chi, "{name}: characteristic");
        }
    }

    #[test]
    fn a_wythoff_symbol_survives_the_round_trip() {
        // What the construction writes back out is what the table said, bar
        // the spacing upstream normalises. The last one is not Wythoffian, so
        // the table holds no bar for it and one is put on the front.
        for (i, u) in UNIFORM.iter().enumerate() {
            let p = kaleido(i).unwrap();
            let mut want: String = u.wythoff.split_whitespace().collect::<Vec<_>>().join(" ");
            if i == UNIFORM.len() - 1 {
                want.insert(0, '|');
            }
            assert_eq!(p.polyform, want, "#{}", i + 1);
            assert!(
                p.polyform.matches('|').count() <= 1,
                "#{}: more than one bar",
                i + 1
            );
        }
    }
}

#[cfg(test)]
mod shapes {
    use super::*;

    #[test]
    fn every_solid_and_its_dual_come_out_drawable() {
        let all: Vec<Shape> = (0..SHAPES).filter_map(shape).collect();
        // Eighty solids and eighty duals.
        assert_eq!(all.len(), 160);

        for s in &all {
            let what = format!("#{} {}", s.number, s.name);
            assert!(!s.faces.is_empty(), "{what}: no faces");
            assert!(s.points.len() >= s.logical_vertices, "{what}: points");

            // Every face names points that exist and has at least three of
            // them, and no face names the same point twice.
            for (i, f) in s.faces.iter().enumerate() {
                assert!(f.points.len() >= 3, "{what}: face {i} is a line");
                for &pt in &f.points {
                    assert!(pt < s.points.len(), "{what}: face {i} point {pt}");
                }
                let mut seen = f.points.clone();
                seen.sort_unstable();
                seen.dedup();
                assert_eq!(seen.len(), f.points.len(), "{what}: face {i} repeats");
            }

            // Nothing runs off to infinity: the truncation of the hemi-duals
            // is what keeps their faces finite.
            for (i, p) in s.points.iter().enumerate() {
                assert!(
                    p.x.is_finite() && p.y.is_finite() && p.z.is_finite(),
                    "{what}: point {i} is {p:?}"
                );
                let r = p.dot(*p).sqrt();
                assert!(r < 60.0, "{what}: point {i} is {r} out");
            }
        }
    }

    #[test]
    fn a_cube_is_six_squares_and_its_dual_is_eight_triangles() {
        let all: Vec<Shape> = (0..SHAPES).filter_map(shape).collect();
        let cube = all.iter().find(|s| s.name == "Cube").unwrap();
        assert_eq!(cube.faces.len(), 6);
        for f in &cube.faces {
            assert_eq!(f.points.len(), 4);
        }
        // The eight corners are at the same distance and the six faces each
        // have four of them.
        assert_eq!(cube.logical_vertices, 8);
        let mut used = vec![0; cube.points.len()];
        for f in &cube.faces {
            for &p in &f.points {
                used[p] += 1;
            }
        }
        assert!(used.iter().take(8).all(|&n| n == 3), "{used:?}");

        let oct = all.iter().find(|s| s.name == "Octahedron").unwrap();
        assert_eq!(oct.dual, "Cube");
        assert_eq!(oct.faces.len(), 8);
        for f in &oct.faces {
            assert_eq!(f.points.len(), 3);
        }
    }

    #[test]
    fn a_face_with_more_than_one_kind_is_coloured_by_kind() {
        let all: Vec<Shape> = (0..SHAPES).filter_map(shape).collect();
        // The cuboctahedron is squares and triangles, so its faces carry two
        // different colour indices.
        let c = all.iter().find(|s| s.name == "Cuboctahedron").unwrap();
        let mut colors: Vec<usize> = c.faces.iter().map(|f| f.color).collect();
        colors.sort_unstable();
        colors.dedup();
        assert_eq!(colors.len(), 2, "{colors:?}");
        // Eight triangles and six squares.
        let tris = c.faces.iter().filter(|f| f.points.len() == 3).count();
        let quads = c.faces.iter().filter(|f| f.points.len() == 4).count();
        assert_eq!((tris, quads), (8, 6));
    }
}
