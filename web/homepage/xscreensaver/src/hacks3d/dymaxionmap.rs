//! Port of `hacks/glx/dymaxionmap.c`.
//!
//! ```text
//! dymaxionmap --- Buckminster Fuller's unwrapped icosahedral globe.
//! Copyright © 2016-2026 Jamie Zawinski.
//!
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
//! ```
//!
//! Fuller's map of the Earth: the globe wrapped in an icosahedron, each face
//! projected onto its plane, and the solid cut open along a line that runs
//! entirely through ocean, so the continents come out as one island. The
//! saver lies the net out flat, folds it up into the icosahedron, spikes each
//! face out into a stellated ball, spins that on the Earth's real axis, and
//! unfolds it again. The dusk terminator creeps across it the whole time.
//!
//! The projection is [`crate::runtime::dymaxion`]. Three things about the way
//! this draws it are not upstream's.
//!
//! Upstream keeps the texture coordinates in step with the folding by pushing
//! the same rotations onto the `GL_TEXTURE` matrix stack that it pushes onto
//! the modelview one, and complains in a comment that iOS only gives it four
//! levels of texture stack. This runtime has no texture matrix at all, so the
//! same two-by-three affine is carried down the recursion by hand and applied
//! to each coordinate as it is emitted. That is what a texture matrix is, and
//! there is no depth limit on a parameter.
//!
//! Upstream blends the day and night maps through a sliding dusk mask and
//! *then* projects the result, which is two and a half million operations
//! every time the terminator moves. Here the day map, the night map and a map
//! of what longitude each texel came from are each projected once at startup,
//! and the blend happens in Dymaxion space against that longitude: a fifth of
//! the work, and the same picture, because averaging and blending are both
//! linear and commute.
//!
//! And upstream caches every one of the seven hundred and twenty terminator
//! positions when it has the memory for it, which at these map sizes would be
//! 1.5 GB. This keeps one, and recomputes, which is what upstream does on a
//! phone.
//!
//! The one cost that could not be moved is building the projection lookup
//! itself: two million calls to work out where each half pixel of the map
//! goes, about a second before the first frame. Upstream shows a loading
//! message during it. There is nowhere to show one here, because a saver's
//! `init` runs to completion before anything is drawn.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::dymaxion;
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::opts::SelectItem;
use crate::runtime::shapes::{calc_normal, unit_sphere};
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

/// Half the height of an equilateral triangle with unit sides.
const H: f32 = 0.866_025_4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Startup,
    Flat,
    Fold,
    Ico,
    StelIn,
    Axis,
    Spin,
    Stel,
    StelOut,
    Ico2,
    Unfold,
}

/// A two-by-three affine, which is all the texture matrix ever holds here:
/// `[a, b, c, d, tx, ty]` sending `(x, y)` to `(a x + c y + tx, b x + d y + ty)`.
#[derive(Clone, Copy)]
struct Affine([f32; 6]);

impl Affine {
    const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let m = &self.0;
        (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
    }

    /// `self * other`, in the order `glMultMatrix` composes them: the new one
    /// acts on the coordinate first.
    fn mul(&self, o: &Affine) -> Affine {
        let (a, b) = (&self.0, &o.0);
        Affine([
            a[0] * b[0] + a[2] * b[1],
            a[1] * b[0] + a[3] * b[1],
            a[0] * b[2] + a[2] * b[3],
            a[1] * b[2] + a[3] * b[3],
            a[0] * b[4] + a[2] * b[5] + a[4],
            a[1] * b[4] + a[3] * b[5] + a[5],
        ])
    }

    fn translate(x: f32, y: f32) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, x, y])
    }

    fn rotate(degrees: f32) -> Affine {
        let (s, c) = degrees.to_radians().sin_cos();
        Affine([c, s, -s, c, 0.0, 0.0])
    }

    fn scale(x: f32, y: f32) -> Affine {
        Affine([x, 0.0, 0.0, y, 0.0, 0.0])
    }
}

/// A star, and how big to draw it.
struct Star {
    p: [f32; 3],
    c: [f32; 3],
    size: f32,
}

/// The maps, projected onto the net once and blended per frame after that.
struct Maps {
    w: usize,
    h: usize,
    /// The day map, in Dymaxion space, as packed RGBA.
    day: Vec<u32>,
    /// The night map, likewise, or empty if there is only one map.
    night: Vec<u32>,
    /// What longitude each texel of the net came from, in turns from nought
    /// to one, or -1 where no part of the map lands.
    lon: Vec<f32>,
    /// And what latitude, in radians. The terminator depends on both.
    lat: Vec<f32>,
    /// How lit a texel is, worked out in advance as far as it can be: the
    /// cosine of the angle to the sun is `p cos(2 pi t) + q sin(2 pi t) + r`
    /// where `t` is how far round the day it is, so all that is left to do
    /// per frame is three multiplies and a comparison.
    pqr: Vec<[f32; 3]>,
    /// The colour of the ocean, for the parts of the net that are outside
    /// the world.
    ocean: u32,
    /// Which way the Earth's axis leans against the sun this time round.
    axial_tilt: f32,
}

struct Dymaxion {
    rot: Rotator,
    rot2: Rotator,
    trackball: Trackball,
    font: Option<TexFont>,

    state: State,
    ratio: f32,
    speed: f32,

    do_roll: bool,
    do_stars: bool,
    do_texture: bool,
    wireframe: bool,

    maps: Option<Maps>,
    nimages: i32,
    current_frame: f64,
    /// Which terminator position the texture currently holds.
    drawn_frame: i32,
    tex_map: u32,
    tex_ground: u32,
    stars: Vec<Star>,

    /// The texture transform as the recursion stands, standing in for the
    /// `GL_TEXTURE` matrix stack.
    tex: Affine,
}

/// `create_daylight_mask`, as a function rather than an image: how lit a
/// point at this longitude and latitude is, from nought to one.
///
/// This is upstream's arithmetic written out plainly. It is not what runs,
/// because it is an arc cosine a texel and there are half a million of them
/// every time the terminator moves; [`Maps::at`] folds everything that does
/// not change with the time of day into three numbers a texel instead. This
/// stays as the thing that says what that is supposed to come to, and a test
/// holds the two together.
#[cfg(test)]
fn daylight(lon_turns: f32, lat: f32, tilt: f32) -> f32 {
    let dusk = std::f32::consts::PI * 0.035;
    let sun = [0.0, tilt.cos(), tilt.sin()];
    let lon = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * lon_turns;
    let (sin_lat, cos_lat) = lat.sin_cos();
    let v = [lon.cos() * cos_lat, lon.sin() * cos_lat, sin_lat];
    let la = 1.0f32;
    let lb = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if lb == 0.0 {
        return 1.0;
    }
    let cc = ((sun[0] * v[0] + sun[1] * v[1] + sun[2] * v[2]) / (la * lb)).clamp(-1.0, 1.0);
    let a = cc.acos() - std::f32::consts::FRAC_PI_2;
    if a < -dusk {
        1.0
    } else if a >= dusk {
        0.0
    } else {
        (dusk - a) / (dusk * 2.0)
    }
}

/// `add_grid_lines`: faint lines of latitude and longitude drawn into the map
/// before it is projected, so that they curve with the faces.
fn add_grid_lines(px: &mut [u8], w: usize, h: usize) {
    let off = 24i32;
    let mut shade = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        let mean = (i32::from(px[i]) + i32::from(px[i + 1]) + i32::from(px[i + 2])) / 3;
        let d = if mean < 0x7F { off } else { -off };
        for k in 0..3 {
            px[i + k] = (i32::from(px[i + k]) + d).clamp(0, 0xFF) as u8;
        }
    };
    for i in 0..24 {
        let x = ((i as f64 + 0.5) * w as f64 / 24.0) as usize;
        for y in 0..h {
            shade(x.min(w - 1), y);
        }
    }
    for i in 1..11 {
        let y = (i as f64 * h as f64 / 12.0) as usize;
        for x in 0..w {
            shade(x, y.min(h - 1));
        }
    }
}

/// `adjust_brightness`.
fn adjust_brightness(px: &mut [u8], amount: f32) {
    for c in px.chunks_exact_mut(4) {
        for v in &mut c[..3] {
            *v = ((f32::from(*v) * amount) as i32).clamp(0, 0xFF) as u8;
        }
    }
}

fn pack(c: &[u8]) -> u32 {
    u32::from(c[0]) | u32::from(c[1]) << 8 | u32::from(c[2]) << 16 | 0xFF00_0000
}

impl Maps {
    /// Decode the two maps, draw the grid on them, and project them and a
    /// record of where each texel came from onto the unfolded net.
    fn build(satellite: bool, grid: bool) -> Option<Maps> {
        let (dayb, nightb) = if satellite {
            (crate::images::EARTH, Some(crate::images::EARTH_NIGHT))
        } else {
            (crate::images::EARTH_FLAT, None)
        };
        let (w, h, mut day) = crate::runtime::png::decode_rgba(dayb)?;
        let (w, h) = (w as usize, h as usize);
        let mut night = match nightb {
            Some(b) => crate::runtime::png::decode_rgba(b).map(|(_, _, p)| p),
            // The same map for both, made much darker, which is what
            // upstream does when day and night name the same picture.
            None => Some(day.clone()),
        }?;

        if grid {
            add_grid_lines(&mut day, w, h);
            add_grid_lines(&mut night, w, h);
        }
        /* Make the day image brighter, because that's easier than doing it
        with GL lights. */
        adjust_brightness(&mut day, 1.4);
        adjust_brightness(&mut night, if satellite { 0.7 } else { 0.2 });

        // R'Lyeh, which is ocean, and stands in for the parts of the net
        // that no part of the world lands on.
        let ocean = {
            let x = ((-123.39 + 180.0) * w as f64 / 360.0) as usize;
            let y = ((-48.44 + 90.0) * h as f64 / 180.0) as usize;
            pack(&day[(y.min(h - 1) * w + x.min(w - 1)) * 4..])
        };

        let mut out = Maps {
            w,
            h,
            day: vec![0; w * h],
            night: vec![0; w * h],
            lon: vec![-1.0; w * h],
            lat: vec![0.0; w * h],
            pqr: Vec::new(),
            ocean,
            axial_tilt: (frand(23.4) as f32).to_radians()
                * if random() & 1 != 0 { 1.0 } else { -1.0 },
        };

        // The projection, at every half pixel so that no texel of the net is
        // missed. Where two source pixels land on the same one they are
        // averaged, which upstream does too: without it the grid lines look
        // terrible.
        let mut seen = vec![false; w * h];
        for y2 in 0..h * 2 {
            let y = y2 as f64 / 2.0;
            let lat = -90.0 + (180.0 * y / h as f64);
            for x2 in 0..w * 2 {
                let x = x2 as f64 / 2.0;
                let lon = -180.0 + (360.0 * x / w as f64);
                let (ox, oy) = dymaxion::convert(lon, lat);
                let dx = ((w as f64 - (w as f64 * ox / dymaxion::WIDTH)) as usize).min(w - 1);
                let dy = ((h as f64 * oy / dymaxion::HEIGHT) as usize).min(h - 1);
                let to = dy * w + dx;
                let from = ((y as usize).min(h - 1) * w + (x as usize).min(w - 1)) * 4;
                let (d, n) = (pack(&day[from..]), pack(&night[from..]));
                if seen[to] {
                    out.day[to] = mean(out.day[to], d);
                    out.night[to] = mean(out.night[to], n);
                } else {
                    out.day[to] = d;
                    out.night[to] = n;
                    out.lon[to] = (x / w as f64) as f32;
                    out.lat[to] = lat.to_radians() as f32;
                    seen[to] = true;
                }
            }
        }

        // Fold everything about the terminator that does not change with the
        // time of day into three numbers a texel.
        let (sin_t, cos_t) = out.axial_tilt.sin_cos();
        out.pqr = (0..w * h)
            .map(|i| {
                let lat = out.lat[i];
                let th0 = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * out.lon[i];
                let (sin_lat, cos_lat) = lat.sin_cos();
                let (sin_th, cos_th) = th0.sin_cos();
                [
                    cos_lat * cos_t * sin_th,
                    cos_lat * cos_t * cos_th,
                    sin_lat * sin_t,
                ]
            })
            .collect();
        Some(out)
    }

    /// The net at one position of the terminator, as RGBA ready to upload.
    fn at(&self, frame: i32, nimages: i32) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h * 4];
        let shift = frame as f32 / nimages as f32;
        let (s, c) = (std::f32::consts::TAU * shift).sin_cos();
        let dusk = std::f32::consts::PI * 0.035;
        let edge = dusk.sin();
        for i in 0..self.w * self.h {
            {
                let p = if self.lon[i] < 0.0 {
                    self.ocean
                } else {
                    let k = self.pqr[i];
                    let dot = k[0] * c + k[1] * s + k[2];
                    // Eight texels in nine are in full day or full night;
                    // only the narrow band between them needs the arc
                    // cosine that upstream computes everywhere.
                    let r = if dot > edge {
                        1.0
                    } else if dot <= -edge {
                        0.0
                    } else {
                        (dusk - (dot.clamp(-1.0, 1.0).acos() - std::f32::consts::FRAC_PI_2))
                            / (dusk * 2.0)
                    };
                    blend(self.day[i], self.night[i], r)
                };
                out[i * 4] = (p & 0xFF) as u8;
                out[i * 4 + 1] = ((p >> 8) & 0xFF) as u8;
                out[i * 4 + 2] = ((p >> 16) & 0xFF) as u8;
                out[i * 4 + 3] = 0xFF;
            }
        }
        out
    }
}

fn mean(a: u32, b: u32) -> u32 {
    let mut out = 0xFF00_0000;
    for k in 0..3 {
        let s = k * 8;
        out |= ((((a >> s) & 0xFF) + ((b >> s) & 0xFF)) >> 1) << s;
    }
    out
}

fn blend(day: u32, night: u32, r: f32) -> u32 {
    let mut out = 0xFF00_0000;
    for k in 0..3 {
        let s = k * 8;
        let v = ((day >> s) & 0xFF) as f32 * r + ((night >> s) & 0xFF) as f32 * (1.0 - r);
        out |= ((v as u32) & 0xFF) << s;
    }
    out
}

impl Dymaxion {
    /// `triangle0`: one face of the icosahedron, as six sub-triangles, so
    /// that a face can be drawn in part and so that it can be spiked out into
    /// a stellation.
    ///
    /// ```text
    ///                A
    ///               / \
    ///              / | \
    ///             /  |  \
    ///            / 0 | 1 \
    ///        E  /_   |   _\  F
    ///          /  \_ | _/  \
    ///         / 5   \D/   2 \
    ///        /    /  |  \    \
    ///       /   / 4  | 3  \   \
    ///      /  /      |       \ \
    ///   B ----------------------- C
    ///                G
    /// ```
    fn triangle0(
        &self,
        g: &mut Gl,
        frontp: bool,
        stel: f32,
        facemask: u32,
        corners: &mut [[f32; 3]; 7],
    ) {
        let wire = self.wireframe;
        let h2 = (H * H - (H / 2.0) * (H / 2.0)).sqrt() - 0.5;
        let ta = [0.0, H, 0.0];
        let tb = [-0.5, 0.0, 0.0];
        let tc = [0.5, 0.0, 0.0];
        let td = [0.0, H / 3.0, 0.0];
        let te = [-h2, H / 2.0, 0.0];
        let tf = [h2, H / 2.0, 0.0];
        let tg = [0.0, 0.0, 0.0];

        /* Eyeballed this to find the depth of stellation that seems to most
        approximate a sphere. */
        let mut d = td;
        d[2] = 0.193 * stel;

        /* We want to raise E, F and G as well but we can't just shift Z: we
        need to keep them on the same vector from the center of the sphere,
        which means also changing F and G's X and Y. */
        let magic_x = 0.044f32;
        let magic_y = 0.028f32;
        let mut e = te;
        let mut f = tf;
        let mut gg = tg;
        e[2] = 0.132 * stel;
        f[2] = 0.132 * stel;
        gg[2] = 0.132 * stel;
        gg[1] -= (magic_x * magic_x + magic_y * magic_y).sqrt() * stel;
        e[0] -= magic_x * stel;
        e[1] += magic_y * stel;
        f[0] += magic_x * stel;
        f[1] += magic_y * stel;

        /// One sub-triangle: three corners for the geometry, which the
        /// stellation moves, and three for the texture, which it does not.
        type Sub = ([f32; 3], [f32; 3], [f32; 3], [f32; 3], [f32; 3], [f32; 3]);
        let faces: [Sub; 7] = [
            (e, d, ta, te, td, ta),
            (d, f, ta, td, tf, ta),
            (d, tc, f, td, tc, tf),
            (gg, tc, d, tg, tc, td),
            (tb, gg, d, tb, tg, td),
            (tb, d, e, tb, td, te),
            (e, d, ta, te, td, ta),
        ];

        for (i, (a, b, c, sa, sb, sc)) in faces.iter().enumerate() {
            if facemask & (1 << i) == 0 {
                continue;
            }
            let n = if frontp {
                calc_normal(*a, *b, *c)
            } else {
                calc_normal(*b, *a, *c)
            };
            g.glx.begin(if wire {
                Shape::LineLoop
            } else {
                Shape::Triangles
            });
            g.glx.normal3f(n[0], n[1], n[2]);
            for (p, t) in [(a, sa), (b, sb), (c, sc)] {
                let (tx, ty) = self.tex.apply(t[0], t[1]);
                g.glx.tex_coord2f(tx, ty);
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
        }

        *corners = [ta, tb, tc, d, e, f, gg];
    }

    /// One face of the net, and then, by recursion, everything folded onto
    /// it. The faces are numbered from the top left of the net; number twelve
    /// is in the middle and is where the walk starts.
    fn triangle(&mut self, g: &mut Gl, which: usize, frontp: bool, fold: f32, stel: f32) {
        let wire = self.wireframe;
        let max = (5.0f32.sqrt() / 3.0).acos();
        let rot = -max * fold / (std::f32::consts::PI / 180.0);
        let mut corners = [[0.0f32; 3]; 7];

        g.glx.color3f(1.0, 1.0, 1.0);
        if !wire {
            g.glx.material_ambient_diffuse([1.0, 1.0, 1.0, 1.0]);
        }

        let mask = match which {
            3 => 1 << 3 | 1 << 4,                   /* One third of the face. */
            4 => 1 << 1 | 1 << 2 | 1 << 3 | 1 << 4, /* Two thirds: convex. */
            6 => 1 << 1 | 1 << 2 | 1 << 3,          /* One half of the face. */
            7 => 1 << 2 | 1 << 3 | 1 << 4,          /* One half of the face. */
            _ => 0x3F,                              /* Full face. */
        };
        self.triangle0(g, frontp, stel, mask, &mut corners);

        if wire && let Some(font) = &self.font {
            g.glx.color3f(0.3, 0.3, 0.3);
            g.glx.push_matrix();
            g.glx.translate(-0.1, 0.2, 0.0);
            g.glx.scale(0.005, 0.005, 0.005);
            font.print_string(&mut g.glx, &which.to_string());
            g.glx.pop_matrix();
        }

        /* The connection hierarchy of the faces starting at the middle. */
        let (a, b): (i32, i32) = match which {
            1 => (0, -1),
            2 => (-1, 3),
            4 => (-1, 5),
            5 => (-1, 6),
            8 => (17, 7),
            9 => (8, -1),
            10 => (18, 9),
            11 => (10, 1),
            12 => (11, 13),
            13 => (2, 14),
            14 => (15, 20),
            15 => (4, 16),
            20 => (21, 19),
            _ => (-1, -1),
        };

        for (next, side) in [(a, -1.0f32), (b, 1.0)] {
            if next < 0 {
                continue;
            }
            g.glx.push_matrix();
            g.glx.translate(side * 0.5, 0.0, 0.0);
            g.glx.rotate(-side * 60.0, 0.0, 0.0, 1.0);
            g.glx.translate(-side * 0.5, 0.0, 0.0);

            // The same transform on the texture coordinates, which upstream
            // gets from the GL_TEXTURE matrix stack.
            let saved = self.tex;
            self.tex = self
                .tex
                .mul(&Affine::translate(side * 0.5, 0.0))
                .mul(&Affine::rotate(-side * 60.0))
                .mul(&Affine::translate(-side * 0.5, 0.0));

            g.glx.rotate(rot, 1.0, 0.0, 0.0);
            self.triangle(g, next as usize, frontp, fold, stel);

            self.tex = saved;
            g.glx.pop_matrix();
        }

        /* Draw a border around the edge of the world. */
        if wire || !frontp || stel != 0.0 || fold >= 0.95 {
            return;
        }
        let edges: u32 = match which {
            0 | 16 | 21 | 19 | 18 | 17 => 1 << 0 | 1 << 2,
            1 | 9 => 1 << 2,
            2 => 1 << 0,
            3 => 1 << 3 | 1 << 4,
            4 => 1 << 3 | 1 << 5,
            5 => 1 << 0 | 1 << 6,
            6 => 1 << 2 | 1 << 7,
            12 => 1 << 1,
            7 => 1 << 8 | 1 << 9,
            _ => 0,
        };
        if edges == 0 {
            return;
        }
        let pairs = [
            (0, 1),
            (1, 2),
            (2, 0),
            (1, 3),
            (3, 2),
            (3, 0),
            (0, 5),
            (0, 6),
            (1, 5),
            (5, 2),
        ];
        g.glx.texturing(false);
        g.glx.lighting(false);
        g.glx.color4f(0.0, 0.2, 0.5, 1.0 - fold);
        g.glx.begin(Shape::Lines);
        for (i, (p, q)) in pairs.iter().enumerate() {
            if edges & (1 << i) == 0 {
                continue;
            }
            for c in [corners[*p], corners[*q]] {
                g.glx.vertex3f(c[0], c[1], c[2]);
            }
        }
        g.glx.end();
        if self.do_texture {
            g.glx.texturing(true);
        }
        g.glx.lighting(true);
    }

    /// `draw_triangles`: the front of the net with the map on it, then the
    /// back with the ground texture.
    fn draw_triangles(&mut self, g: &mut Gl, fold: f32, stel: f32) {
        let c = H / 3.0;
        g.glx.translate(0.0, -H / 3.0, 0.0); /* Center on face 12 */
        /* When closed, center on midpoint of icosahedron. Eyeballed this. */
        g.glx.translate(0.0, 0.0, fold * 0.754);
        g.glx.front_face_cw(false);

        /* Adjust the texture matrix so that it has the same coordinate space
        as the model. */
        let base = Affine::scale(1.0 / 5.5, -1.0 / (3.0 * H)).mul(&Affine::translate(2.5, 3.0 * c));

        for (front, tex) in [(true, self.tex_map), (false, self.tex_ground)] {
            if self.wireframe || !self.do_texture {
                g.glx.texturing(false);
            } else {
                g.glx.texturing(true);
                g.glx.bind_texture(tex);
            }
            g.glx.front_face_cw(!front);
            self.tex = base;
            self.triangle(g, 12, front, fold, if front { stel } else { 0.0 });
        }
    }

    /// `align_axis`: line an axis up with the north and south poles on the
    /// map, which are not in the middle of their faces or anywhere else that
    /// is easy to work out.
    fn align_axis(g: &mut Gl, undo: bool) {
        let (r1, r2) = (20.5, 28.5);
        if undo {
            g.glx.rotate(-r2, 0.0, 1.0, 0.0);
            g.glx.rotate(r2, 1.0, 0.0, 0.0);
            g.glx.rotate(-r1, 1.0, 0.0, 0.0);
        } else {
            g.glx.rotate(r1, 1.0, 0.0, 0.0);
            g.glx.rotate(-r2, 1.0, 0.0, 0.0);
            g.glx.rotate(r2, 0.0, 1.0, 0.0);
        }
    }

    /// `draw_axis`: the wireframe globe and its axis, over the stellated ball.
    fn draw_axis(g: &mut Gl) {
        g.glx.texturing(false);
        g.glx.lighting(false);
        g.glx.push_matrix();
        Self::align_axis(g, false);
        g.glx.translate(0.34, 0.39, -0.61);
        let s = 0.96; /* tighten up the enclosing sphere */
        g.glx.scale(s, s, s);
        g.glx.color3f(0.5, 0.5, 0.0);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0); /* unit_sphere is off by 90 */
        g.glx.rotate(9.5, 0.0, 1.0, 0.0); /* line up the time zones */
        g.glx.front_face_cw(false);
        unit_sphere(&mut g.glx, 12, 24, true);
        g.glx.begin(Shape::Lines);
        g.glx.vertex3f(0.0, -2.0, 0.0);
        g.glx.vertex3f(0.0, 2.0, 0.0);
        g.glx.end();
        g.glx.pop_matrix();
    }

    /// `init_stars`.
    fn init_stars(&mut self, width: i32, height: i32) {
        let size = width.max(height);
        let nstars = size * size / 80;
        let steps = 6;
        let inc = 0.5f32;
        for j in 1..=steps {
            for _ in 0..nstars / steps {
                let d = 0.1;
                let r = 0.15 + frand(0.3) as f32;
                let gc = r + frand(d) as f32 - d as f32;
                let b = r + frand(d) as f32 - d as f32;
                let x = frand(1.0) as f32 - 0.5;
                let y = frand(1.0) as f32 - 0.5;
                let z = if random() & 1 != 0 {
                    frand(1.0) as f32 - 0.5
                } else {
                    ((frand(1.0) + frand(1.0) + frand(1.0)) as f32 / 3.0 - 0.5) / 12.0
                };
                let len = (x * x + y * y + z * z).sqrt();
                self.stars.push(Star {
                    p: [x / len, y / len, z / len],
                    c: [r, gc, b],
                    size: inc * j as f32,
                });
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wireframe = g.res.bool("wireframe");
    let do_texture = g.res.bool("texture") && !wireframe;
    let do_roll = g.res.bool("roll");
    let do_wander = g.res.bool("wander");
    let satellite = g.res.string("image").eq_ignore_ascii_case("BUILTIN_SAT");

    let spin_speed = 0.1;
    let wander_speed = 0.002;
    let mut this = Dymaxion {
        rot: Rotator::new(
            if do_roll { spin_speed } else { 0.0 },
            if do_roll { spin_speed } else { 0.0 },
            0.0,
            1.0,
            if do_wander { wander_speed } else { 0.0 },
            false,
        ),
        rot2: Rotator::new(0.0, 0.0, 0.0, 0.0, wander_speed, false),
        trackball: Trackball::new(),
        font: wireframe.then(|| TexFont::load(&mut g.glx, "sans-serif bold 24")),
        state: State::Startup,
        ratio: 0.0,
        speed: (g.res.float("speed") as f32).max(0.01),
        do_roll,
        do_stars: g.res.bool("stars"),
        do_texture,
        wireframe,
        maps: None,
        nimages: g.res.int("frames").clamp(1, 1440),
        current_frame: 0.0,
        drawn_frame: -1,
        tex_map: 0,
        tex_ground: 0,
        stars: Vec::new(),
        tex: Affine::IDENTITY,
    };

    if do_texture {
        this.maps = Maps::build(satellite, g.res.bool("grid"));
        if this.maps.is_none() {
            this.do_texture = false;
        }
        this.tex_map = g.glx.gen_texture();
        if let Some((w, h, px)) = crate::runtime::png::decode_rgba(crate::images::GROUND) {
            let id = g.glx.gen_texture();
            g.glx.bind_texture(id);
            g.glx.tex_image_2d(w, h, px);
            g.glx.tex_clamp(false);
            g.glx.tex_nearest(false);
            this.tex_ground = id;
        }
    }
    this.current_frame = frand(this.nimages as f64);

    if this.do_stars {
        let (w, h) = (g.width(), g.height());
        this.init_stars(w, h);
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Dymaxion {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width as f32;
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -h, h, 5.0, 200.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.translate(0.0, 0.0, -40.0);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event
            && matches!(key, ' ' | '\t' | '\r' | '\n')
        {
            // Switch between the satellite and the flat map, keeping where
            // the terminator has got to.
            let was = self.maps.as_ref().map(|m| m.night.is_empty());
            let satellite = was == Some(true);
            if let Some(m) = Maps::build(satellite, true) {
                self.maps = Some(m);
                self.drawn_frame = -1;
            }
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let down = self.trackball.button_down();
        let speed = self.speed;

        if !down {
            self.ratio += speed
                * match self.state {
                    State::Startup => 0.01,
                    /* Stay flat longer if animating day and night. */
                    State::Flat => 0.005 * if self.nimages <= 1 { 1.0 } else { 0.3 },
                    State::Fold | State::Ico | State::Stel | State::Unfold => 0.01,
                    State::StelIn => 0.05,
                    State::StelOut | State::Ico2 => 0.07,
                    State::Axis => 0.02,
                    State::Spin => 0.005,
                };
        }

        if self.ratio > 1.0 {
            self.ratio = 0.0;
            self.state = match self.state {
                State::Startup => State::Flat,
                State::Flat => State::Fold,
                State::Fold => State::Ico,
                State::Ico => State::StelIn,
                State::StelIn => State::Stel,
                State::Stel => match random() % 7 {
                    0..=2 => State::StelOut,
                    3..=5 => State::Spin,
                    _ => State::Axis,
                },
                State::Axis | State::Spin => State::StelOut,
                State::StelOut => State::Ico2,
                State::Ico2 => State::Unfold,
                State::Unfold => State::Flat,
            };
        }

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.color_material(true);

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        if self.state != State::Startup {
            let (x, y, _) = self.rot.position(!down);
            g.glx
                .translate((x as f32 - 0.5) * 3.0, (y as f32 - 0.5) * 3.0, 0.0);
        }
        if self.do_roll && self.state != State::Startup {
            let max = 65.0;
            let (x, y, _) = self.rot2.position(!down);
            g.glx.rotate(max / 2.0 - x as f32 * max, 1.0, 0.0, 0.0);
            g.glx.rotate(max / 2.0 - y as f32 * max, 0.0, 1.0, 0.0);
        }

        if self.do_stars {
            g.glx.texturing(false);
            g.glx.lighting(false);
            g.glx.push_matrix();
            g.glx.scale(60.0, 60.0, 60.0);
            g.glx.rotate(90.0, 1.0, 0.0, 0.0);
            g.glx.rotate(35.0, 1.0, 0.0, 0.0);
            let mut size = 0.0;
            for s in &self.stars {
                if s.size != size {
                    if size != 0.0 {
                        g.glx.end();
                    }
                    size = s.size;
                    g.glx.point_size(size);
                    g.glx.begin(Shape::Points);
                }
                g.glx.color3f(s.c[0], s.c[1], s.c[2]);
                g.glx.vertex3f(s.p[0], s.p[1], s.p[2]);
            }
            if size != 0.0 {
                g.glx.end();
            }
            g.glx.pop_matrix();
            g.glx.clear_depth();
        }

        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.blend(Blend::Alpha);
        }

        // Move the terminator on, and rebuild the map when it has moved far
        // enough to show.
        if self.do_texture {
            self.current_frame += 0.1 * f64::from(speed) * (f64::from(self.nimages) / 360.0);
            while self.current_frame >= f64::from(self.nimages) {
                self.current_frame -= f64::from(self.nimages);
            }
            let i = self.current_frame as i32;
            if i != self.drawn_frame
                && let Some(maps) = &self.maps
            {
                let px = maps.at(i, self.nimages);
                g.glx.bind_texture(self.tex_map);
                g.glx.tex_image_2d(maps.w as i32, maps.h as i32, px);
                g.glx.tex_clamp(false);
                g.glx.tex_nearest(false);
                self.drawn_frame = i;
            }
        }

        g.glx.translate(-0.5, -0.4, 0.0);
        g.glx.scale(2.6, 2.6, 2.6);

        let mut fold = 0.0f32;
        let mut stel = 0.0f32;
        match self.state {
            State::Fold => fold = self.ratio,
            State::Unfold => fold = 1.0 - self.ratio,
            State::Ico | State::Ico2 => fold = 1.0,
            State::Stel | State::Axis | State::Spin => {
                fold = 1.0;
                stel = 1.0;
            }
            State::StelIn => {
                fold = 1.0;
                stel = self.ratio;
            }
            State::StelOut => {
                fold = 1.0;
                stel = 1.0 - self.ratio;
            }
            State::Startup => {
                /* Tilt in from flat */
                let e = ease(Ease::InOutSine, f64::from(1.0 - self.ratio)) as f32;
                g.glx.rotate(-90.0 * e, 1.0, 0.0, 0.0);
            }
            State::Flat => {}
        }

        if self.state == State::Spin {
            Self::align_axis(g, false);
            let e = ease(Ease::InOutSine, f64::from(self.ratio)) as f32;
            g.glx.rotate(e * 360.0 * 3.0, 0.0, 0.0, 1.0);
            Self::align_axis(g, true);
        }

        let f = ease(Ease::InOutSine, f64::from(fold)) as f32;
        let s = ease(Ease::InOutSine, f64::from(stel)) as f32;
        self.draw_triangles(g, f, s);

        if self.state == State::Axis {
            Self::draw_axis(g);
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*labelFont:    sans-serif bold 24",
    "*roll:         True",
    "*wander:       True",
    "*texture:      True",
    "*stars:        True",
    "*grid:         True",
    "*speed:        1.0",
    "*image:        BUILTIN_FLAT",
    "*frames:       720",
];

const MAPS: &[SelectItem] = &[
    SelectItem {
        value: "BUILTIN_FLAT",
        label: "Flat map",
    },
    SelectItem {
        value: "BUILTIN_SAT",
        label: "Satellite map",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Animation speed", 0.05, 10.0, 0.05, 2, "1.0"),
    Opt::slider(
        "frames",
        "Day / night smoothness",
        24.0,
        1440.0,
        1.0,
        0,
        "720",
    ),
    Opt::select("image", "Map", MAPS, "BUILTIN_FLAT"),
    Opt::boolean("stars", "Stars", "true"),
    Opt::boolean("grid", "Lat / long", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("roll", "Roll", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "dymaxionmap",
    label: "Dymaxion Map",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2016",
        video: Some("https://www.youtube.com/watch?v=4LnO0UiccGs"),
        blurb: "Buckminster Fuller's map of the Earth projected onto the \
                surface of an unfolded icosahedron. It depicts the Earth's \
                continents as one island, or nearly contiguous land masses.",
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

    #[test]
    fn the_texture_transform_does_what_a_texture_matrix_would() {
        // The recursion walks a tree of faces, turning sixty degrees about
        // one corner or the other at each step, and the coordinates have to
        // turn with it. Going down and coming back has to leave things where
        // they were.
        let base = Affine::scale(1.0 / 5.5, -1.0 / (3.0 * H)).mul(&Affine::translate(2.5, 1.0));
        let there = base
            .mul(&Affine::translate(0.5, 0.0))
            .mul(&Affine::rotate(-60.0))
            .mul(&Affine::translate(-0.5, 0.0));
        let back = there
            .mul(&Affine::translate(0.5, 0.0))
            .mul(&Affine::rotate(60.0))
            .mul(&Affine::translate(-0.5, 0.0));
        for (x, y) in [(0.0, 0.0), (0.5, 0.866), (-0.5, 0.2)] {
            let a = base.apply(x, y);
            let b = back.apply(x, y);
            assert!(
                (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5,
                "{a:?} {b:?}"
            );
            // And going one step really does move things.
            let c = there.apply(x, y);
            assert!((a.0 - c.0).abs() + (a.1 - c.1).abs() > 1e-3);
        }
    }

    #[test]
    fn the_net_is_projected_once_and_the_terminator_moves_over_it() {
        // Building the maps is two million projections, so this does it once
        // and checks everything about it in one go.
        let m = Maps::build(false, false).expect("the flat map decodes");
        assert_eq!((m.w, m.h), (1024, 512));

        // Every texel of the net either records where it came from or is
        // outside the world, and the inside covers most of the box: twenty
        // triangles of a five and a half by three grid.
        let inside = m.lon.iter().filter(|&&l| l >= 0.0).count();
        let all = m.w * m.h;
        assert!(inside * 3 > all && inside < all, "{inside} of {all}");
        for &l in &m.lon {
            assert!(l < 0.0 || (0.0..=1.0).contains(&l), "longitude {l}");
        }

        // Two positions of the terminator give different pictures.
        let a = m.at(0, 720);
        let b = m.at(360, 720);
        assert_eq!(a.len(), all * 4);
        let differ = a
            .chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(p, q)| p != q)
            .count();
        assert!(differ > all / 10, "only {differ} texels changed");

        // And the parts outside the world stay the colour of the ocean
        // whatever the terminator does, so the folded edges do not tear.
        let ocean = [
            (m.ocean & 0xFF) as u8,
            ((m.ocean >> 8) & 0xFF) as u8,
            ((m.ocean >> 16) & 0xFF) as u8,
        ];
        for (i, &l) in m.lon.iter().enumerate() {
            if l < 0.0 {
                assert_eq!(&a[i * 4..i * 4 + 3], &ocean, "outside the world at {i}");
            }
        }
    }

    #[test]
    fn the_quick_terminator_agrees_with_the_plain_one() {
        // The fast path skips the arc cosine everywhere but the dusk band,
        // and rewrites the dot product as three multiplies. It has to come
        // to the same answer as upstream's arithmetic wherever it is asked.
        let tilt = 0.3f32;
        let (sin_t, cos_t) = tilt.sin_cos();
        let dusk = std::f32::consts::PI * 0.035;
        let edge = dusk.sin();
        let mut checked = 0;
        let mut in_band = 0;
        for i in 0..97 {
            let lon = i as f32 / 97.0;
            for j in 0..53 {
                let lat = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * (j as f32 / 52.0);
                let th0 = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * lon;
                let (sin_lat, cos_lat) = lat.sin_cos();
                let (sin_th, cos_th) = th0.sin_cos();
                let k = [
                    cos_lat * cos_t * sin_th,
                    cos_lat * cos_t * cos_th,
                    sin_lat * sin_t,
                ];
                for step in 0..12 {
                    let shift = step as f32 / 12.0;
                    let (s, c) = (std::f32::consts::TAU * shift).sin_cos();
                    let dot = k[0] * c + k[1] * s + k[2];
                    let quick = if dot > edge {
                        1.0
                    } else if dot <= -edge {
                        0.0
                    } else {
                        in_band += 1;
                        (dusk - (dot.clamp(-1.0, 1.0).acos() - std::f32::consts::FRAC_PI_2))
                            / (dusk * 2.0)
                    };
                    let plain = daylight((lon + shift).fract(), lat, tilt);
                    assert!(
                        (quick - plain).abs() < 1e-3,
                        "at {lon},{lat},{shift}: {quick} against {plain}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 50_000);
        // And the band really is narrow, which is why skipping it pays: the
        // part of a sphere within six degrees of a great circle is about a
        // ninth of it, so eight texels in nine skip the arc cosine.
        assert!(in_band * 7 < checked, "{in_band} of {checked} were in dusk");
    }

    #[test]
    fn it_folds_up_and_unfolds_again() {
        // Flat, then folded into a solid, then flat again: the depth the
        // geometry occupies is what says which.
        // The fold is in the matrix stack rather than in the vertices, so
        // the depth has to be measured after it: the net lying flat and
        // face-on has no depth at all, and the icosahedron has plenty.
        let mut r = start(StartArgs::new(
            320,
            240,
            "texture=false&stars=false&roll=false&wander=false&speed=10",
            20260812,
        ));
        let mut flat = 0;
        let mut solid = 0;
        for _ in 0..1200 {
            r.step();
            let f = r.frame();
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for b in &f.batches {
                for v in &f.vertices[b.first..b.first + b.count] {
                    let z = b.modelview.transform(v.pos)[2];
                    lo = lo.min(z);
                    hi = hi.max(z);
                }
            }
            if hi - lo < 0.01 {
                flat += 1;
            } else if hi - lo > 1.0 {
                solid += 1;
            }
        }
        assert!(flat > 20, "it was never flat: {flat}");
        assert!(solid > 20, "it never folded up: {solid}");
    }

    #[test]
    fn every_face_of_the_net_is_drawn() {
        // Twenty faces, of which two are cut in half, so twenty-two pieces,
        // and the recursion has to reach all of them.
        let mut r = start(StartArgs::new(
            320,
            240,
            "texture=false&stars=false",
            20260812,
        ));
        r.step();
        let f = r.frame();
        let tris: usize = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
            .map(|b| b.count / 3)
            .sum();
        // Both sides of the net, each face six sub-triangles except the five
        // that are drawn in part.
        assert!(tris >= 2 * (17 * 6 + 5 * 2), "only {tris} triangles");
    }
}
