/* timetunnel. Based on dangerball.c, hack by Sean Brennan <zettix@yahoo.com>*/
/* dangerball, Copyright (c) 2001-2018 Jamie Zawinski <jwz@jwz.org>
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Port of `hacks/glx/timetunnel.c`.
//!
//! An animation similar to the title sequence of Dr. Who in the 70s.
//!
//! It is a twenty-eight second film, not a simulation. Twelve effects run off
//! one clock, and each is a little table of keyframes: a time, and one to four
//! numbers to read off at that time. Everything on the screen is the linear
//! interpolation between the keyframe before now and the one after. The wall
//! tunnel opens out into the police-box silhouette between 2.77 and 3.07
//! seconds because the table says so, and it says so because that is when it
//! happens on the film.
//!
//! What is drawn is barely anything: a tunnel of thirty quads, a second one of
//! four, and a handful of textured quads floating at various depths. All of the
//! work is in *how* they are composited. Each quad is blended at a strength
//! that comes from the timeline rather than from the geometry, so the blending
//! is done with a blend constant, and one of them is drawn with the blend
//! equation reversed so that it subtracts itself out of the tunnel behind it
//! rather than adding to it. Those are [`Blend::ConstantFade`] and friends, put
//! into the runtime for this saver.
//!
//! Two other things about it are unusual here. It scrolls its tunnels by
//! moving the *texture matrix* rather than the geometry, and the runtime has
//! none, so the matrix is carried by hand as a 2x3 affine and applied as each
//! texture coordinate is written, the way `dymaxionmap` does. And upstream's
//! own OpenGL ES build gives up on the whole thing: `draw_sign` and `draw_cyl`
//! are both inside `#ifndef HAVE_JWZGLES`, so on a phone this saver draws a
//! wall tunnel and nothing else. The port is of the real one.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Fog, Shape, TexEnv};
use crate::runtime::png;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent,
    screenhack_event_helper,
};

const MAX_TEXTURE: usize = 10;
const CYL_LEN: f32 = 14.0;
const DIAMOND_LEN: f32 = 10.0;

/// How long the film is. Upstream's `effect_maxsecs`, and the ceiling the
/// start and end knobs are clamped to.
const EFFECT_MAXSECS: f32 = 30.0;

/* -------------------------------------------------------------------------
 * The timeline
 * ---------------------------------------------------------------------- */

/// One animated quantity: a list of keyframes, the values read off them, and
/// how fast the thing it drives scrolls.
///
/// Upstream calls a keyframe a knot. The first number in one is the time; the
/// rest are the values, so a knot of width three carries two of them.
struct Effect {
    knots: &'static [&'static [f32]],
    /// Which way, and how fast, the texture that goes with this effect
    /// scrolls. Only three of the twelve are asked for it.
    direction: f32,
    state: [f32; 4],
}

impl Effect {
    fn new(knots: &'static [&'static [f32]], direction: f32) -> Self {
        Effect {
            knots,
            direction,
            state: [0.0; 4],
        }
    }

    /// `update_knots`: read the values off the timeline at `eff_time`.
    ///
    /// Upstream walks every knot in order and lets each one that has already
    /// happened overwrite the state, so what survives is the last one before
    /// now, interpolated towards the one after it. Two knots at the same time
    /// give a zero interval, which it takes as fully arrived rather than as a
    /// division by nought, and that is how the film cuts.
    fn update(&mut self, eff_time: f32) {
        for (i, cur) in self.knots.iter().enumerate() {
            if cur[0] > eff_time {
                continue;
            }
            // The last knot repeats itself, to carry its values forward.
            let next = self.knots[(i + 1).min(self.knots.len() - 1)];
            let span = next[0] - cur[0];
            let t = if span <= 0.0 {
                1.0
            } else {
                ((eff_time - cur[0]) / span).min(1.0)
            };
            for j in 1..cur.len() {
                self.state[j - 1] = cur[j] + (next[j] - cur[j]) * t;
            }
        }
    }
}

/// `init_effects`: the film, as twelve tables of keyframes.
///
/// The comments are upstream's, and the numbers are the times things happen.
fn init_effects() -> Vec<Effect> {
    vec![
        /* effect 1: wall tunnel. percent closed */
        Effect::new(
            &[
                &[0.0, 0.055],
                &[2.77, 0.055],
                &[3.07, 1.0],
                &[8.08, 1.0],
                &[8.08, 0.0],
                &[10.0, 0.0],
            ],
            -0.2,
        ),
        /* effect 2: tardis. distance and alpha */
        Effect::new(
            &[
                &[0.0, 0.0, 0.0],
                &[3.44, 0.0, 0.0],
                &[3.36, 5.4, 0.0],
                &[4.24, 3.66, 1.0],
                &[6.51, 2.4, 0.94],
                &[8.08, 0.75, 0.0],
                &[8.08, 0.0, 0.0],
                &[10.0, 0.0, 0.0],
            ],
            1.0,
        ),
        /* effect 3: cylinder. alpha */
        Effect::new(
            &[
                &[0.0, 0.0],
                &[6.41, 0.00],
                &[8.08, 1.0],
                &[14.81, 1.0],
                &[15.65, 0.0],
            ],
            0.889,
        ),
        /* effect 4: fog. color, density, start, end */
        Effect::new(
            &[
                &[0.0, 1.0, 0.45, 3.0, 15.0],
                &[6.40, 1.0, 0.45, 3.0, 14.0],
                &[8.08, 1.0, 0.95, 1.0, 14.0],
                &[15.17, 1.0, 0.95, 1.0, 6.0],
                &[15.51, 1.0, 0.95, 3.0, 8.0],
                &[23.35, 1.0, 0.95, 3.0, 8.0],
                &[24.02, 0.0, 0.95, 2.3, 5.0],
                &[26.02, 0.0, 0.95, 2.3, 5.0],
                &[27.72, 0.0, 1.00, 0.3, 0.9],
            ],
            1.0,
        ),
        /* effect 5: logo. dist, alpha */
        Effect::new(
            &[
                &[0.0, 0.0, 0.0],
                &[16.52, 0.00, 0.0],
                &[16.52, 0.80, 0.01],
                &[17.18, 1.15, 1.0],
                &[22.36, 5.3, 1.0],
                &[22.69, 5.7, 0.0],
                &[22.69, 0.0, 0.0],
            ],
            1.0,
        ),
        /* effect 6: diamond tunnel. alpha */
        Effect::new(&[&[0.0, 0.00], &[15.17, 0.00], &[15.51, 1.0]], 0.24),
        /* effect 7: tardis cap draw. positive draws cap */
        Effect::new(&[&[0.0, -1.00], &[4.24, -1.00], &[4.24, 1.00]], 1.0),
        /* effect 8: star/asterisk: alpha */
        Effect::new(
            &[
                &[0.0, 0.00],
                &[10.77, 0.00],
                &[11.48, 1.00],
                &[15.35, 1.00],
                &[16.12, 0.00],
            ],
            1.0,
        ),
        /* effect 9: whohead 1 alpha */
        Effect::new(
            &[
                &[0.0, 0.00],
                &[13.35, 0.00],
                &[14.48, 1.00],
                &[15.17, 1.00],
                &[15.97, 0.00],
            ],
            1.0,
        ),
        /* effect 10: whohead-brite alpha */
        Effect::new(
            &[
                &[0.0, 0.00],
                &[11.34, 0.00],
                &[12.34, 0.20],
                &[13.35, 0.60],
                &[14.48, 0.00],
            ],
            1.0,
        ),
        /* effect 11: whohead-psy alpha */
        Effect::new(
            &[
                &[0.0, 0.00],
                &[14.87, 0.00],
                &[15.17, 1.00],
                &[15.91, 0.00],
                &[16.12, 0.00],
            ],
            1.0,
        ),
        /* effect 12: whohead-silhouette pos-z, alpha */
        Effect::new(
            &[
                &[0.0, 1.0, 0.00],
                &[15.07, 1.0, 0.00],
                &[15.07, 1.0, 1.00],
                &[16.01, 1.0, 1.00],
                &[16.78, 0.5, 1.00],
                &[16.78, 0.1, 0.00],
            ],
            1.0,
        ),
    ]
}

/* -------------------------------------------------------------------------
 * The texture matrix the runtime does not have
 * ---------------------------------------------------------------------- */

/// A 2x3 affine standing in for `glMatrixMode (GL_TEXTURE)`.
///
/// A texture coordinate goes through it on its way to `glTexCoord2f`, which is
/// where GL would have applied it. Only the two operations upstream uses are
/// here, and they compose on the right as GL's do.
#[derive(Clone, Copy)]
struct TexMat([f32; 6]);

impl TexMat {
    const IDENTITY: TexMat = TexMat([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// `m * other`, the way `glMultMatrix` puts it.
    fn mul(self, other: [f32; 6]) -> TexMat {
        let m = self.0;
        let o = other;
        TexMat([
            m[0] * o[0] + m[2] * o[1],
            m[1] * o[0] + m[3] * o[1],
            m[0] * o[2] + m[2] * o[3],
            m[1] * o[2] + m[3] * o[3],
            m[0] * o[4] + m[2] * o[5] + m[4],
            m[1] * o[4] + m[3] * o[5] + m[5],
        ])
    }

    /// `glTranslatef (x, y, 0)`.
    fn translate(self, x: f32, y: f32) -> TexMat {
        self.mul([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// `glRotatef (deg, 0, 0, 1)`.
    fn rotate(self, deg: f32) -> TexMat {
        let (s, c) = deg.to_radians().sin_cos();
        self.mul([c, s, -s, c, 0.0, 0.0])
    }

    fn apply(self, s: f32, t: f32) -> (f32, f32) {
        let m = self.0;
        (m[0] * s + m[2] * t + m[4], m[1] * s + m[3] * t + m[5])
    }
}

/* -------------------------------------------------------------------------
 * The textures
 * ---------------------------------------------------------------------- */

/// `wrapVal`: an index folded back into a range, so the blur below reads the
/// image as a torus.
fn wrap_val(val: i32, min: i32, max: i32) -> i32 {
    if val >= max {
        min + (val - max) % (max - min)
    } else if val < min {
        max - (min - val) % (max - min)
    } else {
        val
    }
}

/// `LoadTexture`, less the parts that read a file or rescale.
///
/// Upstream rescales anything that is not a power of two, which of these five
/// pictures is only the logo, and its own OpenGL ES build declines to load the
/// logo at all for exactly that reason. WebGL 2 repeats a texture of any size,
/// so it is used at the hundred and eighty pixels it was drawn at rather than
/// resampled up to two hundred and fifty six.
///
/// The two filters are what turn a picture into the shapes this saver blends.
/// `anegative` throws the colours away and keeps only the shape: what was
/// transparent becomes `bw_color`, what was opaque becomes the opposite of it,
/// and everything becomes opaque. `blur` then softens that, and it is
/// upstream's box blur exactly, quantisation and all: each pixel scatters
/// `floor(v / 25)` into each of its twenty-five neighbours, so what comes back
/// is the average rounded down by up to a twenty-fifth of full scale, which
/// darkens the result a little every pass.
fn load_texture(
    data: &[u8],
    blur: u32,
    bw_color: f32,
    anegative: bool,
    onealpha: bool,
) -> Option<(i32, i32, Vec<u8>)> {
    let (width, height, mut px) = png::decode_rgba(data)?;

    if anegative {
        for p in px.chunks_exact_mut(4) {
            let tmpa = if p[3] == 0 {
                p[3] = 0xff;
                (bw_color * 255.0) as u8
            } else {
                p[3] = if onealpha { 0xff } else { 0xff - p[3] };
                ((1.0 - bw_color) * 255.0) as u8
            };
            /* make texture uniform b/w color */
            p[0] = tmpa;
            p[1] = tmpa;
            p[2] = tmpa;
        }
    }

    if blur > 0 {
        if !anegative {
            /* anegative already b/w's the whole image */
            for p in px.chunks_exact_mut(4) {
                if p[3] == 0 {
                    p[0] = (255.0 * bw_color) as u8;
                    p[1] = (255.0 * bw_color) as u8;
                    p[2] = (255.0 * bw_color) as u8;
                }
            }
        }
        let boxsize = 2;
        let boxdiv = 1.0 / (boxsize as f32 * 2.0 + 1.0) / (boxsize as f32 * 2.0 + 1.0);
        let mut tmpbuf = vec![0u8; px.len()];
        for _ in 0..blur {
            tmpbuf.fill(0);
            for cchan in 0..4 {
                for iy in 0..height {
                    for ix in 0..width {
                        let dtaidx = ((width * iy + ix) * 4 + cchan) as usize;
                        let tmpfa = f32::from(px[dtaidx]) * boxdiv;
                        for by in -boxsize..=boxsize {
                            for bx in -boxsize..=boxsize {
                                let indx = wrap_val(ix + bx, 0, width);
                                let indy = wrap_val(iy + by, 0, height);
                                let tmpidx = ((width * indy + indx) * 4 + cchan) as usize;
                                tmpbuf[tmpidx] = tmpbuf[tmpidx].wrapping_add(tmpfa as u8);
                            }
                        }
                    }
                }
            }
            px.copy_from_slice(&tmpbuf);
        }
    }

    Some((width, height, px))
}

/* -------------------------------------------------------------------------
 * The saver
 * ---------------------------------------------------------------------- */

struct TunnelState {
    effects: Vec<Effect>,
    /// Where the three scrolling textures have got to. Not an effect.
    texshift: [f32; 3],
    effect_time: f32,
    start_time: f32,
    end_time: f32,
    /// When the last frame was, for the clock the film runs on.
    time_old: f64,

    textures: [Option<u32>; MAX_TEXTURE],
    do_texture: bool,
    drawlogo: bool,
    reverse: bool,
    do_fog: bool,
    dilate: f32,
    trackball: Trackball,
}

/// `glTexCoord2f`, through the texture matrix.
fn tex_coord(g: &mut Gl, m: TexMat, s: f32, t: f32) {
    let (s, t) = m.apply(s, t);
    g.glx.tex_coord2f(s, t);
}

impl TunnelState {
    fn effect(&self, n: usize) -> &Effect {
        &self.effects[n - 1]
    }

    fn bind(&self, g: &mut Gl, tex: usize) {
        if self.do_texture
            && let Some(id) = self.textures[tex]
        {
            g.glx.bind_texture(id);
        }
    }

    /// `update_animation`: move the clock on and read the timeline off it.
    fn update_animation(&mut self, g: &mut Gl) {
        let now = g.elapsed();
        /* elapsed time. computed timeshift is tenths of a second */
        let computed_timeshift = ((now - self.time_old) * 10.0) as f32;
        self.time_old = now;

        /* calibrate effect time to lie between start and end times */
        /* loop if time exceeds end time */
        if self.reverse {
            self.effect_time -= computed_timeshift / 10.0 * self.dilate;
        } else {
            self.effect_time += computed_timeshift / 10.0 * self.dilate;
        }
        if self.effect_time >= self.end_time {
            self.effect_time = self.start_time;
        }
        if self.effect_time < self.start_time {
            self.effect_time = self.end_time;
        }

        /* move texture shifters in effect's direction, e.g. tardis
        tunnel moves backward, effect 1's direction */
        let dirs = [
            self.effect(1).direction,
            self.effect(3).direction,
            self.effect(6).direction,
        ];
        for (shift, dir) in self.texshift.iter_mut().zip(dirs) {
            if self.reverse {
                *shift -= dir * computed_timeshift / 10.0;
            } else {
                *shift += dir * computed_timeshift / 10.0;
            }
            /* loop texture shifters if necessary */
            if *shift > 1.0 || *shift < -1.0 {
                *shift -= shift.trunc();
            }
        }

        let t = self.effect_time;
        for e in &mut self.effects {
            e.update(t);
        }
    }

    /// `draw_sign`: a textured quad at a depth, at a strength, in one of the
    /// three blending modes. Nothing happens at no strength.
    fn draw_sign(&self, g: &mut Gl, z: f32, alpha: f32, aspect: f32, tex: usize, blend_mode: i32) {
        if alpha <= 0.0 {
            return;
        }
        g.glx.blend(match blend_mode {
            1 => Blend::ConstantSubtract(alpha),
            2 => Blend::ConstantAdd(alpha),
            _ => Blend::ConstantFade(alpha),
        });
        self.bind(g, tex);
        g.glx.begin(Shape::Quads);
        for (s, t, x, y) in [
            (1.0, 0.0, -1.0, -1.0),
            (1.0, 1.0, -1.0, 1.0),
            (0.0, 1.0, 1.0, 1.0),
            (0.0, 0.0, 1.0, -1.0),
        ] {
            g.glx.tex_coord2f(s, t);
            g.glx.vertex3f(x, y * aspect, z);
        }
        g.glx.end();
        g.glx.blend(Blend::ConstantFade(alpha));
    }

    /// `draw_cyl`: one of the two tunnels, scrolled to wherever its texture
    /// shifter has got to. Upstream keeps the geometry in a display list and
    /// moves the texture matrix over it; here the matrix goes on as the
    /// coordinates are written, so the tunnel is rebuilt each frame. It is
    /// thirty quads.
    fn draw_cyl(&self, g: &mut Gl, alpha: f32, texnum: usize, diamond: bool, shiftnum: usize) {
        if alpha <= 0.0 {
            return;
        }
        let m = TexMat::IDENTITY.translate(self.texshift[shiftnum], 0.0);
        g.glx.blend(Blend::ConstantFade(alpha));
        self.bind(g, texnum);
        if diamond {
            makecyl(
                g,
                m,
                4,
                -0.5,
                DIAMOND_LEN,
                1.0,
                4.0 / 40.0 * DIAMOND_LEN,
                self.do_fog,
            );
        } else {
            makecyl(
                g,
                m,
                30,
                -0.1,
                CYL_LEN,
                1.0,
                10.0 / 40.0 * CYL_LEN,
                self.do_fog,
            );
        }
    }

    /// `make_wall_tunnel`: the tunnel that starts as four walls and opens out
    /// into the outline of a police box. `percent` is how far open it is, and
    /// `cap` closes the far end so the fog has something to sit on.
    fn make_wall_tunnel(&self, g: &mut Gl, percent: f32, cap: f32) {
        /* tardis is about 2x1, so wrap tex around, starting at the base*/
        /* tex coords are:

         _tl__tr_
         |      |
        l|      |r
         |      |
         -bl__br_
            that's br=bottom right, etc. ttr is top-top-right */
        let half_floor = 0.083_333_336;
        let full_wall = 0.333_333_34;
        /* zdepth is how far back tunnel goes */
        /* depth is tex coord scale.  low number = fast texture shifting */
        let depth = 0.3;
        let zdepth = 15.0;

        let br1 = half_floor;
        let r0 = br1;
        let r1 = r0 + full_wall;
        let tr0 = r1;
        let tr1 = r1 + half_floor;
        let tl0 = tr1;
        let tl1 = tl0 + half_floor;
        let l0 = tr1;
        let l1 = l0 + full_wall;

        let m = TexMat::IDENTITY
            .rotate(90.0)
            .translate(self.texshift[0], 0.0);

        self.bind(g, 0);
        g.glx.color3f(1.0, 1.0, 0.0);

        if cap > 0.0 && percent > 0.0 && self.drawlogo && self.do_fog {
            g.glx.begin(Shape::TriangleFan);
            for (x, y) in [
                (0.0, 0.0),
                (-1.0, -2.0),
                (1.0, -2.0),
                (1.0, 2.0),
                (0.2, 2.0),
                (0.2, 2.2),
                (-0.2, 2.2),
                (-0.2, 2.0),
                (-1.0, 2.0),
                (-1.0, -2.0),
            ] {
                g.glx.vertex3f(x, y, zdepth);
            }
            g.glx.end();
        }

        if percent > full_wall * 2.0 {
            g.glx.begin(Shape::Quads);
            let full = ((percent - full_wall * 2.0) / (1.0 - full_wall * 2.0)).min(1.0);
            if full > 0.8 {
                let mut height = full;
                if height > 0.90 {
                    /* TTTR */
                    let texbot = tr0;
                    let textop = tr0 + half_floor * height;
                    for (s, t, x, y, z) in [
                        (0.0, texbot, 0.2, 2.2, 0.0),
                        (0.0, textop, 2.0 - height * 2.0, 2.2, 0.0),
                        (depth, textop, 2.0 - height * 2.0, 2.2, zdepth),
                        (depth, texbot, 0.2, 2.2, zdepth),
                    ] {
                        tex_coord(g, m, s, t);
                        g.glx.vertex3f(x, y, z);
                    }
                    /* TTTL */
                    let texbot = tl1 - half_floor * height;
                    let textop = tl1;
                    for (s, t, x, y, z) in [
                        (0.0, texbot, -2.0 + height * 2.0, 2.2, 0.0),
                        (0.0, textop, -0.2, 2.2, 0.0),
                        (depth, textop, -0.2, 2.2, zdepth),
                        (depth, texbot, -2.0 + height * 2.0, 2.2, zdepth),
                    ] {
                        tex_coord(g, m, s, t);
                        g.glx.vertex3f(x, y, z);
                    }
                }
                if height > 0.90 {
                    height = 0.90;
                }
                /* TTR */
                let texbot = tr0;
                let textop = tr0 + half_floor * height;
                for (s, t, x, y, z) in [
                    (0.0, texbot, 0.2, 2.0, 0.0),
                    (0.0, textop, 0.2, 0.4 + height * 2.0, 0.0),
                    (depth, textop, 0.2, 0.4 + height * 2.0, zdepth),
                    (depth, texbot, 0.2, 2.0, zdepth),
                ] {
                    tex_coord(g, m, s, t);
                    g.glx.vertex3f(x, y, z);
                }
                /* TTL */
                let texbot = tl1 - half_floor * height;
                let textop = tl1;
                for (s, t, x, y, z) in [
                    (0.0, texbot, -0.2, 0.4 + height * 2.0, 0.0),
                    (0.0, textop, -0.2, 2.0, 0.0),
                    (depth, textop, -0.2, 2.0, zdepth),
                    (depth, texbot, -0.2, 0.4 + height * 2.0, zdepth),
                ] {
                    tex_coord(g, m, s, t);
                    g.glx.vertex3f(x, y, z);
                }
            }

            let height = full.min(0.8);
            /* TR */
            let texbot = tr0;
            let textop = tr0 + half_floor * height;
            for (s, t, x, y, z) in [
                (0.0, texbot, 1.0, 2.0, 0.0),
                (0.0, textop, 1.0 - height, 2.0, 0.0),
                (depth, textop, 1.0 - height, 2.0, zdepth),
                (depth, texbot, 1.0, 2.0, zdepth),
            ] {
                tex_coord(g, m, s, t);
                g.glx.vertex3f(x, y, z);
            }
            /* TL */
            let texbot = tl1 - half_floor * height;
            let textop = tl1;
            for (s, t, x, y, z) in [
                (0.0, texbot, -1.0 + height, 2.0, 0.0),
                (0.0, textop, -1.0, 2.0, 0.0),
                (depth, textop, -1.0, 2.0, zdepth),
                (depth, texbot, -1.0 + height, 2.0, zdepth),
            ] {
                tex_coord(g, m, s, t);
                g.glx.vertex3f(x, y, z);
            }

            let height = full;
            /* BR */
            let texbot = tr0;
            let textop = tr0 + half_floor * height;
            for (s, t, x, y, z) in [
                (0.0, texbot, 1.0, -2.0, 0.0),
                (0.0, textop, 1.0 - height, -2.0, 0.0),
                (depth, textop, 1.0 - height, -2.0, zdepth),
                (depth, texbot, 1.0, -2.0, zdepth),
            ] {
                tex_coord(g, m, s, t);
                g.glx.vertex3f(x, y, z);
            }
            /* BL */
            let texbot = tl1 - half_floor * height;
            let textop = tl1;
            for (s, t, x, y, z) in [
                (0.0, texbot, -1.0 + height, -2.0, 0.0),
                (0.0, textop, -1.0, -2.0, 0.0),
                (depth, textop, -1.0, -2.0, zdepth),
                (depth, texbot, -1.0 + height, -2.0, zdepth),
            ] {
                tex_coord(g, m, s, t);
                g.glx.vertex3f(x, y, z);
            }
            g.glx.end();
        }

        if percent > 0.0 {
            g.glx.begin(Shape::Quads);
            let height = (percent / (full_wall * 2.0)).min(1.0);
            let textop = (l0 + l1) / 2.0 - full_wall * 0.5 * height;
            let texbot = (l0 + l1) / 2.0 + full_wall * 0.5 * height;
            for (s, t, x, y, z) in [
                (0.0, textop, -1.0, height * 2.0, 0.0),
                (0.0, texbot, -1.0, -height * 2.0, 0.0),
                (depth, texbot, -1.0, -height * 2.0, zdepth),
                (depth, textop, -1.0, height * 2.0, zdepth),
            ] {
                tex_coord(g, m, s, t);
                g.glx.vertex3f(x, y, z);
            }
            let textop = (r0 + r1) / 2.0 - full_wall * 0.5 * height;
            let texbot = (r0 + r1) / 2.0 + full_wall * 0.5 * height;
            for (s, t, x, y, z) in [
                (0.0, texbot, 1.0, height * 2.0, 0.0),
                (0.0, textop, 1.0, -height * 2.0, 0.0),
                (depth, textop, 1.0, -height * 2.0, zdepth),
                (depth, texbot, 1.0, height * 2.0, zdepth),
            ] {
                tex_coord(g, m, s, t);
                g.glx.vertex3f(x, y, z);
            }
            g.glx.end();
        }
    }
}

/// `makecyl`: the tunnel itself. `stretch` scales the texture coordinates, so
/// a longer tunnel scrolls slower.
fn makecyl(
    g: &mut Gl,
    m: TexMat,
    sides: i32,
    zmin: f32,
    zmax: f32,
    rad: f32,
    stretch: f32,
    fog: bool,
) {
    /* cap */
    if fog {
        g.glx.begin(Shape::TriangleFan);
        tex_coord(g, m, 1.0, 0.0);
        g.glx.vertex3f(0.0, 0.0, zmax);
        for i in 0..=sides {
            let theta = std::f32::consts::TAU * (i as f32 / sides as f32);
            g.glx.vertex3f(theta.cos() * rad, theta.sin() * rad, zmax);
        }
        g.glx.vertex3f(rad, 0.0, zmax);
        g.glx.end();
    }
    g.glx.begin(Shape::QuadStrip);
    for i in 0..=sides {
        // The last ring repeats the first, so the seam meets rather than
        // wrapping the texture the whole way round backwards.
        let (theta, t) = if i != sides {
            (
                std::f32::consts::TAU * (i as f32 / sides as f32),
                i as f32 / sides as f32,
            )
        } else {
            (0.0, 1.0)
        };
        tex_coord(g, m, 0.0, t);
        g.glx.vertex3f(theta.cos() * rad, theta.sin() * rad, zmin);
        tex_coord(g, m, stretch, t);
        g.glx.vertex3f(theta.cos() * rad, theta.sin() * rad, zmax);
    }
    g.glx.end();
}

impl Hack3d for TunnelState {
    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = f64::from(height) / f64::from(width.max(1));
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = f64::from(height) / f64::from(width);
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(90.0, (1.0 / h) as f32, 0.2, 50.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 0.3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        screenhack_event_helper(event)
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 0.3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);

        self.update_animation(g);

        g.glx.push_matrix();
        // The trackball turns the tunnel, and the two half turns around it put
        // its axis back down the way the camera is looking.
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);

        if self.do_fog {
            let f = self.effect(4).state;
            g.glx.fog(Some(Fog::Linear {
                start: f[2],
                end: f[3],
                color: [f[0], f[0], f[0], 1.0],
            }));
        }

        /* --- begin composite image assembly --- */
        /* head mask and draw diamond tunnel */
        self.draw_cyl(g, self.effect(6).state[0], 5, true, 2);
        if self.drawlogo {
            let e = self.effect(12).state;
            self.draw_sign(g, e[0], e[1], 1.0 / 1.33, 9, 1);
        }
        g.glx.blend(Blend::Off);

        /* then tardis tunnel */
        self.make_wall_tunnel(g, self.effect(1).state[0], self.effect(7).state[0]);

        /* then cylinder tunnel */
        self.draw_cyl(g, self.effect(3).state[0], 2, false, 1);
        /* tardis */
        if self.drawlogo {
            let e = self.effect(2).state;
            self.draw_sign(g, e[0], e[1], 2.0, 1, 0);
        }
        /* marquee */
        if self.drawlogo {
            let e = self.effect(5).state;
            self.draw_sign(g, e[0], e[1], 1.0, 3, 0);
        }
        /* who head brite */
        if self.drawlogo {
            self.draw_sign(g, 1.0, self.effect(10).state[0], 1.0 / 1.33, 6, 2);
        }
        /* star */
        let star = self.effect(8).state[0];
        self.draw_sign(g, star, star, 1.0, 4, 1);
        /* normal head */
        if self.drawlogo {
            self.draw_sign(g, 1.0, self.effect(9).state[0], 1.0 / 1.33, 6, 0);
        }
        /* --- end composite image assembly --- */

        g.glx.pop_matrix();
        g.glx.blend(Blend::Off);
        g.glx.color3f(1.0, 1.0, 1.0);

        g.res.int("delay").max(0) as u32
    }
}

/// Load one picture into a texture name, if it decodes.
fn texture(
    g: &mut Gl,
    data: &[u8],
    blur: u32,
    bw: f32,
    anegative: bool,
    onealpha: bool,
) -> Option<u32> {
    let (w, h, px) = load_texture(data, blur, bw, anegative, onealpha)?;
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    g.glx.tex_image_2d(w, h, px);
    g.glx.tex_clamp(false);
    g.glx.tex_nearest(false);
    Some(id)
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let effect_time = 0.0;
    let mut start = g.res.float("start") as f32;
    let mut end = g.res.float("end") as f32;
    /* check bounds on cmd line opts */
    if start > EFFECT_MAXSECS {
        start = EFFECT_MAXSECS;
    }
    if end > EFFECT_MAXSECS {
        end = EFFECT_MAXSECS;
    }
    if start < effect_time {
        start = effect_time;
    }
    if end < effect_time {
        end = effect_time;
    }

    let do_texture = g.res.bool("texture");
    let mut textures = [None; MAX_TEXTURE];
    if do_texture {
        /* the following textures are loaded, and possible overridden:
        tunnel 1, tunnel 2, tunnel 3, marquee, tardis, head */
        textures[0] = texture(g, crate::images::TIMETUNNEL0, 0, 0.0, false, false);
        textures[2] = texture(g, crate::images::TIMETUNNEL1, 0, 0.0, false, false);
        textures[5] = texture(g, crate::images::TIMETUNNEL2, 0, 0.0, false, false);
        textures[4] = texture(g, crate::images::TUNNELSTAR, 0, 0.0, false, false);
        textures[3] = texture(g, crate::images::LOGO_180, 0, 0.0, false, false);
        textures[1] = texture(g, crate::images::LOGO_180, 0, 0.0, false, false);
        textures[6] = texture(g, crate::images::LOGO_180, 0, 0.0, false, false);
        /* negative */
        textures[9] = texture(g, crate::images::LOGO_180, 2, 1.0, true, true);
        g.glx.texturing(true);
        // `setTexParams`: the texture is the colour, and the yellow the wall
        // tunnel is drawn in never reaches the screen.
        g.glx.tex_env(TexEnv::Replace);
    }

    g.glx.depth_test(false); /* who needs it? ;-) */
    g.glx.alpha_test(Some(0.5));
    g.glx.blend(Blend::Off);

    let mut st = TunnelState {
        effects: init_effects(),
        texshift: [0.0; 3],
        effect_time,
        start_time: start,
        end_time: end,
        time_old: g.elapsed(),
        textures,
        do_texture,
        drawlogo: g.res.bool("drawlogo"),
        reverse: g.res.bool("reverse"),
        do_fog: g.res.bool("fog"),
        dilate: g.res.float("dilate") as f32,
        trackball: Trackball::new(),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     30000",
    "*count:     30",
    "*showFPS:   False",
    "*timeStart: 0.0",
    "*timeEnd:   27.79",
    "*wireframe: False",
    "*start:     0.00",
    "*end:       27.79",
    "*dilate:    1.00",
    "*drawlogo:  True",
    "*reverse:   False",
    "*fog:       True",
    "*texture:   True",
];

const OPTS: &[Opt] = &[
    Opt::slider("start", "Start sequence time", 0.0, 27.79, 0.01, 2, "0.00"),
    Opt::slider("end", "End sequence time", 0.0, 27.79, 0.01, 2, "27.79"),
    Opt::boolean("drawlogo", "Draw logo", "true"),
    Opt::boolean("reverse", "Run backward", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "timetunnel",
    label: "Time Tunnel",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Sean P. Brennan",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=GZe5rk_7TnA"),
        blurb: "An animation similar to the title sequence of Dr. Who in the 70s.",
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

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// Run the film for `secs` of its own clock. A step is the saver's delay,
    /// thirty milliseconds.
    fn play(r: &mut Runner3d, secs: f64) {
        for _ in 0..(secs / 0.03) as usize {
            r.step();
        }
    }

    /// A keyframe table is read by linear interpolation between the knot
    /// before now and the one after, and holds its last value afterwards.
    #[test]
    fn a_timeline_interpolates_between_its_knots() {
        let mut e = Effect::new(&[&[0.0, 0.0], &[2.0, 10.0], &[4.0, 10.0]], 1.0);
        e.update(0.0);
        assert_eq!(e.state[0], 0.0);
        e.update(1.0);
        assert_eq!(e.state[0], 5.0);
        e.update(2.0);
        assert_eq!(e.state[0], 10.0);
        // Past the end, the last knot repeats itself.
        e.update(9.0);
        assert_eq!(e.state[0], 10.0);
    }

    /// Two knots at the same time are a cut, not a division by nought: the
    /// second one arrives whole. That is how the wall tunnel vanishes at 8.08
    /// seconds after being fully open at 8.08 seconds.
    #[test]
    fn two_knots_at_one_time_are_a_cut() {
        let e = &init_effects()[0];
        let mut e = Effect::new(e.knots, e.direction);
        e.update(8.07);
        assert!(e.state[0] > 0.9, "the wall tunnel closed early");
        e.update(8.09);
        assert!(e.state[0] < 0.1, "the wall tunnel did not cut away");
    }

    /// The film runs on a clock, so the picture at four seconds in is the same
    /// however many frames it took to get there.
    #[test]
    fn the_film_runs_on_a_clock_not_on_frames() {
        let mut a = start(StartArgs::new(640, 480, "", 20260812));
        for i in 1..=100 {
            a.tick(f64::from(i) * 0.04);
        }
        let mut b = start(StartArgs::new(640, 480, "", 20260812));
        for i in 1..=20 {
            b.tick(f64::from(i) * 0.2);
        }
        let (fa, fb) = (a.frame().vertices.len(), b.frame().vertices.len());
        assert_eq!(
            fa, fb,
            "four seconds took a different shape at a different frame rate"
        );
    }

    /// The whole sequence draws without falling over, and the picture keeps
    /// changing all the way through it.
    #[test]
    fn the_sequence_plays_through() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        let mut counts = std::collections::BTreeSet::new();
        // The whole twenty-eight seconds of it.
        for _ in 0..950 {
            r.step();
            counts.insert(r.frame().vertices.len());
        }
        assert!(
            counts.len() > 8,
            "the film only ever had {} shapes in it",
            counts.len()
        );
    }

    /// Every stage of the film is composited with a blend constant, and one of
    /// them subtracts. Without the blend equation the star would add to the
    /// tunnel instead of punching through it.
    #[test]
    fn the_star_subtracts_itself_out_of_the_tunnel() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        let mut subtracted = 0;
        let mut faded = 0;
        for _ in 0..950 {
            r.step();
            for b in &r.frame().batches {
                match b.blend {
                    Blend::ConstantSubtract(a) => {
                        assert!(a > 0.0);
                        subtracted += 1;
                    }
                    Blend::ConstantFade(_) | Blend::ConstantAdd(_) => faded += 1,
                    _ => {}
                }
            }
        }
        assert!(subtracted > 0, "nothing ever subtracted");
        assert!(faded > 0, "nothing ever faded");
    }

    /// The tunnels scroll by moving their texture coordinates, which is the
    /// texture matrix the runtime does not have, carried by hand.
    #[test]
    fn the_tunnel_scrolls_its_texture() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        // Nine seconds in, the cylinder tunnel is up.
        play(&mut r, 9.0);
        let first: Vec<[f32; 2]> = r.frame().vertices.iter().map(|v| v.uv).collect();
        play(&mut r, 0.5);
        let then: Vec<[f32; 2]> = r.frame().vertices.iter().map(|v| v.uv).collect();
        assert!(!first.is_empty());
        assert_ne!(first, then, "the tunnel walls stood still");
    }

    /// The texture matrix composes the way GL's does: a rotation then a
    /// translation moves the coordinate first and turns the result.
    #[test]
    fn the_texture_matrix_composes_like_gls() {
        let m = TexMat::IDENTITY.rotate(90.0).translate(0.25, 0.0);
        let (s, t) = m.apply(0.0, 0.0);
        assert!((s - 0.0).abs() < 1e-6, "s was {s}");
        assert!((t - 0.25).abs() < 1e-6, "t was {t}");
        // And the other order is not the same.
        let n = TexMat::IDENTITY.translate(0.25, 0.0).rotate(90.0);
        let (s2, t2) = n.apply(0.0, 0.0);
        assert!((s2 - 0.25).abs() < 1e-6, "s2 was {s2}");
        assert!((t2 - 0.0).abs() < 1e-6, "t2 was {t2}");
    }

    /// The negative of the logo is a silhouette: what was transparent comes
    /// out white and what was drawn comes out black, and the blur then softens
    /// the edge between them.
    #[test]
    fn the_negative_logo_is_a_soft_silhouette() {
        let (w, h, plain) = load_texture(crate::images::LOGO_180, 0, 0.0, false, false).unwrap();
        let (w2, h2, neg) = load_texture(crate::images::LOGO_180, 2, 1.0, true, true).unwrap();
        assert_eq!((w, h), (w2, h2));
        assert_eq!(
            (w, h),
            (180, 180),
            "the logo is not a power of two, and is used as it is"
        );

        // Every pixel of the negative is opaque and grey.
        for p in neg.chunks_exact(4) {
            assert_eq!(p[0], p[1]);
            assert_eq!(p[1], p[2]);
        }
        // The corner of the logo is transparent, so the negative is bright
        // there; the middle is drawn, so it is dark.
        let at = |px: &[u8], x: usize, y: usize| px[(y * w as usize + x) * 4];
        assert!(at(&plain, 2, 2) < 250 || plain[(2 * w as usize + 2) * 4 + 3] == 0);
        assert!(
            at(&neg, 2, 2) > at(&neg, 90, 90),
            "the silhouette came out inside out"
        );
    }

    /// Turning the logo off leaves the tunnels and takes away the signs.
    #[test]
    fn the_logo_can_be_turned_off() {
        let with = run("", 400).frame().vertices.len();
        let without = run("drawlogo=false", 400).frame().vertices.len();
        assert!(without <= with);
    }
}
