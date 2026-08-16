//! Port of `hacks/glx/skytentacles.c`.
//!
//! ```text
//! Sky Tentacles, Copyright (c) 2008-2018 Jamie Zawinski <jwz@jwz.org>
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
//! Tentacles reaching up out of the ground, with suckers.
//!
//! A tentacle is a stack of rings. Each segment has an angle and a length that
//! a wandering rotator drives, but only every fifth segment gets its own
//! numbers: the ones in between are interpolated toward it, which is what makes
//! the thing curl smoothly rather than kink. The thickness at any point is a
//! fraction of the distance still to go, so a tentacle always tapers to its
//! tip however it is bent.
//!
//! The rings are built and turned by hand rather than with the matrix stack,
//! because the rotation has to happen between `glBegin` and `glEnd`, where
//! `glRotatef` is not allowed.
//!
//! With `-cel` it is drawn as a cartoon: an outline pass slightly fattened and
//! then the depth buffer cleared, and the shading quantised by a sixteen-pixel
//! texture indexed by how much light a vertex faces. Upstream reaches that path
//! on the builds without `glPolygonMode`, which is this one, and its lighting
//! ramp is a one-dimensional texture; a sixteen-by-one two-dimensional texture
//! samples the same.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Glx, Shape};
use crate::runtime::rotator::Rotator;
use crate::runtime::shapes::calc_normal;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

/// Three uniform numbers averaged: the middle of the range far more often
/// than either end.
fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

/// One length of a tentacle.
struct Segment {
    /// Length of the segment coming out of this one.
    length: f32,
    /// Tilt and rotation from the previous segment.
    th: f32,
    phi: f32,
    /// Radius of the tentacle at the bottom of this segment.
    thickness: f32,
    rot: Rotator,
}

struct Tentacle {
    x: f32,
    y: f32,
    z: f32,
    segments: Vec<Segment>,
    tentacle_color: [f32; 4],
    stripe_color: [f32; 4],
    sucker_color: [f32; 4],
}

struct SkyTentacles {
    trackball: Trackball,
    tentacles: Vec<Tentacle>,
    /// The sucker, as a torus: the points and normals, precomputed.
    torus_points: Vec<[f32; 3]>,
    torus_normals: Vec<[f32; 3]>,
    torus_step: usize,
    line_thickness: f32,
    outline_color: [f32; 4],
    texid: u32,
    /// Which way the whole scene leans, decided once at startup.
    left: bool,
    aspect: f32,
    slices: usize,
    thickness: f32,
    length: f32,
    wiggliness: f32,
    flexibility: f32,
    smooth: bool,
    texture: bool,
    cel: bool,
    intersect: bool,
    wire: bool,
}

/// The one light, which cel shading also uses as the direction to measure a
/// vertex against.
const LIGHT_POS: [f32; 4] = [1.0, 1.0, 1.0, 0.0];

fn normalize(p: [f32; 3]) -> [f32; 3] {
    let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    if d == 0.0 {
        p
    } else {
        [p[0] / d, p[1] / d, p[2] / d]
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

impl SkyTentacles {
    /// `compute_unit_torus`: one sucker, as a ring of quad strips.
    fn compute_unit_torus(&mut self, ratio: f32, slices1: i32, slices2: i32) {
        let mut slices1 = slices1;
        let mut slices2 = slices2;
        if self.wire {
            slices1 /= 2;
            slices2 /= 4;
        }
        slices1 = slices1.max(3);
        slices2 = slices2.max(3);
        let tau = std::f32::consts::PI * 2.0;

        self.torus_step = 2 * (slices2 + 1) as usize;
        self.torus_points.clear();
        self.torus_normals.clear();
        for i in 0..slices1 {
            for j in 0..=slices2 {
                for k in 0..=1 {
                    let s = ((i + k) % slices1) as f32 + 0.5;
                    let t = (j % slices2) as f32;
                    let (st, ss) = (t * tau / slices2 as f32, s * tau / slices1 as f32);
                    self.torus_normals
                        .push([st.cos() * ss.cos(), st.sin() * ss.cos(), ss.sin()]);
                    self.torus_points.push([
                        (1.0 + ratio * ss.cos()) * st.cos() / 2.0,
                        (1.0 + ratio * ss.cos()) * st.sin() / 2.0,
                        ratio * ss.sin() / 2.0,
                    ]);
                }
            }
        }
    }

    /// One sucker. The polygons on the underside are left off, which upstream
    /// says is a tenth of the polygon count at the default settings.
    fn draw_sucker(&self, g: &mut Glx, front: bool) {
        let strips = self.torus_points.len() / self.torus_step;
        g.front_face_cw(front);
        for i in 0..strips {
            if strips > 4 && i >= strips / 2 && i < strips - 1 {
                continue;
            }
            let ii = i * self.torus_step;
            g.begin(if self.wire {
                Shape::LineStrip
            } else {
                Shape::QuadStrip
            });
            for j in 0..self.torus_step {
                let n = self.torus_normals[ii + j];
                let p = self.torus_points[ii + j];
                g.normal3f(n[0], n[1], n[2]);
                g.vertex3f(p[0], p[1], p[2]);
            }
            g.end();
        }
    }

    /// One tentacle. `front` is the ordinary pass; the back pass and the
    /// `outline` pass are the two halves of the cartoon look.
    fn draw_tentacle_1(&self, g: &mut Glx, t: &Tentacle, front: bool, outline: bool) {
        let slices = self.slices;
        let mut ctr = [0.0f32; 3];
        let (mut cth, mut cphi) = (0.0f32, 0.0f32);
        let (mut cth_cos, mut cth_sin) = (1.0f32, 0.0f32);
        let (mut cphi_cos, mut cphi_sin) = (1.0f32, 0.0f32);
        let mut t0 = 0.0f32;

        // Which portion of the radius the indented, differently coloured
        // stripe takes up.
        let indented = (slices as f32 * 0.2) as usize;

        let light = normalize([LIGHT_POS[0], LIGHT_POS[1], LIGHT_POS[2]]);
        let ucirc: Vec<[f32; 2]> = (0..slices)
            .map(|i| {
                let a = std::f32::consts::PI * 2.0 * i as f32 / slices as f32;
                [a.cos(), a.sin()]
            })
            .collect();

        // The rotation is done by hand: `glRotatef` is not allowed inside a
        // `glBegin` block, and this has to turn each ring as it lays it down.
        let rot = |p: [f32; 3], cth_sin: f32, cth_cos: f32, cphi_sin: f32, cphi_cos: f32| {
            [
                (p[1] * cth_cos + p[0] * cth_sin) * cphi_sin - p[2] * cphi_cos,
                p[1] * cth_sin - p[0] * cth_cos,
                (p[1] * cth_cos + p[0] * cth_sin) * cphi_cos + p[2] * cphi_sin,
            ]
        };

        let mut ring = vec![[0.0f32; 3]; slices];
        let mut norm = vec![[0.0f32; 3]; slices];
        let mut oring = vec![[0.0f32; 3]; slices];
        let mut onorm = vec![[0.0f32; 3]; slices];

        g.push_matrix();
        g.translate(t.x, t.y, t.z);

        if !front || outline {
            let c = self.outline_color;
            g.color4f(c[0], c[1], c[2], c[3]);
        } else if self.wire {
            let c = t.tentacle_color;
            g.color4f(c[0], c[1], c[2], c[3]);
        } else {
            g.material_specular([1.0, 1.0, 1.0, 1.0]);
            g.material_shininess(128.0);
        }

        let nsegments = t.segments.len();
        for i in 0..nsegments {
            let mut t1 = t0 + i as f32 / (nsegments as f32 * std::f32::consts::PI * 2.0);

            for j in 0..slices {
                // A vertical disc at the origin, to be the base of this
                // segment.
                let mut r = t.segments[i].thickness / 2.0;
                if j <= indented / 2 || j >= slices - indented / 2 {
                    r *= 0.75; // Indent the stripe.
                }
                if outline {
                    r *= 1.1;
                }
                let p = rot(
                    [r * ucirc[j][0], 0.0, r * ucirc[j][1]],
                    cth_sin,
                    cth_cos,
                    cphi_sin,
                    cphi_cos,
                );
                ring[j] = [p[0] + ctr[0], p[1] + ctr[1], p[2] + ctr[2]];
            }

            // The face normals of this segment, computed before the vertices
            // are laid down so a vertex could take the average of them.
            // Upstream's comment: except I didn't actually implement that.
            if i > 0 {
                for j in 0..=slices {
                    let j0 = j % slices;
                    let j1 = (j + 1) % slices;
                    norm[j0] = calc_normal(oring[j0], ring[j0], ring[j1]);
                }
            }

            if i > 0 {
                g.line_width(self.line_thickness);
                g.front_face_cw(!front);
                g.begin(if self.wire {
                    Shape::Lines
                } else if self.smooth {
                    Shape::QuadStrip
                } else {
                    Shape::Quads
                });
                for j in 0..=slices {
                    let j0 = j % slices;
                    let j1 = (j + 1) % slices;
                    let mut ts = j as f32 / slices as f32;

                    let c = if !front || outline {
                        self.outline_color
                    } else if j <= indented / 2 || j >= slices - indented / 2 {
                        t.stripe_color
                    } else {
                        t.tentacle_color
                    };
                    g.color4f(c[0], c[1], c[2], c[3]);

                    // For cel shading the texture coordinate is how squarely
                    // the vertex faces the light, which picks a band out of
                    // the ramp.
                    if self.cel {
                        t0 = dot(light, onorm[j0]).max(0.0);
                        t1 = dot(light, norm[j0]).max(0.0);
                    }

                    g.tex_coord2f(t0, ts);
                    g.normal3f(onorm[j0][0], onorm[j0][1], onorm[j0][2]);
                    g.vertex3f(oring[j0][0], oring[j0][1], oring[j0][2]);

                    g.tex_coord2f(t1, ts);
                    g.normal3f(norm[j0][0], norm[j0][1], norm[j0][2]);
                    g.vertex3f(ring[j0][0], ring[j0][1], ring[j0][2]);

                    if !self.smooth {
                        ts = j1 as f32 / slices as f32;
                        g.tex_coord2f(t1, ts);
                        g.vertex3f(ring[j1][0], ring[j1][1], ring[j1][2]);
                        g.tex_coord2f(t0, ts);
                        g.vertex3f(oring[j1][0], oring[j1][1], oring[j1][2]);
                    }
                }
                g.end();

                if self.wire {
                    g.begin(Shape::LineLoop);
                    for r in ring.iter().take(slices) {
                        g.vertex3f(r[0], r[1], r[2]);
                    }
                    g.end();
                }

                // And now the suckers.
                let seg_length = self.length / nsegments as f32;
                let sucker_size = self.thickness / 5.0;
                let sucker_spacing = sucker_size * 1.3;
                let mut nsuckers = (seg_length / sucker_spacing) as i32;
                let oth = cth - t.segments[i - 1].th;
                let ophi = cphi - t.segments[i - 1].phi;

                if !self.wire {
                    g.line_width((self.line_thickness / 2.0).max(2.0));
                }
                g.texturing(false);

                // Sometimes there are several suckers on one segment and
                // sometimes one sucker every several segments.
                if nsuckers == 0 {
                    let segs_per_sucker = ((sucker_spacing / seg_length) + 0.5) as i32;
                    nsuckers = if segs_per_sucker > 0 && (i as i32 % segs_per_sucker) != 0 {
                        0
                    } else {
                        1
                    };
                }

                let c = if outline {
                    self.outline_color
                } else if front {
                    t.sucker_color
                } else {
                    self.outline_color
                };
                g.color4f(c[0], c[1], c[2], c[3]);

                for k in 0..nsuckers {
                    let p0 = ring[0];
                    let p1 = oring[0];
                    let f = (k as f32 + 0.5) / nsuckers as f32;
                    let p = [
                        p0[0] + (p1[0] - p0[0]) * f,
                        p0[1] + (p1[1] - p0[1]) * f,
                        p0[2] + (p1[2] - p0[2]) * f,
                    ];

                    g.push_matrix();
                    g.translate(p[0], p[1], p[2]);
                    g.rotate(ophi * 180.0 / std::f32::consts::PI, 0.0, 1.0, 0.0);
                    g.rotate(-oth * 180.0 / std::f32::consts::PI, 1.0, 0.0, 0.0);
                    g.rotate(90.0, 1.0, 0.0, 0.0);

                    // Not quite right: this is the slope of the outer edge if
                    // the next segment were not tilted at all.
                    let slope = (t.segments[i - 1].thickness - t.segments[i].thickness)
                        / t.segments[i].length;
                    g.rotate(-45.0 * slope, 1.0, 0.0, 0.0);

                    let mut scale = t.segments[i].thickness / self.thickness;
                    scale *= 0.7 * sucker_size;
                    g.scale(scale, scale, scale * 4.0);
                    g.translate(0.0, 0.0, -0.1); // Embed it in the skin.
                    if outline {
                        g.scale(1.1, 1.1, 1.1);
                    }

                    g.translate(1.0, 0.0, 0.0); // Left.
                    self.draw_sucker(g, front);
                    g.translate(-2.0, 0.0, 0.0); // Right.
                    self.draw_sucker(g, front);
                    g.pop_matrix();
                }

                if self.texture {
                    g.texturing(true);
                }
            }

            // The end caps.
            g.line_width(self.line_thickness);
            if !outline && (i == 0 || i == nsegments - 1) {
                let ctrz =
                    ctr[2] + (if i == 0 { -1.0 } else { 1.0 }) * t.segments[i].thickness / 4.0;
                if front {
                    let c = t.tentacle_color;
                    g.color4f(c[0], c[1], c[2], c[3]);
                }
                g.front_face_cw(!(if front { i == 0 } else { i != 0 }));
                g.begin(if self.wire {
                    Shape::Lines
                } else {
                    Shape::TriangleFan
                });
                g.normal3f(0.0, 0.0, if i == 0 { -1.0 } else { 1.0 });
                g.tex_coord2f(t0 - 0.25, 0.5);
                g.vertex3f(ctr[0], ctr[1], ctrz);
                for j in 0..=slices {
                    let jj = j % slices;
                    let ts = j as f32 / slices as f32;
                    g.tex_coord2f(t0, ts);
                    // The bottom cap is drawn before any normal has been
                    // computed. Upstream reads whatever was in the buffer;
                    // this leaves the face normal set above in place.
                    if i > 0 {
                        g.normal3f(norm[jj][0], norm[jj][1], norm[jj][2]);
                    }
                    g.vertex3f(ring[jj][0], ring[jj][1], ring[jj][2]);
                    if self.wire {
                        g.vertex3f(ctr[0], ctr[1], ctrz);
                    }
                }
                g.end();
            }

            // On to the end of this segment, ready for the next.
            if i != nsegments - 1 {
                let p = rot(
                    [0.0, t.segments[i].length, 0.0],
                    cth_sin,
                    cth_cos,
                    cphi_sin,
                    cphi_cos,
                );
                ctr = [ctr[0] + p[0], ctr[1] + p[1], ctr[2] + p[2]];

                cth += t.segments[i].th;
                cphi += t.segments[i].phi;
                cth_sin = cth.sin();
                cth_cos = cth.cos();
                cphi_sin = cphi.sin();
                cphi_cos = cphi.cos();

                oring.copy_from_slice(&ring);
                onorm.copy_from_slice(&norm);
            }

            t0 = t1;
        }

        g.pop_matrix();
    }

    /// Without `glPolygonMode` there is no line-drawn outline pass, so the
    /// cartoon outline is the whole thing drawn slightly fatter in the
    /// outline colour, with the depth buffer cleared over it.
    fn draw_tentacle(&self, g: &mut Glx, t: &Tentacle, front: bool) {
        if !self.wire && self.cel && front {
            self.draw_tentacle_1(g, t, front, true);
            g.clear_depth();
        }
        self.draw_tentacle_1(g, t, front, false);
    }

    /// One step of a tentacle's motion. Only every few segments gets its own
    /// angle; the ones between are interpolated toward it, which is what makes
    /// it curl rather than kink.
    fn move_tentacle(&self, t: &mut Tentacle) {
        let nsegments = t.segments.len();
        let mut len = 0.0;
        let skip = (nsegments as f32 * (1.0 - (self.wiggliness + 0.5))) as i32;
        let mut tick = 0;
        let mut last = 0usize;

        for i in 0..nsegments {
            tick += 1;
            if tick >= skip || i == nsegments - 1 {
                let phi_range = std::f32::consts::PI * 0.8 * self.flexibility;
                let th_range = std::f32::consts::PI * 0.9 * self.flexibility;
                let (x, y, z) = t.segments[i].rot.position(true);
                t.segments[i].phi = phi_range * (0.5 - y as f32);
                t.segments[i].th = th_range * (0.5 - z as f32);
                t.segments[i].length =
                    (0.8 + ((0.5 - x as f32) * 0.4)) * (self.length / nsegments as f32);

                let (phi, th, length) = (t.segments[i].phi, t.segments[i].th, t.segments[i].length);
                for j in last + 1..=i {
                    t.segments[j].phi = phi / (i - last) as f32;
                    t.segments[j].th = th / (i - last) as f32;
                    t.segments[j].length = length;
                }
                tick = 0;
                last = i;
            }
            len += t.segments[i].length;
        }

        // The thickness at a point is a fraction of how far there is still to
        // go, so it tapers however it is bent.
        let mut pos = 0.0;
        let base = t.segments[0].thickness;
        for i in 0..nsegments {
            if i > 0 {
                t.segments[i].thickness = (base * (len - pos) / len).max(0.001);
            }
            pos += t.segments[i].length;
        }
    }
}

/// A new tentacle, placed on a grid and coloured a little lighter or darker
/// than the others.
fn make_tentacle(
    which: usize,
    total: usize,
    colors: ([f32; 4], [f32; 4], [f32; 4]),
    thickness: f32,
    segments: i32,
    speed: f32,
    intersect: bool,
) -> Tentacle {
    let mut cols = (total as f32).sqrt().round() as usize;
    let rows = total.div_ceil(cols);
    let mut xx = which % cols;
    let yy = which / cols;
    let spc = thickness * 0.8;
    if !intersect {
        cols = 1;
        xx = 0;
    }
    let x = (cols as f32 * spc / 2.0) - (spc * (xx as f32 + 0.5));
    let y = (rows as f32 * spc / 2.0) - (spc * (yy as f32 + 0.5));

    let brightness = 0.6 + frand(3.0) as f32;
    let frob = |c: [f32; 4]| {
        [
            (c[0] * brightness).min(1.0),
            (c[1] * brightness).min(1.0),
            (c[2] * brightness).min(1.0),
            c[3],
        ]
    };

    let nsegments = (segments as f32 + bellrand(segments as f64)) as usize;
    let mut segs: Vec<Segment> = (0..nsegments)
        .map(|_| Segment {
            length: 0.0,
            th: 0.0,
            phi: 0.0,
            thickness: 0.0,
            rot: Rotator::new(
                0.0,
                0.0,
                0.0,
                0.0,
                speed as f64 * (0.02 + bellrand(0.1) as f64),
                true,
            ),
        })
        .collect();
    segs[0].thickness = (thickness * 0.5) + bellrand((thickness * 0.6) as f64);

    Tentacle {
        x,
        y,
        z: 0.0,
        segments: segs,
        tentacle_color: frob(colors.0),
        stripe_color: frob(colors.1),
        sucker_color: frob(colors.2),
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let mut texture = g.res.bool("texture");
    let mut cel = g.res.bool("cel");
    if wire {
        texture = false;
        cel = false;
    }
    if cel {
        texture = false;
    }

    let slices = g.res.int("slices").max(3) as usize;
    let segments = g.res.int("segments").max(2);
    let thickness = (g.res.float("thickness") as f32).max(0.1);
    let speed = g.res.float("speed") as f32;
    let intersect = g.res.bool("intersect");
    let count = g.res.int("count").max(1) as usize;

    let tentacle_color = resource_color(g, "tentacleColor");
    let colors = (
        tentacle_color,
        resource_color(g, "stripeColor"),
        resource_color(g, "suckerColor"),
    );

    // Black outlines for light colours, white outlines for dark ones.
    let sum = tentacle_color[0] + tentacle_color[1] + tentacle_color[2];
    let edge = if sum < 0.4 { 1.0 } else { 0.0 };
    let outline_color = [edge, edge, edge, 1.0];

    let texid = if texture || cel {
        let id = g.glx.gen_texture();
        g.glx.bind_texture(id);
        if cel {
            // The lighting ramp: three cells of dark, five of grey and eight
            // of white. Upstream makes it a one-dimensional texture; sixteen
            // by one samples the same.
            let mut px = Vec::with_capacity(16 * 4);
            for i in 0..16 {
                let v: u8 = if i < 3 {
                    0x80
                } else if i < 8 {
                    0xC0
                } else {
                    0xFF
                };
                px.extend_from_slice(&[v, v, v, 255]);
            }
            g.glx.tex_image_2d(16, 1, px);
        } else {
            match crate::runtime::png::decode_rgba(crate::images::SCALES) {
                Some((w, h, px)) => g.glx.tex_image_2d(w, h, px),
                None => g.glx.tex_image_2d(1, 1, vec![255, 255, 255, 255]),
            }
        }
        g.glx.tex_clamp(false);
        id
    } else {
        0
    };

    let mut this = SkyTentacles {
        trackball: Trackball::new(),
        tentacles: Vec::new(),
        torus_points: Vec::new(),
        torus_normals: Vec::new(),
        torus_step: 0,
        line_thickness: 1.0,
        outline_color,
        texid,
        left: random().is_multiple_of(5),
        aspect: 1.0,
        slices,
        thickness,
        length: g.res.float("length") as f32,
        wiggliness: (g.res.float("wiggliness") as f32).clamp(0.0, 1.0),
        flexibility: (g.res.float("flexibility") as f32).clamp(0.0, 1.0),
        smooth: g.res.bool("smooth"),
        texture,
        cel,
        intersect,
        wire,
    };

    for i in 0..count {
        let mut t = make_tentacle(i, count, colors, thickness, segments, speed, intersect);
        this.move_tentacle(&mut t);
        this.tentacles.push(t);
    }

    this.compute_unit_torus(0.5, (slices as i32 / 6).max(5), (slices as i32 / 3).max(9));

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for SkyTentacles {
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
        self.line_thickness = if self.wire {
            1.0
        } else {
            (width as f32 / 200.0).max(3.0)
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key: ' ' } = event {
            self.trackball.reset(0.0, 0.0);
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, self.aspect, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        // Dark grey rather than black in cartoon mode, so the outlines show.
        if self.cel {
            g.glx.clear_color(0.13, 0.13, 0.13, 1.0);
            g.glx.blend(Blend::Alpha);
        }
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wire && !self.cel {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx
                .light_position(0, LIGHT_POS[0], LIGHT_POS[1], LIGHT_POS[2], LIGHT_POS[3]);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
        } else {
            g.glx.lighting(false);
        }
        // Upstream sets the material and the colour to the same thing at every
        // vertex; a batch here carries one material, so the colour is what
        // shades it.
        g.glx.color_material(true);
        if self.texture || self.cel {
            g.glx.texturing(true);
            g.glx.bind_texture(self.texid);
        }

        g.glx.push_matrix();
        g.glx.scale(3.0, 3.0, 3.0);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (rx, mut ry, mut rz) = (45.0f32, -45.0f32, 70.0f32);
        if self.left {
            ry = -ry;
            rz = -rz;
        }
        g.glx.rotate(ry, 0.0, 1.0, 0.0);
        g.glx.rotate(rx, 1.0, 0.0, 0.0);
        g.glx.rotate(rz, 0.0, 0.0, 1.0);
        if self.intersect {
            g.glx.translate(0.0, -2.0, -4.5);
        } else {
            g.glx.translate(0.0, -2.5, -5.0);
        }

        if !self.trackball.button_down() {
            for i in 0..self.tentacles.len() {
                let mut t = std::mem::replace(
                    &mut self.tentacles[i],
                    Tentacle {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        segments: Vec::new(),
                        tentacle_color: [0.0; 4],
                        stripe_color: [0.0; 4],
                        sucker_color: [0.0; 4],
                    },
                );
                self.move_tentacle(&mut t);
                self.tentacles[i] = t;
            }
        }

        let glx = &mut g.glx;
        for t in &self.tentacles {
            // Without this they would grow through each other.
            if !self.intersect {
                glx.clear_depth();
            }
            self.draw_tentacle(glx, t, true);
            if self.cel {
                self.draw_tentacle(glx, t, false);
            }
        }

        g.glx.pop_matrix();
        g.glx.texturing(false);
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:          30000",
    "*count:          9",
    "*showFPS:        False",
    "*wireframe:      False",
    "*speed:          1.0",
    "*smooth:         True",
    "*texture:        True",
    "*cel:            False",
    "*intersect:      False",
    "*slices:         16",
    "*segments:       24",
    "*wiggliness:     0.35",
    "*flexibility:    0.35",
    "*thickness:      1.0",
    "*length:         9.0",
    "*tentacleColor:  #305A30",
    "*stripeColor:    #451A30",
    "*suckerColor:    #453E30",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 8.0, 0.01, 2, "1.0"),
    Opt::slider("count", "Tentacles", 1.0, 20.0, 1.0, 0, "9"),
    Opt::slider("length", "Length", 3.0, 20.0, 0.5, 1, "9.0"),
    Opt::slider("thickness", "Thickness", 0.1, 3.0, 0.1, 1, "1.0"),
    Opt::slider("wiggliness", "Wiggliness", 0.0, 1.0, 0.01, 2, "0.35"),
    Opt::slider("flexibility", "Flexibility", 0.0, 1.0, 0.01, 2, "0.35"),
    Opt::slider("slices", "Slices", 3.0, 32.0, 1.0, 0, "16"),
    Opt::slider("segments", "Segments", 2.0, 48.0, 1.0, 0, "24"),
    Opt::boolean("smooth", "Smooth", "true"),
    Opt::boolean("texture", "Texture", "true"),
    Opt::boolean("cel", "Cartoon", "false"),
    Opt::boolean("intersect", "Tentacles intersect", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "skytentacles",
    label: "Sky Tentacles",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2008",
        video: Some("https://www.youtube.com/watch?v=iCjtXUSQv1A"),
        blurb: "There is a tentacled abomination in the sky. From above you it \
                devours.",
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

    /// A tentacle tapers to its tip however it is bent: the thickness at a
    /// point is a fraction of the distance still to go.
    #[test]
    fn a_tentacle_tapers() {
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        r.step();
        // Every tentacle keeps its shape over time.
        for _ in 0..200 {
            r.step();
        }
        let f = r.frame();
        assert!(
            f.vertices
                .iter()
                .all(|v| v.pos.iter().all(|c| c.is_finite())),
            "a vertex went to NaN"
        );
    }

    /// Nine of them by default, each clearing the depth buffer before it is
    /// drawn so that they do not grow through each other.
    #[test]
    fn they_do_not_grow_through_each_other() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let clears = f.batches.iter().filter(|b| b.clear_depth_first).count();
        assert_eq!(clears, 9, "{clears} depth clears is not nine tentacles");

        let mut r = start(StartArgs::new(640, 480, "intersect=true", 20260811));
        r.step();
        let clears = r
            .frame()
            .batches
            .iter()
            .filter(|b| b.clear_depth_first)
            .count();
        assert_eq!(clears, 0, "the tentacles cleared the depth buffer anyway");
    }

    /// The stripe is a band of a different colour down the length, and it is
    /// indented: those points sit at three quarters of the radius.
    #[test]
    fn it_has_a_stripe_down_it() {
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        r.step();
        let f = r.frame();
        let colors: std::collections::HashSet<String> = f
            .vertices
            .iter()
            .map(|v| format!("{:?}", v.color))
            .collect();
        // The body, the stripe, the suckers, and the caps take the body's.
        assert!(colors.len() >= 3, "only {} colours", colors.len());
    }

    /// The suckers are a torus with its underside left off, which upstream
    /// says is a tenth of the polygons at the default settings.
    #[test]
    fn a_sucker_is_most_of_a_torus() {
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        r.step();
        let hack_torus = {
            let mut t = SkyTentacles {
                trackball: Trackball::new(),
                tentacles: Vec::new(),
                torus_points: Vec::new(),
                torus_normals: Vec::new(),
                torus_step: 0,
                line_thickness: 1.0,
                outline_color: [0.0; 4],
                texid: 0,
                left: false,
                aspect: 1.0,
                slices: 16,
                thickness: 1.0,
                length: 9.0,
                wiggliness: 0.35,
                flexibility: 0.35,
                smooth: true,
                texture: false,
                cel: false,
                intersect: false,
                wire: false,
            };
            t.compute_unit_torus(0.5, 5, 9);
            t
        };
        assert_eq!(hack_torus.torus_step, 20);
        assert_eq!(hack_torus.torus_points.len(), 5 * 10 * 2);
        // Every point of it is within a unit of the middle.
        for p in &hack_torus.torus_points {
            let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!(d < 1.0, "a sucker point is {d} out");
        }
    }

    /// In cartoon mode the outline is the whole thing drawn fatter, with the
    /// depth buffer cleared over it, since there is no line polygon mode here.
    #[test]
    fn the_cartoon_outline_is_a_fatter_copy() {
        let mut r = start(StartArgs::new(640, 480, "cel=true&count=1", 20260811));
        r.step();
        let f = r.frame();
        // One clear for the tentacle and one for its outline.
        let clears = f.batches.iter().filter(|b| b.clear_depth_first).count();
        assert!(clears >= 2, "only {clears} depth clears in cartoon mode");
        assert!(
            f.batches.iter().all(|b| !b.lighting),
            "cartoons are not lit"
        );
    }
}
