//! Port of `hacks/glx/involute.c`.
//!
//! ```text
//! involute, Copyright (c) 2004-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Utilities for rendering OpenGL gears with involute teeth.
//! ```
//!
//! Three savers draw gears, and this is the gear. A [`Gear`] is a couple of
//! dozen numbers: how big, how many teeth, how thick, which of up to three
//! concentric rings it has inside it and how deep each one is. Everything else
//! is turned rings and discs.
//!
//! The teeth are the work. A tooth is a fixed profile of radii and angles,
//! sampled at two, four, nine or eighteen points depending on how much of the
//! screen the gear is going to take up, and the profile is walked once per
//! tooth to make one closed ring of points. The outside of the gear is that
//! ring extruded; the inside is a plain circle extruded; the flat faces join
//! the two, point for point, which is why both rings must have the same count.
//!
//! Normals are the other half of the work, and are why the ring is built as a
//! whole rather than a tooth at a time: each point's normal is the average of
//! the two faces meeting there, so a tooth's flanks shade smoothly into its tip
//! while the gear still reads as faceted.
//!
//! Upstream carries two debugging switches, one to draw normals as lines and
//! one to stop wireframe mode abbreviating. Both are off in every build, so the
//! branches they guard are not here.

use super::gl::{Glx, Shape};
use super::shapes::calc_normal;

/// How finely a gear's teeth are sampled, which upstream picks from how big the
/// gear will be on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Size {
    #[default]
    Small,
    Medium,
    Large,
    Huge,
}

/// One gear. The fields upstream's `gear` carries for its callers to track
/// position and gearing live in the savers; these are the ones the drawing
/// needs.
#[derive(Clone, Debug)]
pub struct Gear {
    /// Radius at the middle of the teeth.
    pub r: f64,
    /// Rotation, in degrees.
    pub th: f64,
    pub nteeth: i32,
    pub tooth_h: f64,
    /// 0 for a normal right-angled gear, 1 for forty-five degrees.
    pub tooth_slope: f64,

    /// The larger inside hole, and up to two smaller ones inside that.
    pub inner_r: f64,
    pub inner_r2: f64,
    pub inner_r3: f64,

    /// Height of the edge, and of the two inner discs if any.
    pub thickness: f64,
    pub thickness2: f64,
    pub thickness3: f64,

    pub spokes: i32,
    pub nubs: i32,
    /// Spoke against hole: how much of the ring the spokes take up.
    pub spoke_thickness: f64,
    /// Factory defect.
    pub wobble: f64,

    /// Teeth on the inside rather than the outside.
    pub inverted_p: bool,
    /// One of a bound pair: 1 for the first, 2 for the second, 0 for neither.
    pub coax_p: i32,
    pub coax_displacement: f64,
    pub coax_thickness: f64,

    pub size: Size,
    pub color: [f32; 4],
    pub color2: [f32; 4],
}

impl Default for Gear {
    fn default() -> Self {
        Gear {
            r: 1.0,
            th: 0.0,
            nteeth: 10,
            tooth_h: 0.1,
            tooth_slope: 0.0,
            inner_r: 0.0,
            inner_r2: 0.0,
            inner_r3: 0.0,
            thickness: 0.1,
            thickness2: 0.0,
            thickness3: 0.0,
            spokes: 0,
            nubs: 0,
            spoke_thickness: 1.0,
            wobble: 0.0,
            inverted_p: false,
            coax_p: 0,
            coax_displacement: 0.0,
            coax_thickness: 0.0,
            size: Size::Large,
            color: [1.0; 4],
            color2: [1.0; 4],
        }
    }
}

const TAU: f32 = std::f32::consts::PI * 2.0;

/// An uncapped tube of this radius from `top` to `bottom`, facing either in or
/// out.
fn draw_ring(
    g: &mut Glx,
    segments: i32,
    r: f32,
    top: f32,
    bottom: f32,
    slope: f32,
    in_p: bool,
    wire_p: bool,
) {
    let width = TAU / segments as f32;
    let s1 = 1.0 + ((bottom - top) * slope / 2.0);
    let s2 = 1.0 - ((bottom - top) * slope / 2.0);

    if top != bottom {
        g.front_face_cw(!in_p);
        g.begin(if wire_p {
            Shape::Lines
        } else {
            Shape::QuadStrip
        });
        for i in 0..segments + i32::from(!wire_p) {
            let th = i as f32 * width;
            let (cth, sth) = (th.cos(), th.sin());
            if in_p {
                g.normal3f(-cth, -sth, 0.0);
            } else {
                g.normal3f(cth, sth, 0.0);
            }
            g.vertex3f(s1 * cth * r, s1 * sth * r, top);
            g.vertex3f(s2 * cth * r, s2 * sth * r, bottom);
        }
        g.end();
    }

    if wire_p {
        for z in [top, bottom] {
            g.begin(Shape::LineLoop);
            for i in 0..segments {
                let th = i as f32 * width;
                g.vertex3f(th.cos() * r, th.sin() * r, z);
            }
            g.end();
        }
    }
}

/// A donut between two radii, facing either up or down. The first radius may
/// be zero, for a filled disc.
fn draw_disc(g: &mut Glx, segments: i32, ra: f32, rb: f32, z: f32, up_p: bool, wire_p: bool) {
    debug_assert!(ra >= 0.0 && rb > 0.0);
    if ra < 0.0 || rb <= 0.0 {
        return;
    }
    let width = TAU / segments as f32;

    if ra == 0.0 {
        g.front_face_cw(up_p);
    } else {
        g.front_face_cw(!up_p);
    }

    let shape = if wire_p {
        Shape::Lines
    } else if ra == 0.0 {
        Shape::TriangleFan
    } else {
        Shape::QuadStrip
    };
    g.begin(shape);
    g.normal3f(0.0, 0.0, if up_p { -1.0 } else { 1.0 });

    if ra == 0.0 && !wire_p {
        g.vertex3f(0.0, 0.0, z);
    }

    for i in 0..segments + i32::from(!wire_p) {
        let th = i as f32 * width;
        let (cth, sth) = (th.cos(), th.sin());
        if wire_p || ra != 0.0 {
            g.vertex3f(cth * ra, sth * ra, z);
        }
        g.vertex3f(cth * rb, sth * rb, z);
    }
    g.end();
}

/// N thick radial bars between two radii, as a solid slab each.
#[allow(clippy::too_many_arguments)]
fn draw_spokes(
    g: &mut Glx,
    n: i32,
    thickness: f32,
    segments: i32,
    ra: f32,
    rb: f32,
    z1: f32,
    z2: f32,
    slope: f32,
    wire_p: bool,
) {
    debug_assert!(ra > 0.0 && rb > 0.0);
    if ra <= 0.0 || rb <= 0.0 {
        return;
    }
    // Upstream divides by this without looking. A gear with no spoke thickness
    // has no spokes, which is a better answer than an unbounded loop.
    if thickness <= 0.0 {
        return;
    }

    let s1 = 1.0 + ((z2 - z1) * slope / 2.0);
    let s2 = 1.0 - ((z2 - z1) * slope / 2.0);

    let segments = segments * 3;
    // Round up to a multiple of n, upstream's way and no faster than it needs
    // to be.
    let mut segments2 = 0;
    while segments2 < segments {
        segments2 += n;
    }

    let mut insegs = (((segments2 / n) as f32 + 0.5) / thickness) as i32;
    let mut outsegs = (segments2 / n) - insegs;
    if insegs <= 0 {
        insegs = 1;
    }
    if outsegs <= 0 {
        outsegs = 1;
    }

    let segments2 = (insegs + outsegs) * n;
    let width = TAU / segments2 as f32;

    let mut tick = 0;
    let mut state = 0;
    for i in 0..segments2 {
        let th1 = i as f32 * width;
        let th2 = th1 + width;
        let (cth1, sth1) = (th1.cos(), th1.sin());
        let (cth2, sth2) = (th2.cos(), th2.sin());

        let mut changed = i == 0;
        if state == 0 && tick == insegs {
            tick = 0;
            state = 1;
            changed = true;
        } else if state == 1 && tick == outsegs {
            tick = 0;
            state = 0;
            changed = true;
        }

        let shape = if wire_p { Shape::Lines } else { Shape::Quads };

        if (state == 1 || (state == 0 && changed)) && !wire_p {
            // top
            g.front_face_cw(false);
            g.begin(shape);
            g.normal3f(0.0, 0.0, -1.0);
            g.vertex3f(s1 * cth1 * ra, s1 * sth1 * ra, z1);
            g.vertex3f(s1 * cth1 * rb, s1 * sth1 * rb, z1);
            g.vertex3f(s1 * cth2 * rb, s1 * sth2 * rb, z1);
            g.vertex3f(s1 * cth2 * ra, s1 * sth2 * ra, z1);
            g.end();

            // bottom
            g.front_face_cw(true);
            g.begin(shape);
            g.normal3f(0.0, 0.0, 1.0);
            g.vertex3f(s2 * cth1 * ra, s2 * sth1 * ra, z2);
            g.vertex3f(s2 * cth1 * rb, s2 * sth1 * rb, z2);
            g.vertex3f(s2 * cth2 * rb, s2 * sth2 * rb, z2);
            g.vertex3f(s2 * cth2 * ra, s2 * sth2 * ra, z2);
            g.end();
        }

        if state == 1 && changed {
            // left: the leading edge of a spoke
            g.front_face_cw(true);
            g.begin(shape);
            let n = calc_normal(
                [s1 * cth1 * rb, s1 * sth1 * rb, z1],
                [s1 * cth1 * ra, s1 * sth1 * ra, z1],
                [s2 * cth1 * rb, s2 * sth1 * rb, z2],
            );
            g.normal3f(n[0], n[1], n[2]);
            g.vertex3f(s1 * cth1 * ra, s1 * sth1 * ra, z1);
            g.vertex3f(s1 * cth1 * rb, s1 * sth1 * rb, z1);
            g.vertex3f(s2 * cth1 * rb, s2 * sth1 * rb, z2);
            g.vertex3f(s2 * cth1 * ra, s2 * sth1 * ra, z2);
            g.end();
        }

        if state == 0 && changed {
            // right: the trailing edge
            g.front_face_cw(false);
            g.begin(shape);
            let n = calc_normal(
                [s1 * cth2 * ra, s1 * sth2 * ra, z1],
                [s1 * cth2 * rb, s1 * sth2 * rb, z1],
                [s2 * cth2 * rb, s2 * sth2 * rb, z2],
            );
            g.normal3f(n[0], n[1], n[2]);
            g.vertex3f(s1 * cth2 * ra, s1 * sth2 * ra, z1);
            g.vertex3f(s1 * cth2 * rb, s1 * sth2 * rb, z1);
            g.vertex3f(s2 * cth2 * rb, s2 * sth2 * rb, z2);
            g.vertex3f(s2 * cth2 * ra, s2 * sth2 * ra, z2);
            g.end();
        }

        tick += 1;
    }
}

/// Which of the gear's inside rings is the widest, and where and how deep it
/// is. Returns 0 for the outermost, 1 for either of the others, which is which
/// of the two colours the nubs take.
pub fn biggest_ring(gear: &Gear) -> (usize, f64, f64, f64) {
    let r0 = gear.r - gear.tooth_h / 2.0;
    let (r1, r2, r3) = (gear.inner_r, gear.inner_r2, gear.inner_r3);
    let w1 = if r1 != 0.0 { r0 - r1 } else { r0 };
    let mut w2 = if r2 != 0.0 { r1 - r2 } else { 0.0 };
    let w3 = if r3 != 0.0 { r2 - r3 } else { 0.0 };

    if gear.spokes != 0 {
        w2 = 0.0;
    }

    if w1 > w2 && w1 > w3 {
        (0, (r0 + r1) / 2.0, w1, gear.thickness)
    } else if w2 > w1 && w2 > w3 {
        (1, (r1 + r2) / 2.0, w2, gear.thickness2)
    } else {
        (1, (r2 + r3) / 2.0, w3, gear.thickness3)
    }
}

/// Little cylinders embedded in the widest ring, aligned with the teeth.
fn draw_gear_nubs(g: &mut Glx, gear: &Gear, wire_p: bool) {
    if gear.nubs == 0 {
        return;
    }
    let steps = if gear.size < Size::Large { 5 } else { 20 };
    let (which, mut r, size, height) = biggest_ring(gear);
    let mut size = size / 5.0;
    let mut height = height * 0.7;

    let cc = if which == 1 { gear.color2 } else { gear.color };
    g.material_ambient_diffuse(cc);

    if gear.inverted_p {
        r = gear.r + size + gear.tooth_h;
    }

    let width = TAU as f64 / f64::from(gear.nubs);
    // Line the first nub up with a tooth.
    let off = std::f64::consts::PI / f64::from(gear.nteeth * 2);

    for i in 0..gear.nubs {
        let th = (f64::from(i) * width) + off;
        g.push_matrix();
        g.rotate((th * 180.0 / std::f64::consts::PI) as f32, 0.0, 0.0, 1.0);
        g.translate(r as f32, 0.0, 0.0);

        if gear.inverted_p {
            // Nubs go on the outside rim.
            size = gear.thickness / 3.0;
            height = (gear.r - gear.inner_r) / 2.0;
            g.translate(height as f32, 0.0, 0.0);
            g.rotate(90.0, 0.0, 1.0, 0.0);
        }

        let (size, height) = (size as f32, height as f32);
        if wire_p {
            draw_ring(
                g,
                if gear.size >= Size::Large {
                    steps / 2
                } else {
                    steps
                },
                size,
                0.0,
                0.0,
                0.0,
                false,
                wire_p,
            );
        } else {
            draw_disc(g, steps, 0.0, size, -height, true, wire_p);
            draw_disc(g, steps, 0.0, size, height, false, wire_p);
            draw_ring(g, steps, size, -height, height, 0.0, false, wire_p);
        }
        g.pop_matrix();
    }
}

/// A much simpler representation of a gear: a spoked wheel of lines, for when
/// it is spinning too fast to be worth drawing properly.
pub fn draw_schematic(g: &mut Glx, gear: &Gear, wire_p: bool) {
    let width = TAU / gear.nteeth as f32;
    if !wire_p {
        g.lighting(false);
    }
    g.color4f(
        gear.color[0] * 0.6,
        gear.color[1] * 0.6,
        gear.color[2] * 0.6,
        1.0,
    );

    let z = -gear.thickness as f32 / 2.0;
    g.begin(Shape::Lines);
    for i in 0..gear.nteeth {
        let th = (i as f32 * width) + (width / 4.0);
        g.vertex3f(0.0, 0.0, z);
        g.vertex3f(th.cos() * gear.r as f32, th.sin() * gear.r as f32, z);
    }
    g.end();

    g.begin(Shape::LineLoop);
    for i in 0..gear.nteeth {
        let th = (i as f32 * width) + (width / 4.0);
        g.vertex3f(th.cos() * gear.r as f32, th.sin() * gear.r as f32, z);
    }
    g.end();

    if !wire_p {
        g.lighting(true);
    }
}

/// The discs and axles inside the teeth.
fn draw_gear_interior(g: &mut Glx, gear: &Gear, wire_p: bool) {
    let mut steps = gear.nteeth * 2;
    if steps < 10 {
        steps = 10;
    }
    if wire_p || gear.size < Size::Large {
        steps /= 2;
    }
    if gear.size < Size::Large && steps > 16 {
        steps = 16;
    }

    // Ring 1 facing in is done with the teeth.

    if gear.inner_r2 != 0.0 {
        // Slightly larger than inner_r, since the points do not line up.
        let ra = (gear.inner_r * 1.04) as f32;
        let rb = gear.inner_r2 as f32;
        let za = -gear.thickness2 as f32 / 2.0;
        let zb = gear.thickness2 as f32 / 2.0;
        let slope = gear.tooth_slope as f32;
        let s1 = 1.0 + (gear.thickness2 * gear.tooth_slope / 2.0) as f32;
        let s2 = 1.0 - (gear.thickness2 * gear.tooth_slope / 2.0) as f32;

        g.material_ambient_diffuse(gear.color2);

        if gear.coax_p != 1 && gear.inner_r3 == 0.0 {
            draw_ring(g, steps, rb, za, zb, slope, true, wire_p);
        }

        if gear.spokes != 0 {
            draw_spokes(
                g,
                gear.spokes,
                gear.spoke_thickness as f32,
                steps,
                ra,
                rb,
                za,
                zb,
                slope,
                wire_p,
            );
        } else if !wire_p {
            draw_disc(g, steps, s1 * ra, s1 * rb, za, true, wire_p);
            draw_disc(g, steps, s2 * ra, s2 * rb, zb, false, wire_p);
        }
    }

    if gear.inner_r3 != 0.0 {
        let ra = gear.inner_r2 as f32;
        let rb = gear.inner_r3 as f32;
        let za = -gear.thickness3 as f32 / 2.0;
        let zb = gear.thickness3 as f32 / 2.0;
        let slope = gear.tooth_slope as f32;
        let s1 = 1.0 + (gear.thickness3 * gear.tooth_slope / 2.0) as f32;
        let s2 = 1.0 - (gear.thickness3 * gear.tooth_slope / 2.0) as f32;

        g.material_ambient_diffuse(gear.color);
        draw_ring(g, steps, ra, za, zb, slope, false, wire_p);

        if gear.coax_p != 1 {
            draw_ring(g, steps, rb, za, zb, slope, true, wire_p);
        }
        if !wire_p {
            draw_disc(g, steps, s1 * ra, s1 * rb, za, true, wire_p);
            draw_disc(g, steps, s2 * ra, s2 * rb, zb, false, wire_p);
        }
    }

    // The axle tube of a bound pair, which reaches up to its partner.
    if gear.coax_p == 1 {
        let cap_height = gear.coax_thickness / 3.0;
        let ra = if gear.inner_r3 != 0.0 {
            gear.inner_r3
        } else if gear.inner_r2 != 0.0 {
            gear.inner_r2
        } else {
            gear.inner_r
        } as f32;
        let za = -(gear.thickness / 2.0 + cap_height) as f32;
        let zb = (gear.coax_thickness / 2.0 + gear.coax_displacement + cap_height) as f32;

        g.material_ambient_diffuse(gear.color);
        if wire_p {
            steps /= 2;
        }
        draw_ring(g, steps, ra, za, zb, gear.tooth_slope as f32, false, wire_p);
        if !wire_p {
            draw_disc(g, steps, 0.0, ra, za, true, wire_p);
            draw_disc(g, steps, 0.0, ra, zb, false, wire_p);
        }
    }
}

/// One ring of the gear's outline, with a normal at every point.
struct ToothFace {
    points: Vec<[f32; 3]>,
    pnormals: Vec<[f32; 3]>,
}

impl ToothFace {
    /// Each point's normal is the average of the two faces that meet there, so
    /// a tooth's flanks shade into its tip.
    fn compute_normals(&mut self, tooth_slope: f32) {
        let n = self.points.len();
        let mut fnormals = vec![[0.0f32; 3]; n];
        for (i, fnormal) in fnormals.iter_mut().enumerate() {
            let p1 = self.points[i];
            let p2 = self.points[if i == n - 1 { 0 } else { i + 1 }];
            let p3 = [
                p1[0] - p1[0] * tooth_slope,
                p1[1] - p1[1] * tooth_slope,
                p1[2] + 1.0,
            ];
            *fnormal = calc_normal(p1, p2, p3);
        }

        self.pnormals = (0..n)
            .map(|i| {
                let a = if i == 0 { n - 1 } else { i - 1 };
                let (n1, n2) = (fnormals[a], fnormals[i]);
                [
                    (n1[0] + n2[0]) / 2.0,
                    (n1[1] + n2[1]) / 2.0,
                    (n1[2] + n2[2]) / 2.0,
                ]
            })
            .collect();
    }

    fn flip(&mut self) {
        for v in &mut self.pnormals {
            *v = [-v[0], -v[1], -v[2]];
        }
    }
}

/// The vertices and normals of every tooth: the heavy lifting.
///
/// Upstream caches this in a display list, since the numbers are different for
/// essentially every gear and so cannot be shared. Here it is recomputed each
/// frame, because a display list in this runtime replays commands rather than
/// results and would cost the same.
fn gear_teeth_geometry(gear: &Gear) -> (ToothFace, ToothFace) {
    let width = TAU / gear.nteeth as f32;
    let rh = gear.tooth_h as f32;
    let tw = width;
    let big_r = gear.r as f32;

    // The profile of one tooth: radii out from the middle, and the angles they
    // are reached at. See the diagram in upstream's source.
    let mut r = [0.0f32; 9];
    r[0] = big_r + (rh * 0.50);
    r[1] = big_r + (rh * 0.40);
    r[2] = big_r + (rh * 0.25);
    r[3] = big_r + (rh * 0.05);
    r[4] = big_r - (r[2] - big_r);
    r[5] = big_r - (r[1] - big_r);
    r[6] = big_r - (r[0] - big_r);
    r[7] = r[6]; /* unused */
    r[8] = gear.inner_r as f32;

    let mut th = [0.0f32; 20];
    th[0] = -tw
        * match gear.size {
            Size::Small => 0.5,
            Size::Medium => 0.41,
            _ => 0.45,
        };
    th[1] = -tw * 0.375;
    th[2] = -tw * 0.300;
    th[3] = -tw * 0.230;
    th[4] = -tw * if gear.nteeth >= 5 { 0.16 } else { 0.12 };
    th[5] = -tw * 0.100;
    th[6] = -tw * if gear.size == Size::Medium { 0.1 } else { 0.04 };
    th[7] = -tw * 0.020;
    th[8] = 0.0;
    th[9] = -th[7];
    th[10] = -th[6];
    th[11] = -th[5];
    th[12] = -th[4];
    th[13] = -th[3];
    th[14] = -th[2];
    th[15] = -th[1];
    th[16] = -th[0];
    th[17] = width * 0.47;
    th[18] = width * 0.50;
    th[19] = width * 0.53;

    if gear.inverted_p {
        // Put the teeth on the inside.
        for t in &mut th {
            *t = -*t;
        }
        for x in &mut r {
            *x = big_r - (*x - big_r);
        }
    }

    // (outer radius, inner radius, angle) for each point of the profile, at the
    // chosen level of detail.
    let profile: &[(usize, usize, usize)] = match gear.size {
        Size::Small => &[(6, 8, 0), (0, 8, 8)],
        Size::Medium => &[(6, 8, 0), (0, 8, 6), (0, 8, 10), (6, 8, 16)],
        Size::Large => &[
            (6, 8, 0),
            (4, 8, 2),
            (2, 8, 4),
            (0, 8, 6),
            (0, 8, 10),
            (2, 8, 12),
            (4, 8, 14),
            (6, 8, 16),
            (6, 8, 18),
        ],
        Size::Huge => &[
            (6, 8, 0),
            (5, 8, 1),
            (4, 8, 2),
            (3, 8, 3),
            (2, 8, 4),
            (1, 8, 5),
            (0, 8, 6),
            (0, 8, 8),
            (0, 8, 10),
            (1, 8, 11),
            (2, 8, 12),
            (3, 8, 13),
            (4, 8, 14),
            (5, 8, 15),
            (6, 8, 16),
            (6, 8, 17),
            (6, 8, 18),
            (6, 8, 19),
        ],
    };

    let mut orim = ToothFace {
        points: Vec::with_capacity(profile.len() * gear.nteeth as usize),
        pnormals: Vec::new(),
    };
    let mut irim = ToothFace {
        points: Vec::with_capacity(profile.len() * gear.nteeth as usize),
        pnormals: Vec::new(),
    };

    for i in 0..gear.nteeth {
        let big_th = (i as f32 * width) + (width / 4.0);
        let (oon, oin) = (orim.points.len(), irim.points.len());

        for &(opr, ipr, pth) in profile {
            let a = big_th + th[pth];
            orim.points.push([a.cos() * r[opr], a.sin() * r[opr], 0.0]);
            irim.points.push([a.cos() * r[ipr], a.sin() * r[ipr], 0.0]);
        }

        if gear.inverted_p {
            // Inside out means the points of a tooth come round the other way.
            orim.points[oon..].reverse();
            irim.points[oin..].reverse();
        }
    }

    orim.compute_normals(gear.tooth_slope as f32);
    irim.compute_normals(0.0);

    if gear.inverted_p {
        orim.flip();
        irim.flip();
    }
    (orim, irim)
}

/// The teeth: the outer rim, the inner hole, and the two flat faces joining
/// them point for point.
fn draw_gear_teeth(g: &mut Glx, gear: &Gear, wire_p: bool) {
    let z1 = -gear.thickness as f32 / 2.0;
    let z2 = gear.thickness as f32 / 2.0;
    let s1 = 1.0 + (gear.thickness * gear.tooth_slope / 2.0) as f32;
    let s2 = 1.0 - (gear.thickness * gear.tooth_slope / 2.0) as f32;

    let (orim, irim) = gear_teeth_geometry(gear);
    g.material_ambient_diffuse(gear.color);

    // The outer rim, the teeth themselves.
    g.front_face_cw(!gear.inverted_p);
    g.begin(if wire_p {
        Shape::Lines
    } else {
        Shape::QuadStrip
    });
    for (p, n) in orim.points.iter().zip(&orim.pnormals) {
        g.normal3f(n[0], n[1], n[2]);
        g.vertex3f(s1 * p[0], s1 * p[1], z1);
        g.vertex3f(s2 * p[0], s2 * p[1], z2);
    }
    if !wire_p {
        // Close the loop.
        let (p, n) = (orim.points[0], orim.pnormals[0]);
        g.normal3f(n[0], n[1], n[2]);
        g.vertex3f(s1 * p[0], s1 * p[1], z1);
        g.vertex3f(s2 * p[0], s2 * p[1], z2);
    }
    g.end();

    if wire_p {
        for (z, s) in [(z1, s1), (z2, s2)] {
            g.begin(Shape::LineLoop);
            for p in &orim.points {
                g.vertex3f(s * p[0], s * p[1], z);
            }
            g.end();
        }
    }

    // The inner rim, the hole.
    g.front_face_cw(gear.inverted_p);
    g.begin(if wire_p {
        Shape::Lines
    } else {
        Shape::QuadStrip
    });
    for (p, n) in irim.points.iter().zip(&irim.pnormals) {
        g.normal3f(-n[0], -n[1], -n[2]);
        g.vertex3f(s1 * p[0], s1 * p[1], z1);
        g.vertex3f(s2 * p[0], s2 * p[1], z2);
    }
    if !wire_p {
        let (p, n) = (irim.points[0], irim.pnormals[0]);
        g.normal3f(-n[0], -n[1], -n[2]);
        g.vertex3f(s1 * p[0], s1 * p[1], z1);
        g.vertex3f(s2 * p[0], s2 * p[1], z2);
    }
    g.end();

    if wire_p {
        for z in [z1, z2] {
            g.begin(Shape::LineLoop);
            for p in &irim.points {
                g.vertex3f(p[0], p[1], z);
            }
            g.end();
        }
    }

    // The two flat faces, which is why both rims have to have the same number
    // of points.
    if !wire_p {
        debug_assert_eq!(orim.points.len(), irim.points.len());
        for (z, s, first) in [(z1, s1, true), (z2, s2, false)] {
            g.front_face_cw(!(first ^ gear.inverted_p));
            g.normal3f(0.0, 0.0, z);
            g.begin(Shape::QuadStrip);
            for (o, i) in orim.points.iter().zip(&irim.points) {
                g.vertex3f(s * o[0], s * o[1], z);
                g.vertex3f(s * i[0], s * i[1], z);
            }
            let (o, i) = (orim.points[0], irim.points[0]);
            g.vertex3f(s * o[0], s * o[1], z);
            g.vertex3f(s * i[0], s * i[1], z);
            g.end();
        }
    }
}

/// One gear, unrotated at the origin.
pub fn draw_gear(g: &mut Glx, gear: &Gear, wire_p: bool) {
    g.material_specular([1.0, 1.0, 1.0, 1.0]);
    g.material_shininess(128.0);
    g.material_ambient_diffuse(gear.color);
    g.color4f(gear.color[0], gear.color[1], gear.color[2], 1.0);

    g.push_matrix();
    g.rotate(gear.wobble as f32, 1.0, 0.0, 0.0);
    draw_gear_teeth(g, gear, wire_p);
    draw_gear_interior(g, gear, wire_p);
    draw_gear_nubs(g, gear, wire_p);
    g.pop_matrix();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_gear() -> Gear {
        Gear {
            r: 1.0,
            nteeth: 15,
            tooth_h: 0.17,
            inner_r: 0.8,
            inner_r2: 0.6,
            inner_r3: 0.55,
            thickness: 0.2,
            thickness2: 0.02,
            thickness3: 0.2,
            size: Size::Huge,
            color: [0.8, 0.8, 0.9, 1.0],
            color2: [0.68, 0.68, 0.77, 1.0],
            ..Gear::default()
        }
    }

    /// The two rims must have the same number of points, since the flat faces
    /// join them one to one. Upstream aborts if they ever do not.
    #[test]
    fn the_two_rims_match_point_for_point() {
        for size in [Size::Small, Size::Medium, Size::Large, Size::Huge] {
            for nteeth in [7, 15, 32] {
                let gear = Gear {
                    nteeth,
                    size,
                    ..a_gear()
                };
                let (orim, irim) = gear_teeth_geometry(&gear);
                assert_eq!(orim.points.len(), irim.points.len());
                assert_eq!(orim.points.len() % nteeth as usize, 0);
                assert_eq!(orim.pnormals.len(), orim.points.len());
            }
        }
    }

    /// The detail levels are what they say: more points on a bigger gear.
    #[test]
    fn the_mesh_gets_denser_with_size() {
        let mut counts = Vec::new();
        for size in [Size::Small, Size::Medium, Size::Large, Size::Huge] {
            let gear = Gear { size, ..a_gear() };
            counts.push(gear_teeth_geometry(&gear).0.points.len());
        }
        assert_eq!(counts, vec![15 * 2, 15 * 4, 15 * 9, 15 * 18]);
    }

    /// A tooth reaches out past the gear's radius and its gap falls short of
    /// it, which is what makes it a tooth.
    #[test]
    fn the_teeth_stick_out_and_the_gaps_go_in() {
        let gear = a_gear();
        let (orim, _) = gear_teeth_geometry(&gear);
        let radius = |p: &[f32; 3]| (p[0] * p[0] + p[1] * p[1]).sqrt();
        let hi = orim.points.iter().map(radius).fold(0.0f32, f32::max);
        let lo = orim.points.iter().map(radius).fold(f32::MAX, f32::min);

        assert!(
            (hi - (gear.r + gear.tooth_h / 2.0) as f32).abs() < 1e-5,
            "the tip is at {hi}"
        );
        assert!(
            (lo - (gear.r - gear.tooth_h / 2.0) as f32).abs() < 1e-5,
            "the root is at {lo}"
        );
    }

    /// The normals point outwards, away from the axis, which is what makes a
    /// gear look lit rather than inside out.
    #[test]
    fn the_normals_face_out() {
        let (orim, _) = gear_teeth_geometry(&a_gear());
        for (p, n) in orim.points.iter().zip(&orim.pnormals) {
            let dot = p[0] * n[0] + p[1] * n[1];
            assert!(dot > 0.0, "a normal at {p:?} points inwards: {n:?}");
        }
    }

    /// Inverting a gear turns it inside out: the teeth point in, and so do the
    /// normals.
    #[test]
    fn an_inverted_gear_has_its_teeth_inside() {
        let gear = Gear {
            inverted_p: true,
            ..a_gear()
        };
        let (orim, _) = gear_teeth_geometry(&gear);
        let radius = |p: &[f32; 3]| (p[0] * p[0] + p[1] * p[1]).sqrt();
        let lo = orim.points.iter().map(radius).fold(f32::MAX, f32::min);
        assert!(
            (lo - (gear.r - gear.tooth_h / 2.0) as f32).abs() < 1e-5,
            "the tip should reach in to {}, not {lo}",
            gear.r - gear.tooth_h / 2.0
        );
        for (p, n) in orim.points.iter().zip(&orim.pnormals) {
            assert!(p[0] * n[0] + p[1] * n[1] < 0.0, "a normal still faces out");
        }
    }

    /// The widest ring is the one the nubs go in.
    #[test]
    fn the_biggest_ring_is_found() {
        // Outermost is widest: 1.0 - 0.17/2 - 0.8 = 0.115 against 0.2 and 0.05.
        let gear = Gear {
            inner_r: 0.8,
            inner_r2: 0.6,
            inner_r3: 0.55,
            ..a_gear()
        };
        let (which, pos, size, height) = biggest_ring(&gear);
        assert_eq!(which, 1, "the middle ring is the widest here");
        assert!((size - 0.2).abs() < 1e-9, "size {size}");
        assert!((pos - 0.7).abs() < 1e-9, "at {pos}");
        assert!((height - gear.thickness2).abs() < 1e-9);

        // Spokes take the middle ring out of the running.
        let spoked = Gear { spokes: 6, ..gear };
        assert_eq!(biggest_ring(&spoked).0, 0);
    }
}
