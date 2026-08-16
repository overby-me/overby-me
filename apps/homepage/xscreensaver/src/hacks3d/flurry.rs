//! Port of `hacks/glx/flurry.c` and its `flurry-smoke.c`, `flurry-spark.c`,
//! `flurry-star.c` and `flurry-texture.c`.
//!
//! ```text
//! Copyright (c) 2002, Calum Robinson
//! All rights reserved.
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions are met:
//!
//! * Redistributions of source code must retain the above copyright notice,
//!   this list of conditions and the following disclaimer.
//!
//! * Redistributions in binary form must reproduce the above copyright notice,
//!   this list of conditions and the following disclaimer in the documentation
//!   and/or other materials provided with the distribution.
//!
//! * Neither the name of the author nor the names of its contributors may be
//!   used to endorse or promote products derived from this software without
//!   specific prior written permission.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
//! AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
//! IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
//! ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
//! LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
//! CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
//! SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
//! INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
//! CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
//! ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
//! POSSIBILITY OF SUCH DAMAGE.
//! ```
//!
//! The Mac OS X screen saver, the one that looks like a star being torn apart
//! into coloured smoke. It is a particle system with three kinds of thing in
//! it, only one of which you can see.
//!
//! The *star* is a point wandering on a Lissajous figure, put through three
//! rotations whose speeds are irrational multiples of each other so it never
//! repeats. It is the source: one puff of smoke per stream leaves it every
//! hundred and twenty-first of a second.
//!
//! The *sparks* are the attractors, and they are not drawn at all: upstream's
//! code to draw them is behind an `#ifdef` that is never set. Each one moves
//! on its own version of the same Lissajous, and every smoke particle is
//! pulled towards every spark with an inverse-square force, with one extra
//! helping for the spark whose number matches the particle's own, which is
//! what keeps the streams separate instead of letting them average into a
//! blob.
//!
//! The *smoke* is what you see. Each puff is a screen-space quad stretched
//! along the direction it moved this frame, so a fast particle draws a streak
//! and a slow one a dot, and it fades as it widens until it is wider than the
//! stream and dies. The texture is an eight by eight sheet of soft blobs
//! generated at startup, each one a cosine falloff with noise speckled in and
//! smoothed twice, and every particle picks a cell and walks through the
//! sheet a frame at a time.
//!
//! Nothing is ever cleared. Each frame lays a black rectangle over the screen
//! at an alpha proportional to how long the frame took, so the trails fade at
//! a rate that does not depend on the frame rate, and the smoke is added over
//! the top. A colour buffer is not guaranteed to survive to the next frame
//! here, so the screen is kept in a texture and put back at the start of one,
//! the same way `noof` does it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, SelectItem, StartArgs, XEvent, frand, random,
    random_below,
};

const NUMSMOKEPARTICLES: usize = 3600;
const MAX_SPARKS: usize = 64;
const MAXANGLES: f64 = 16384.0;
const BIGMYSTERY: f64 = 1800.0;

const GRAVITY: f32 = 1500000.0;
const INCOHESION: f32 = 0.07;
const COLOR_INCOHERENCE: f32 = 0.15;
const STREAM_SPEED: f32 = 450.0;
/// Zero upstream, but the calls that use it still draw three random numbers,
/// so they are still made.
const FIELD_COHERENCE: f32 = 0.0;
const FIELD_SPEED: f64 = 12.0;
const SERAPH_DISTANCE: f64 = 2000.0;
const STREAM_SIZE: f32 = 25000.0;
const FIELD_RANGE: f32 = 1000.0;
const STREAM_BIAS: f32 = 7.0;

/// `RandFlt`.
fn rand_flt(min: f64, max: f64) -> f64 {
    min + frand(max - min)
}

/// `RandBell`: three uniforms added together, which is a rough bell curve,
/// and always negative.
fn rand_bell(scale: f32) -> f32 {
    scale * -((frand(0.5) + frand(0.5) + frand(0.5)) as f32)
}

/// This function computes the distance from 0,0 to x,y with ~3.5% error.
fn fast_distance_2d(x: f32, y: f32) -> f32 {
    /* first compute the absolute value of x,y */
    let x = x.abs();
    let y = y.abs();
    /* compute the minimum of x,y */
    let mn = x.min(y);
    /* return the distance */
    x + y - (mn * 0.5) - (mn * 0.25) + (mn * 0.0625)
}

/// Upstream has thirteen of these and its presets pick six, so the other
/// seven are not here: cyan, magenta and yellow, which are fixed points on
/// the wheel like red and blue and green; `cyclic`, which is the twenty-second
/// cycle; and white, multi and dark, which stand still at a grey. The numbers
/// are upstream's own, because a fixed mode's colour *is* its number: it names
/// the sixth of the cycle it stands at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ColorMode {
    Red = 0,
    Blue = 2,
    Green = 4,
    SlowCyclic = 6,
    Tiedye = 8,
    Rainbow = 9,
}

impl ColorMode {
    /// How long the colour takes to go round once, and the colour it cycles
    /// about. The three fixed modes stand still; the rest walk the wheel.
    fn cycle_time(self) -> f32 {
        match self {
            ColorMode::Rainbow => 1.5,
            ColorMode::Tiedye => 4.5,
            ColorMode::SlowCyclic => 120.0,
            _ => 20.0,
        }
    }

    /// The colour the flurry cycles about: a cosine of the time in each
    /// channel, a third of a cycle apart. A mode below `SlowCyclic` stands
    /// still at its own sixth of the wheel instead of walking it.
    fn base(self, f_time: f64, random_seed: f64) -> [f32; 3] {
        let cycle_time = self.cycle_time();
        let color_rot = (2.0 * std::f64::consts::PI / cycle_time as f64) as f32;
        let phase = [0.0, cycle_time / 3.0, cycle_time * 2.0 / 3.0];
        let n = self as i32;
        let color_time = if n < ColorMode::SlowCyclic as i32 {
            (n as f32 / 6.0) * cycle_time
        } else {
            (f_time + random_seed) as f32
        };
        [0, 1, 2].map(|i| 0.109375 * (((color_time + phase[i]) * color_rot).cos() + 1.0))
    }
}

/// The point the smoke comes off.
#[derive(Clone, Copy)]
struct Star {
    position: [f32; 3],
    mystery: f64,
    rot_speed: f64,
}

impl Star {
    fn new() -> Star {
        Star {
            position: [0, 1, 2].map(|_| rand_flt(-10000.0, 10000.0) as f32),
            rot_speed: rand_flt(0.4, 0.9),
            mystery: rand_flt(0.0, 10.0),
        }
    }

    fn update(&mut self, f_time: f64) {
        /* speed control */
        let rotations_per_second = 2.0 * std::f64::consts::PI * 12.0 / MAXANGLES * self.rot_speed;
        let this_angle = f_time * rotations_per_second;

        let t = f_time * rotations_per_second;
        let mut cf = (7.0 * t).cos() + (3.0 * t).cos() + (13.0 * t).cos();
        cf /= 6.0;
        cf += 0.75;
        let this_point_in_radians = 2.0 * std::f64::consts::PI * self.mystery / BIGMYSTERY;

        let p = lissajous(250.0, cf, this_point_in_radians, this_angle);
        self.position = tumble(p, this_point_in_radians, this_angle).map(|v| v as f32);
    }
}

/// The figure the star and the sparks both walk, at their own radius.
fn lissajous(scale: f64, cf: f64, radians: f64, angle: f64) -> [f64; 3] {
    [
        scale * cf * (11.0 * (radians + (3.0 * angle))).cos(),
        scale * cf * (12.0 * (radians + (4.0 * angle))).sin(),
        scale * (23.0 * (radians + (12.0 * angle))).cos(),
    ]
}

/// Four rotations about three axes at speeds that are not multiples of each
/// other, which is what stops the path ever coming back to itself.
fn tumble(p: [f64; 3], radians: f64, angle: f64) -> [f64; 3] {
    let rotation = angle * 0.501 + 5.01 * radians / (2.0 * std::f64::consts::PI);
    let (cr, sr) = (rotation.cos(), rotation.sin());

    let (x1, y1, z1) = (p[0] * cr - p[1] * sr, p[1] * cr + p[0] * sr, p[2]);
    let (x2, y2, z2) = (x1 * cr - z1 * sr, y1, z1 * cr + x1 * sr);
    let (x3, y3, z3) = (x2, y2 * cr - z2 * sr, z2 * cr + y2 * sr + SERAPH_DISTANCE);

    let rotation = angle * 2.501 + 85.01 * radians / (2.0 * std::f64::consts::PI);
    let (cr, sr) = (rotation.cos(), rotation.sin());
    [x3 * cr - y3 * sr, y3 * cr + x3 * sr, z3]
}

/// One of the attractors. Never drawn: upstream's `DrawSpark` is behind an
/// `#ifdef DRAW_SPARKS` that nothing defines.
#[derive(Clone, Copy)]
struct Spark {
    position: [f32; 3],
    mystery: f64,
    delta: [f32; 3],
    color: [f32; 4],
}

impl Spark {
    fn new() -> Spark {
        Spark {
            position: [0, 1, 2].map(|_| rand_flt(-100.0, 100.0) as f32),
            mystery: 0.0,
            delta: [0.0; 3],
            color: [0.0; 4],
        }
    }

    fn update(&mut self, mode: ColorMode, f_time: f64, f_delta_time: f64, random_seed: f64) {
        let rotations_per_second = 2.0 * std::f64::consts::PI * FIELD_SPEED / MAXANGLES;
        let this_angle = f_time * rotations_per_second;

        let base = mode.base(f_time, random_seed);

        let old = self.position;

        let t = f_time * rotations_per_second;
        let mut cf = (7.0 * t).cos() + (3.0 * t).cos() + (13.0 * t).cos();
        cf /= 6.0;
        cf += 2.0;
        let radians = 2.0 * std::f64::consts::PI * self.mystery / BIGMYSTERY;

        self.color[0] = base[0]
            + 0.0625
                * (0.5
                    + (15.0 * (radians + 3.0 * this_angle)).cos() as f32
                    + (7.0 * (radians + this_angle)).sin() as f32);
        self.color[1] = base[1] + 0.0625 * (0.5 + (radians + this_angle).sin() as f32);
        self.color[2] = base[2] + 0.0625 * (0.5 + (37.0 * (radians + this_angle)).cos() as f32);

        let p = lissajous(FIELD_RANGE as f64, cf, radians, this_angle);
        let p = tumble(p, radians, this_angle);
        // `fieldCoherence` is zero, so this adds nothing; the numbers are
        // still drawn, because leaving them out would shift everything that
        // comes after off the same seed.
        self.position = [0, 1, 2].map(|i| p[i] as f32 + rand_bell(5.0 * FIELD_COHERENCE));

        self.delta = [0, 1, 2].map(|i| (self.position[i] - old[i]) / f_delta_time as f32);
    }
}

/// Every puff of smoke there is, in parallel arrays. Upstream keeps four of
/// them side by side in each struct so that the AltiVec build can do four at
/// once; nothing here does, so this is flat and a particle's number is its
/// own index.
struct Smoke {
    color: Vec<[f32; 4]>,
    position: Vec<[f32; 3]>,
    oldposition: Vec<[f32; 3]>,
    delta: Vec<[f32; 3]>,
    /// 0 alive, 1 dead. Upstream also has a 3, but only its vector code ever
    /// writes one.
    dead: Vec<u8>,
    time: Vec<f32>,
    anim_frame: Vec<i32>,

    next_particle: usize,
    last_particle_time: f32,
    first_time: bool,
    old: [f32; 3],
}

impl Smoke {
    fn new() -> Smoke {
        Smoke {
            color: vec![[0.0; 4]; NUMSMOKEPARTICLES],
            position: vec![[0.0; 3]; NUMSMOKEPARTICLES],
            oldposition: vec![[0.0; 3]; NUMSMOKEPARTICLES],
            delta: vec![[0.0; 3]; NUMSMOKEPARTICLES],
            dead: vec![1; NUMSMOKEPARTICLES],
            time: vec![0.0; NUMSMOKEPARTICLES],
            anim_frame: vec![0; NUMSMOKEPARTICLES],
            next_particle: 0,
            last_particle_time: 0.25,
            first_time: true,
            old: [0, 1, 2].map(|_| rand_flt(-100.0, 100.0) as f32),
        }
    }
}

/// One flurry: a star, its sparks, and the smoke between them. A preset is
/// one or more of these on top of each other.
struct Stream {
    color_mode: ColorMode,
    smoke: Smoke,
    star: Star,
    spark: Vec<Spark>,
    stream_expansion: f32,
    num_streams: usize,
    random_seed: f64,
    f_time: f64,
    f_old_time: f64,
    f_delta_time: f64,
    brite_factor: f64,
    drag: f32,
    dframe: i32,
}

impl Stream {
    fn new(
        now: f64,
        streams: usize,
        colour: ColorMode,
        thickness: f32,
        speed: f64,
        bf: f64,
    ) -> Stream {
        let random_seed = rand_flt(0.0, 300.0);
        let mut st = Stream {
            color_mode: colour,
            smoke: Smoke::new(),
            star: Star::new(),
            spark: Vec::with_capacity(MAX_SPARKS),
            stream_expansion: thickness,
            num_streams: streams,
            random_seed,
            f_old_time: 0.0,
            f_time: now + random_seed,
            f_delta_time: now + random_seed,
            brite_factor: bf,
            drag: 1.0,
            dframe: 0,
        };
        st.star.rot_speed = speed;

        for i in 0..MAX_SPARKS {
            let mut s = Spark::new();
            /* 100 * (i + 1) / (flurry->numStreams + 1); */
            // Integer division upstream, so the sparks land on a coarse
            // set of phases rather than an even spread.
            s.mystery = (1800 * (i as i32 + 1) / 13) as f64;
            st.spark.push(s);
            let (mode, t, dt, seed) = (st.color_mode, st.f_time, st.f_delta_time, st.random_seed);
            st.spark[i].update(mode, t, dt, seed);
        }
        st
    }

    /// `UpdateSmoke_ScalarBase`: let twelve more puffs go, then pull every
    /// live one towards every spark and let the drag slow it down.
    fn update_smoke(&mut self) {
        let [sx, sy, sz] = self.star.position;

        if !self.smoke.first_time {
            /* release 12 puffs every frame */
            if self.f_time as f32 - self.smoke.last_particle_time >= 1.0 / 121.0 {
                let dx = self.smoke.old[0] - sx;
                let dy = self.smoke.old[1] - sy;
                let dz = self.smoke.old[2] - sz;
                let mag = 5.0;
                let (deltax, deltay, deltaz) = (dx * mag, dy * mag, dz * mag);

                for i in 0..self.num_streams {
                    let n = self.smoke.next_particle;
                    self.smoke.delta[n] = [deltax, deltay, deltaz];
                    self.smoke.position[n] = [sx, sy, sz];
                    self.smoke.oldposition[n] = [sx, sy, sz];

                    let coherence = (1.0 + rand_bell(0.25 * INCOHESION)).max(0.0);
                    let spark = self.spark[i % MAX_SPARKS];
                    let dx = sx - spark.position[0];
                    let dy = sy - spark.position[1];
                    let dz = sz - spark.position[2];
                    let rsquared = dx * dx + dy * dy + dz * dz;
                    let f = STREAM_SPEED * coherence;
                    let mag = f / rsquared.sqrt();

                    self.smoke.delta[n][0] -= dx * mag;
                    self.smoke.delta[n][1] -= dy * mag;
                    self.smoke.delta[n][2] -= dz * mag;
                    for c in 0..3 {
                        self.smoke.color[n][c] =
                            spark.color[c] * (1.0 + rand_bell(COLOR_INCOHERENCE));
                    }
                    self.smoke.color[n][3] = 0.85 * (1.0 + rand_bell(0.5 * COLOR_INCOHERENCE));
                    self.smoke.time[n] = self.f_time as f32;
                    self.smoke.dead[n] = 0;
                    self.smoke.anim_frame[n] = (random() & 63) as i32;

                    self.smoke.next_particle = (n + 1) % NUMSMOKEPARTICLES;
                }

                self.smoke.last_particle_time = self.f_time as f32;
            }
        } else {
            self.smoke.last_particle_time = self.f_time as f32;
            self.smoke.first_time = false;
        }

        self.smoke.old = self.star.position;

        // How hard everything pulls is normalised against a reference rate
        // of 42.5 frames a second, so that the smoke moves the same distance
        // however fast the frames come.
        //
        // Upstream divides the frame count by `fTime`, which is the time
        // since it started *plus* a random offset of up to five minutes, so
        // for the first few minutes of a run the number it calls the frame
        // rate is several times too low and everything pulls several times
        // too hard: the smoke is flung past its own speed limit and dies
        // young, and the flurry is a handful of streaks rather than a bloom.
        // It comes right once the run is long compared to the offset. The
        // offset is taken back out here, so that the quantity is the frame
        // rate it is named after from the first frame.
        let frame_rate = self.dframe as f64 / (self.f_time - self.random_seed).max(1e-6);
        let frame_rate_modifier = (42.5 / frame_rate) as f32;

        for i in 0..NUMSMOKEPARTICLES {
            if self.smoke.dead[i] != 0 {
                continue;
            }

            let mut delta = self.smoke.delta[i];

            for j in 0..self.num_streams {
                let spark = &self.spark[j % MAX_SPARKS];
                let d = [0, 1, 2].map(|c| self.smoke.position[i][c] - spark.position[c]);
                let rsquared = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];

                let mut f = (GRAVITY / rsquared) * frame_rate_modifier;

                if i % self.num_streams == j {
                    f *= 1.0 + STREAM_BIAS;
                }
                let mag = f / rsquared.sqrt();

                for (dc, dd) in delta.iter_mut().zip(d) {
                    *dc -= dd * mag;
                }
            }

            /* slow this particle down by drag */
            for dc in &mut delta {
                *dc *= self.drag;
            }

            if delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2] >= 25000000.0 {
                self.smoke.dead[i] = 1;
                continue;
            }

            /* update the position */
            self.smoke.delta[i] = delta;
            self.smoke.oldposition[i] = self.smoke.position[i];
            for (pc, dc) in self.smoke.position[i].iter_mut().zip(delta) {
                *pc += dc * self.f_delta_time as f32;
            }
        }
    }

    /// `DrawSmoke_Scalar`: one screen-space quad a particle, stretched along
    /// the way it moved and turned across it, so a fast particle is a streak.
    fn draw_smoke(&mut self, g: &mut Gl, brightness: f32, gl_width: f32, gl_height: f32) {
        let screen_ratio = gl_width / 1024.0;
        let hslash2 = gl_height * 0.5;
        let wslash2 = gl_width * 0.5;

        let width = (STREAM_SIZE + 2.5 * self.stream_expansion) * screen_ratio;

        g.glx.begin(Shape::Quads);
        for i in 0..NUMSMOKEPARTICLES {
            if self.smoke.dead[i] != 0 {
                continue;
            }
            let this_width = (STREAM_SIZE
                + (self.f_time as f32 - self.smoke.time[i]) * self.stream_expansion)
                * screen_ratio;
            if this_width >= width {
                self.smoke.dead[i] = 1;
                continue;
            }
            let z = self.smoke.position[i][2];
            let sx = self.smoke.position[i][0] * gl_width / z + wslash2;
            let sy = self.smoke.position[i][1] * gl_width / z + hslash2;
            let oldz = self.smoke.oldposition[i][2];
            if sx > gl_width + 50.0
                || sx < -50.0
                || sy > gl_height + 50.0
                || sy < -50.0
                || z < 25.0
                || oldz < 25.0
            {
                continue;
            }

            let w = (this_width / z).max(1.0);
            let oldx = self.smoke.oldposition[i][0];
            let oldy = self.smoke.oldposition[i][1];
            let oldscreenx = (oldx * gl_width / oldz) + wslash2;
            let oldscreeny = (oldy * gl_width / oldz) + hslash2;
            let dx = sx - oldscreenx;
            let dy = sy - oldscreeny;

            let d = fast_distance_2d(dx, dy);

            let sm = if d != 0.0 { w / d } else { 0.0 };
            let ow = (this_width / oldz).max(1.0);
            let os = if d != 0.0 { ow / d } else { 0.0 };

            let m = 1.0 + sm;

            let dxs = dx * sm;
            let dys = dy * sm;
            let dxos = dx * os;
            let dyos = dy * os;
            let dxm = dx * m;
            let dym = dy * m;

            self.smoke.anim_frame[i] += 1;
            if self.smoke.anim_frame[i] >= 64 {
                self.smoke.anim_frame[i] = 0;
            }

            let u0 = (self.smoke.anim_frame[i] & 7) as f32 * 0.125;
            let v0 = (self.smoke.anim_frame[i] >> 3) as f32 * 0.125;
            let u1 = u0 + 0.125;
            let v1 = v0 + 0.125;
            let mut cm = 1.375 - this_width / width;
            if self.smoke.dead[i] == 3 {
                cm *= 0.125;
                self.smoke.dead[i] = 1;
            }
            cm *= brightness;
            let c = self.smoke.color[i];
            g.glx.color4f(c[0] * cm, c[1] * cm, c[2] * cm, c[3] * cm);

            g.glx.tex_coord2f(u0, v0);
            g.glx.vertex3f(sx + dxm - dys, sy + dym + dxs, 0.0);
            g.glx.tex_coord2f(u0, v1);
            g.glx.vertex3f(sx + dxm + dys, sy + dym - dxs, 0.0);
            g.glx.tex_coord2f(u1, v1);
            g.glx
                .vertex3f(oldscreenx - dxm + dyos, oldscreeny - dym - dxos, 0.0);
            g.glx.tex_coord2f(u1, v0);
            g.glx
                .vertex3f(oldscreenx - dxm - dyos, oldscreeny - dym + dxos, 0.0);
        }
        g.glx.end();
    }

    /// `GLRenderScene`.
    fn render(&mut self, g: &mut Gl, now: f64, b: f64, gl_width: f32, gl_height: f32) {
        self.dframe += 1;

        self.f_old_time = self.f_time;
        self.f_time = now + self.random_seed;
        self.f_delta_time = self.f_time - self.f_old_time;

        self.drag = 0.9965f64.powf(self.f_delta_time * 85.0) as f32;

        self.star.update(self.f_time);

        for i in 0..self.num_streams.min(MAX_SPARKS) {
            self.spark[i].color = [1.0; 4];
            let (mode, t, dt, seed) = (
                self.color_mode,
                self.f_time,
                self.f_delta_time,
                self.random_seed,
            );
            self.spark[i].update(mode, t, dt, seed);
        }

        self.update_smoke();

        g.glx.blend(Blend::AlphaAdd);
        g.glx.texturing(true);
        self.draw_smoke(g, b as f32, gl_width, gl_height);
        g.glx.texturing(false);
    }
}

/// Simple smoothing routine.
fn smooth_texture(a: &mut [[u8; 32]; 32]) {
    let mut filter = [[0u8; 32]; 32];
    for i in 1..31 {
        for j in 1..31 {
            let mut t = a[i][j] as f32 * 4.0;
            t += a[i - 1][j] as f32;
            t += a[i + 1][j] as f32;
            t += a[i][j - 1] as f32;
            t += a[i][j + 1] as f32;
            t /= 8.0;
            filter[i][j] = t as u8;
        }
    }
    for i in 1..31 {
        for j in 1..31 {
            a[i][j] = filter[i][j];
        }
    }
}

/// Add some randomness to texture data.
fn speckle_texture(a: &mut [[u8; 32]; 32]) {
    for row in a.iter_mut().take(30).skip(2) {
        for cell in row.iter_mut().take(30).skip(2) {
            let mut speck: i32 = 1;
            while speck <= 32 && !random().is_multiple_of(2) {
                *cell = (*cell as i32 + speck).min(255) as u8;
                speck += speck;
            }
            let mut speck: i32 = 1;
            while speck <= 32 && !random().is_multiple_of(2) {
                *cell = (*cell as i32 - speck).max(0) as u8;
                speck += speck;
            }
        }
    }
}

fn make_small_texture(a: &mut [[u8; 32]; 32], first: bool) {
    for (i, row) in a.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            let r = (((i as f64 - 15.5) * (i as f64 - 15.5))
                + ((j as f64 - 15.5) * (j as f64 - 15.5)))
                .sqrt();
            let t = if r > 15.0 {
                0.0
            } else {
                255.0 * (r * std::f64::consts::PI / 31.0).cos()
            };
            *cell = if first {
                t as u8
            } else {
                ((t + *cell as f64 + *cell as f64) / 3.0).min(255.0) as u8
            };
        }
    }
    speckle_texture(a);
    smooth_texture(a);
    smooth_texture(a);
}

/// `MakeTexture`: an eight by eight sheet of soft blobs, each a cosine
/// falloff speckled and smoothed, and each the average of itself and the one
/// before, so that walking the sheet is a loop rather than a jump.
fn make_texture() -> Vec<u8> {
    let mut small = [[0u8; 32]; 32];
    let mut big = vec![0u8; 256 * 256 * 4];
    for i in 0..8 {
        for j in 0..8 {
            if i == 7 && j == 7 {
                /* AverageLastAndFirstTextures */
                for (y, row) in small.iter_mut().enumerate() {
                    for (x, cell) in row.iter_mut().enumerate() {
                        let t = (*cell as i32 + big[(y * 256 + x) * 4] as i32) / 2;
                        *cell = t.min(255) as u8;
                    }
                }
            } else {
                make_small_texture(&mut small, i == 0 && j == 0);
            }
            /* CopySmallTextureToBigTexture */
            for (y, row) in small.iter().enumerate() {
                for (x, cell) in row.iter().enumerate() {
                    // Luminance and alpha, both the same value, which the
                    // recorder takes as RGBA.
                    let o = ((y + i * 32) * 256 + (x + j * 32)) * 4;
                    big[o] = *cell;
                    big[o + 1] = *cell;
                    big[o + 2] = *cell;
                    big[o + 3] = *cell;
                }
            }
        }
    }
    big
}

struct Flurry {
    streams: Vec<Stream>,
    texture: u32,
    /// The screen as it stood at the end of the last frame.
    screenshot: u32,
    tex_w: i32,
    tex_h: i32,
    old_frame_time: f64,
    /// Seconds of the saver's own time, which is the delays it has asked for
    /// added up rather than anything the wall clock says.
    now: f64,
    gl_width: f32,
    gl_height: f32,
}

impl Hack3d for Flurry {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        // Upstream reads the wall clock and refuses to draw a frame that
        // arrives sooner than a sixtieth of a second after the last, with a
        // note that Flurry is designed for that speed and saturates above it.
        // Here the host asks for a frame and is told how long to wait before
        // the next, so a frame *is* the delay long: taking the clock from
        // there rather than from the wall keeps the two in step and makes the
        // saver run the same whatever the machine is doing.
        let delay = g.res.int("delay").max(0) as u32;
        let delta_frame_time = (f64::from(delay) / 1_000_000.0).max(1.0 / 200.0);
        let mut alpha = if self.old_frame_time < 0.0 {
            /* special case the first frame -- clear to black */
            1.0
        } else {
            5.0 * delta_frame_time
        };
        self.now += delta_frame_time;
        let new_frame_time = self.now;
        self.old_frame_time = new_frame_time;
        if alpha > 0.2 {
            alpha = 0.2;
        }

        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.lighting(false);
        g.glx.clear();

        // Nothing is ever cleared here: the smoke is laid over what is
        // already on screen and a black rectangle fades it. A colour buffer
        // does not survive to the next frame, so the last one is put back
        // from a texture first.
        let tw = g.width() as f32 / self.tex_w as f32;
        let th = g.height() as f32 / self.tex_h as f32;
        g.glx.blend(Blend::Off);
        g.glx.texturing(true);
        g.glx.bind_texture(self.screenshot);
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        g.glx.begin(Shape::Quads);
        for (u, v, x, y) in [
            (0.0, 0.0, 0.0, 0.0),
            (tw, 0.0, self.gl_width, 0.0),
            (tw, th, self.gl_width, self.gl_height),
            (0.0, th, 0.0, self.gl_height),
        ] {
            g.glx.tex_coord2f(u, v);
            g.glx.vertex3f(x, y, 0.0);
        }
        g.glx.end();
        g.glx.texturing(false);

        g.glx.blend(Blend::Alpha);
        g.glx.color4f(0.0, 0.0, 0.0, alpha as f32);
        g.glx.begin(Shape::Quads);
        for (x, y) in [
            (0.0, 0.0),
            (self.gl_width, 0.0),
            (self.gl_width, self.gl_height),
            (0.0, self.gl_height),
        ] {
            g.glx.vertex3f(x, y, 0.0);
        }
        g.glx.end();

        let brite = delta_frame_time.powf(0.75) * 10.0;
        g.glx.bind_texture(self.texture);
        let (w, h) = (self.gl_width, self.gl_height);
        for i in 0..self.streams.len() {
            let b = brite * self.streams[i].brite_factor;
            self.streams[i].render(g, new_frame_time, b, w, h);
        }

        g.glx.blend(Blend::Off);
        g.glx.color3f(1.0, 1.0, 1.0);

        // And keep what that came to for the next frame.
        g.glx.texturing(true);
        g.glx.bind_texture(self.screenshot);
        g.glx.copy_tex_sub_image_2d();
        g.glx.texturing(false);

        delay
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx
            .ortho(0.0, width as f32, 0.0, height as f32, -1.0, 1.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        self.gl_width = width as f32;
        self.gl_height = height as f32;

        // The kept screen has to be at least as big as the window.
        let pot = |n: i32| (n.max(1) as u32).next_power_of_two() as i32;
        let (tw, th) = (pot(width), pot(height));
        if tw != self.tex_w || th != self.tex_h {
            self.tex_w = tw;
            self.tex_h = th;
            g.glx.bind_texture(self.screenshot);
            g.glx.tex_image_2d(tw, th, Vec::new());
        }
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }
}

/// Upstream's presets, each a list of flurries to lay over each other.
fn preset(name: &str) -> Vec<(usize, ColorMode, f32, f64, f64)> {
    let n = match name {
        "water" => 0,
        "fire" => 1,
        "psychedelic" => 2,
        "rgb" => 3,
        "binary" => 4,
        "classic" => 5,
        "insane" => 6,
        // "random", and anything else, which upstream treats as an error and
        // exits on. There is nothing to exit to here.
        _ => random_below(6),
    };
    match n {
        0 => vec![(1, ColorMode::Blue, 100.0, 2.0, 2.0); 9],
        1 => vec![(12, ColorMode::SlowCyclic, 10000.0, 0.2, 1.0)],
        2 => vec![(10, ColorMode::Rainbow, 200.0, 2.0, 1.0)],
        3 => vec![
            (3, ColorMode::Red, 100.0, 0.8, 1.0),
            (3, ColorMode::Green, 100.0, 0.8, 1.0),
            (3, ColorMode::Blue, 100.0, 0.8, 1.0),
        ],
        4 => vec![
            (16, ColorMode::Tiedye, 1000.0, 0.5, 1.0),
            (16, ColorMode::Tiedye, 1000.0, 1.5, 1.0),
        ],
        5 => vec![(5, ColorMode::Tiedye, 10000.0, 1.0, 1.0)],
        _ => vec![(64, ColorMode::Tiedye, 1000.0, 0.5, 0.5)],
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut streams: Vec<Stream> = preset(g.res.string("preset"))
        .into_iter()
        .map(|(n, c, thick, speed, bf)| Stream::new(g.time, n, c, thick, speed, bf))
        .collect();
    // Upstream builds its list by pushing on the front, so the last one made
    // is drawn first.
    streams.reverse();

    let texture = g.glx.gen_texture();
    g.glx.bind_texture(texture);
    g.glx.tex_image_2d(256, 256, make_texture());

    let screenshot = g.glx.gen_texture();

    let mut st = Flurry {
        streams,
        texture,
        screenshot,
        tex_w: 0,
        tex_h: 0,
        old_frame_time: -1.0,
        now: 0.0,
        gl_width: 1.0,
        gl_height: 1.0,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:      10000",
    "*showFPS:    False",
    "*preset:     random",
];

const PRESETS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random",
    },
    SelectItem {
        value: "classic",
        label: "Classic",
    },
    SelectItem {
        value: "rgb",
        label: "RGB",
    },
    SelectItem {
        value: "fire",
        label: "Fire",
    },
    SelectItem {
        value: "water",
        label: "Water",
    },
    SelectItem {
        value: "binary",
        label: "Binary",
    },
    SelectItem {
        value: "psychedelic",
        label: "Psychedelic",
    },
    SelectItem {
        value: "insane",
        label: "Insane",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::select("preset", "Preset", PRESETS, "random"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "flurry",
    label: "Flurry",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Calum Robinson and Tobias Sargeant",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=0beqUyN5ZsI"),
        blurb: "A colourful star(fish)like flurry of particles.",
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

    fn one(mode: ColorMode, n: usize) -> Stream {
        crate::runtime::ya_rand_init(20260812);
        Stream::new(0.0, n, mode, 10000.0, 1.0, 1.0)
    }

    #[test]
    fn the_texture_is_a_sheet_of_soft_blobs() {
        crate::runtime::ya_rand_init(20260812);
        let t = make_texture();
        assert_eq!(t.len(), 256 * 256 * 4);
        // Luminance and alpha are the same, as upstream's two-channel image
        // has them.
        for p in t.chunks_exact(4) {
            assert_eq!(p[0], p[3]);
        }
        // Each of the sixty-four cells is bright in the middle and dark at
        // its corner, which is what makes a puff of smoke round.
        let at = |x: usize, y: usize| t[(y * 256 + x) * 4] as i32;
        for i in 0..8 {
            for j in 0..8 {
                let (ox, oy) = (j * 32, i * 32);
                assert!(at(ox + 16, oy + 16) > 150, "cell {i},{j} has no middle");
                assert_eq!(at(ox, oy), 0, "cell {i},{j} has a lit corner");
            }
        }
    }

    #[test]
    fn the_star_never_comes_back_to_itself() {
        let mut s = Star::new();
        let mut seen: Vec<[f32; 3]> = Vec::new();
        for i in 0..500 {
            s.update(i as f64 * 0.02);
            // It stays in the box its own radius allows, and pushed out along
            // z by the distance the whole figure is held at.
            assert!(s.position[0].abs() < 1200.0, "{:?}", s.position);
            assert!(s.position[1].abs() < 1200.0, "{:?}", s.position);
            assert!(s.position[2] > 500.0, "{:?}", s.position);
            for p in &seen {
                let d = (0..3).map(|c| (p[c] - s.position[c]).powi(2)).sum::<f32>();
                assert!(d > 1e-6, "the star repeated at {:?}", s.position);
            }
            seen.push(s.position);
        }
    }

    #[test]
    fn the_smoke_leaves_the_star_and_dies_of_old_age() {
        let mut g = Gl::for_test(640, 480);
        let mut st = one(ColorMode::Tiedye, 5);
        // Nothing is alive until the first update has been through.
        assert_eq!(st.smoke.dead.iter().filter(|&&d| d == 0).count(), 0);

        let mut most = 0;
        for i in 1..400 {
            st.render(&mut g, i as f64 * 0.02, 1.0, 640.0, 480.0);
            most = most.max(st.smoke.dead.iter().filter(|&&d| d == 0).count());
        }
        // With the frame rate right from the start, the smoke lives long
        // enough to fill out.
        assert!(most > 400, "{most}");
        // A puff per stream every hundred and twenty-first of a second, and
        // each lives until it has widened past the stream, so the population
        // settles well short of the three thousand six hundred it has room
        // for.
        let alive = st.smoke.dead.iter().filter(|&&d| d == 0).count();
        assert!(alive > 20 && alive <= NUMSMOKEPARTICLES, "{alive}");

        // Every live particle came from the star and is being pulled about,
        // so none of them is still sitting exactly where it was born.
        let moved = (0..NUMSMOKEPARTICLES)
            .filter(|&i| st.smoke.dead[i] == 0 && st.smoke.position[i] != st.smoke.oldposition[i])
            .count();
        assert!(moved * 2 > alive, "{moved} of {alive} moved");
    }

    #[test]
    fn a_preset_lays_its_flurries_over_each_other() {
        crate::runtime::ya_rand_init(20260812);
        assert_eq!(preset("water").len(), 9);
        assert_eq!(preset("rgb").len(), 3);
        assert_eq!(preset("classic").len(), 1);
        assert_eq!(preset("insane")[0].0, 64);
        // A fixed mode gives a colour that does not depend on the time, and
        // a cycling one gives one that does.
        for m in [ColorMode::Red, ColorMode::Blue, ColorMode::Green] {
            assert_eq!(m.base(0.0, 0.0), m.base(100.0, 0.0));
        }
        assert_ne!(
            ColorMode::Rainbow.base(0.0, 0.0),
            ColorMode::Rainbow.base(1.0, 0.0)
        );
        // A fixed mode is a fixed point on the wheel: red, green and blue
        // land on different colours.
        let r = ColorMode::Red.base(0.0, 0.0);
        let gr = ColorMode::Green.base(0.0, 0.0);
        assert!(r[0] > r[1] && r[0] > r[2], "{r:?}");
        assert!(gr[1] > gr[0] || gr[2] > gr[0], "{gr:?}");
    }

    #[test]
    fn a_frame_is_one_block_a_flurry() {
        crate::runtime::ya_rand_init(20260812);
        let mut g = Gl::for_test(640, 480);
        let mut st = Flurry {
            streams: vec![Stream::new(0.0, 5, ColorMode::Tiedye, 10000.0, 1.0, 1.0)],
            texture: 1,
            screenshot: 2,
            tex_w: 1024,
            tex_h: 512,
            old_frame_time: -1.0,
            now: 0.0,
            gl_width: 640.0,
            gl_height: 480.0,
        };
        let mut most = 0;
        for _ in 1..200 {
            g.glx.start_frame(640, 480);
            st.draw(&mut g);
            most = most.max(g.glx.frame().batches.len());
        }
        // The kept screen, the black fade, the smoke, and the copy back.
        assert!(most <= 6, "{most}");
        assert!(g.glx.frame().vertices.len() > 100);
    }
}
