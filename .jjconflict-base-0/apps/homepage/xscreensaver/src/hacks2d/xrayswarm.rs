//! Port of `hacks/xrayswarm.c`.
//!
//! ```text
//! Copyright (c) 2000 by Chris Leger (xrayjones@users.sourceforge.net)
//!
//! xrayswarm - a shameless ripoff of the 'swarm' screensaver on SGI
//!   boxes.
//!
//! Version 1.0 - initial release.  doesn't read any special command-line
//!   options, and only supports the variable 'delay' via Xresources.
//!   (the delay resouces is most useful on systems w/o gettimeofday, in
//!   which case automagical level-of-detail for FPS maintainance can't
//!   be used.)
//!
//!   The code isn't commented, but isn't too ugly. It should be pretty
//!   easy to understand, with the exception of the colormap stuff.
//!
//! Permission is hereby granted, free of charge, to any person obtaining
//! a copy of this software and associated documentation files (the
//! "Software"), to deal in the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to
//! permit persons to whom the Software is furnished to do so, subject to
//! the following conditions:
//!
//! The above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
//! IN NO EVENT SHALL THE X CONSORTIUM BE LIABLE FOR ANY CLAIM, DAMAGES OR
//! OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
//! ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
//! OTHER DEALINGS IN THE SOFTWARE.
//!
//! Except as contained in this notice, the name of the X Consortium shall
//! not be used in advertising or otherwise to promote the sale, use or
//! other dealings in this Software without prior written authorization
//! from the X Consortium.
//! ```
//!
//! Two populations, and only one of them is going anywhere. A handful of
//! targets drift about under a random walk: every step they are shoved in a
//! uniformly random direction, their speed is clipped to a ceiling, and they
//! bounce off the edges. The swarm is everything else, and each of its members
//! accelerates flat out towards whichever target it has been assigned, with a
//! little noise mixed into the direction it thinks that is.
//!
//! Flying at a moving point at full acceleration is what makes the shape.
//! Nothing brakes, so a swarm member overshoots its target, has to turn around,
//! overshoots again, and settles into a rosette around it. Meanwhile the target
//! has wandered off, and the whole knot is dragged after it. A speed floor
//! keeps anything from ever coming to rest.
//!
//! The vapour trail is a ring buffer of the last few dozen positions, redrawn
//! whole every frame. Redrawing all of it rather than adding one segment is
//! what pays for the colour schemes: the fade along a trail is a table of
//! colour indices, and the two schizo schemes rotate that table by the buffer
//! head each frame, so the colours run backwards along a trail that has not
//! itself moved.
//!
//! Reassignment is deliberately lazy and deliberately sticky. Five swarm
//! members per step, in rotation, look for a better target, and a candidate has
//! to be nearer than twice the distance of the one they already have. So a
//! swarm splits and re-forms in slow lumps instead of everyone switching at
//! once. Every so often the whole thing mutates: a parameter is nudged by up to
//! a quarter, a target is demoted into the swarm or a swarm member promoted to
//! a target, the colour scheme changes, or the trails are thrown away and
//! everyone starts again somewhere new.
//!
//! That mutation is the one place this departs from the C. Upstream has two
//! versions of it: one for systems with `gettimeofday` and one without. The
//! timed version is what a normal build compiles, and it cannot fire, because
//! the window it measures starts afresh at the top of every frame, so its test
//! for half a second having passed is really a test for one frame having taken
//! half a second. Ported literally, the saver would roll a few parameters at
//! startup and then never change again. This is the untimed version, which
//! rolls the same changes against a per-frame probability.
//!
//! The rest of that timed block is upstream's own level-of-detail: it measures
//! frames per second and shortens the trails until it hits its target. That has
//! no place here, where a browser drops frames by itself and the alternative is
//! a saver that looks different on a faster machine.
//!
//! One quirk kept as-is: the frame loop assigns a new step size but does not
//! recompute the two constants derived from it, so the acceleration correction
//! term is computed against the startup step size until the first mutation
//! recomputes them.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, rgb};
use crate::runtime::{About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, frand, random};

const MAX_TRAIL_LEN: usize = 60;
const MAX_BUGS: usize = 100;
const MAX_TARGETS: usize = 10;
const DESIRED_DT: f32 = 0.2;

/// The colour schemes. The last two are schizo variants that upstream draws
/// less often than the rest.
const GRAY_TRAILS: i32 = 0;
const GRAY_SCHIZO: i32 = 1;
const COLOR_TRAILS: i32 = 2;
const RANDOM_TRAILS: i32 = 3;
const RANDOM_SCHIZO: i32 = 4;
const COLOR_SCHIZO: i32 = 5;
const NUM_SCHEMES: i32 = 6;

/// `frand`, in the `float` this hack computes in.
fn frandf(f: f32) -> f32 {
    frand(f as f64) as f32
}

fn rand_mod(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (random() % n as u32) as usize
}

/// A swarm member or a target. Both move, both leave a trail.
#[derive(Clone)]
struct Bug {
    pos: [f32; 2],
    /// The ring buffer of past positions, already in pixels.
    hist: [[i32; 2]; MAX_TRAIL_LEN],
    vel: [f32; 2],
    /// Which target this one is flying at. Upstream holds a pointer, and zeroes
    /// it for the bugs past the live count that nothing looks at.
    closest: usize,
}

impl Default for Bug {
    fn default() -> Self {
        Self {
            pos: [0.0; 2],
            hist: [[0; 2]; MAX_TRAIL_LEN],
            vel: [0.0; 2],
            closest: 0,
        }
    }
}

/// `initCMap`: black, three unused entries, three sixteen-step fades, and then
/// a random walk through colour space for the random schemes.
///
/// Kept as upstream's flat run of channel bytes, because the walk at the end
/// reads back across colour boundaries and the offsets only make sense there.
fn init_cmap() -> Vec<Pixel> {
    let mut c: Vec<u8> = Vec::with_capacity(768);
    {
        let mut push = |v: i32| c.push(v as u8);

        // Colour 0 is black.
        push(0);
        push(0);
        push(0);

        // Upstream's comments call colours 1, 2 and 3 red, green and blue; the
        // code writes red three times. Nothing indexes them.
        for _ in 0..3 {
            push(255);
            push(0);
            push(0);
        }

        // Greyscale fade at 4, red fade at 20, blue fade at 36; sixteen each.
        for i in 0..16 {
            let temp = (i * 16).min(255);
            push(255 - temp);
            push(255 - temp);
            push(255 - temp);
        }
        for i in 0..16 {
            let temp = (i * 16).min(255);
            push(255 - temp);
            push((255.0 - (i as f32 / 16.0 + 0.001).powf(0.3) * 255.0) as i32);
            push(65 - temp / 4);
        }
        for i in 0..16 {
            let temp = (i * 16).min(255);
            push(32 - temp / 8);
            push((180.0 - (i as f32 / 16.0 + 0.001).powf(0.3) * 180.0) as i32);
            push(255 - temp);
        }
    }

    // The random colours start at 52. Red and green take a random walk of up to
    // sixteen a step; blue is the average of this colour's red and the previous
    // colour's blue, divided by a number that keeps growing, so the walk sheds
    // its blue within a few entries and never gets it back.
    let s = c.len();
    c.push((random() & 255) as u8);
    c.push((random() & 255) as u8);
    c.push(c[s] / 2 + c[s - 1] / 2);
    for i in 0..MAX_TRAIL_LEN {
        let n = c.len();
        c.push(((c[n - 3] as i32 + (random() & 31) as i32 - 16) & 255) as u8);
        c.push(((c[n - 2] as i32 + (random() & 31) as i32 - 16) & 255) as u8);
        let n = c.len();
        c.push((c[n - 2] as f32 / (i + 2) as f32 + c[n - 3] as f32 / (i + 2) as f32) as i32 as u8);
    }

    // `numColors` is one past the last colour written, and upstream allocates
    // that one from a zeroed array, so it is black.
    c.push(0);
    c.push(0);
    c.push(0);
    c.chunks_exact(3).map(|v| rgb(v[0], v[1], v[2])).collect()
}

struct XraySwarm {
    gc: Gc,
    palette: Vec<Pixel>,
    xsize: i32,
    ysize: i32,
    delay: i32,
    maxx: f32,
    maxy: f32,

    dt: f32,
    target_vel: f32,
    target_acc: f32,
    max_vel: f32,
    max_acc: f32,
    noise: f32,
    min_vel_multiplier: f32,

    nbugs: usize,
    ntargets: usize,
    trail_len: usize,

    dt_inv: f32,
    half_dt_sq: f32,
    target_vel_sq: f32,
    max_vel_sq: f32,
    min_vel_sq: f32,
    min_vel: f32,

    bugs: Vec<Bug>,
    targets: Vec<Bug>,
    head: usize,
    tail: usize,
    color_scheme: i32,
    change_prob: f32,

    /// The fade tables, one palette index per step along a trail.
    gray_index: [usize; MAX_TRAIL_LEN],
    red_index: [usize; MAX_TRAIL_LEN],
    blue_index: [usize; MAX_TRAIL_LEN],
    gray_s_index: [usize; MAX_TRAIL_LEN],
    red_s_index: [usize; MAX_TRAIL_LEN],
    blue_s_index: [usize; MAX_TRAIL_LEN],
    random_index: [usize; MAX_TRAIL_LEN],
    num_random_colors: usize,

    /// Which swarm member is next to reconsider its target.
    check_index: usize,
    rsc_call_depth: i32,
    rbc_call_depth: i32,

    delay_accum: i32,
    sleep_count: i32,
}

impl XraySwarm {
    fn new(d: &mut Dpy) -> Self {
        let xsize = d.width();
        let ysize = d.height();
        let line_width = if xsize > 2560 || ysize > 2560 { 3 } else { 1 };
        let mut gc = Gc::new(d.res.pixel("foreground"), d.res.pixel("background"));
        gc.set_line_width(line_width);

        let mut st = Self {
            gc,
            palette: init_cmap(),
            xsize,
            ysize,
            delay: d.res.int("delay").max(0),
            maxx: 1.0,
            maxy: ysize as f32 / xsize as f32,

            dt: 0.3,
            target_vel: 0.03,
            target_acc: 0.02,
            max_vel: 0.05,
            max_acc: 0.03,
            noise: 0.01,
            min_vel_multiplier: 0.5,

            nbugs: 0,
            ntargets: 0,
            trail_len: 0,

            dt_inv: 0.0,
            half_dt_sq: 0.0,
            target_vel_sq: 0.0,
            max_vel_sq: 0.0,
            min_vel_sq: 0.0,
            min_vel: 0.0,

            bugs: vec![Bug::default(); MAX_BUGS],
            targets: vec![Bug::default(); MAX_TARGETS],
            head: 0,
            tail: 0,
            color_scheme: COLOR_TRAILS,
            change_prob: 0.08,

            gray_index: [0; MAX_TRAIL_LEN],
            red_index: [0; MAX_TRAIL_LEN],
            blue_index: [0; MAX_TRAIL_LEN],
            gray_s_index: [0; MAX_TRAIL_LEN],
            red_s_index: [0; MAX_TRAIL_LEN],
            blue_s_index: [0; MAX_TRAIL_LEN],
            random_index: [0; MAX_TRAIL_LEN],
            num_random_colors: MAX_TRAIL_LEN,

            check_index: 0,
            rsc_call_depth: 0,
            rbc_call_depth: 0,
            delay_accum: 0,
            sleep_count: 0,
        };

        st.compute_constants();
        st.init_bugs(true);
        st.compute_color_indices();

        // Roll the starting parameters away from the defaults.
        for _ in 0..=(rand_mod(5) + 5) {
            st.random_small_change();
        }
        st
    }

    fn compute_constants(&mut self) {
        self.half_dt_sq = self.dt * self.dt * 0.5;
        self.dt_inv = 1.0 / self.dt;
        self.target_vel_sq = self.target_vel * self.target_vel;
        self.max_vel_sq = self.max_vel * self.max_vel;
        self.min_vel = self.max_vel * self.min_vel_multiplier;
        self.min_vel_sq = self.min_vel * self.min_vel;
    }

    /// `initBugs`. The counts and the trail length are rolled once, on the
    /// first call, and kept from then on.
    fn init_bugs(&mut self, first: bool) {
        self.head = 0;
        self.tail = 0;
        self.bugs.iter_mut().for_each(|b| *b = Bug::default());
        self.targets.iter_mut().for_each(|b| *b = Bug::default());

        if first {
            self.ntargets = ((0.25 + frandf(0.75) * frandf(1.0)) * MAX_TARGETS as f32) as usize;
            self.ntargets = self.ntargets.clamp(1, MAX_TARGETS);

            self.nbugs = ((0.25 + frandf(0.75) * frandf(1.0)) * MAX_BUGS as f32) as usize;
            if self.nbugs <= self.ntargets {
                self.nbugs = self.ntargets + 1;
            }
            self.nbugs = self.nbugs.min(MAX_BUGS);

            self.trail_len = ((1.0 - frandf(0.6) * frandf(1.0)) * MAX_TRAIL_LEN as f32) as usize;
            self.trail_len = self.trail_len.clamp(1, MAX_TRAIL_LEN);
        }

        let (maxx, maxy, max_vel, target_vel) =
            (self.maxx, self.maxy, self.max_vel, self.target_vel);
        let (xsize, head, ntargets) = (self.xsize as f32, self.head, self.ntargets);
        for i in 0..self.nbugs {
            let b = &mut self.bugs[i];
            b.pos[0] = frandf(maxx);
            b.pos[1] = frandf(maxy);
            b.vel[0] = frandf(max_vel / 2.0);
            b.vel[1] = frandf(max_vel / 2.0);
            // Both coordinates scale by the width: the vertical extent is
            // already stored as a fraction of it.
            b.hist[head][0] = (b.pos[0] * xsize) as i32;
            b.hist[head][1] = (b.pos[1] * xsize) as i32;
            b.closest = rand_mod(ntargets);
        }
        for i in 0..self.ntargets {
            let b = &mut self.targets[i];
            b.pos[0] = frandf(maxx);
            b.pos[1] = frandf(maxy);
            b.vel[0] = frandf(target_vel / 2.0);
            b.vel[1] = frandf(target_vel / 2.0);
            b.hist[head][0] = (b.pos[0] * xsize) as i32;
            b.hist[head][1] = (b.pos[1] * xsize) as i32;
        }
    }

    fn pick_new_targets(&mut self) {
        for i in 0..self.nbugs {
            self.bugs[i].closest = rand_mod(self.ntargets);
        }
    }

    /// `computeColorIndices`. The tables are filled backwards: the newest end
    /// of a trail gets the brightest colour.
    fn compute_color_indices(&mut self) {
        let n = self.trail_len;
        let step = |i: usize, base: usize, top: usize| -> usize {
            (base + (i as f32 * 16.0 / n as f32 + 0.5) as usize).min(top)
        };
        for i in 0..n {
            self.gray_index[n - 1 - i] = step(i, 4, 19);
            self.red_index[n - 1 - i] = step(i, 20, 35);
            self.blue_index[n - 1 - i] = step(i, 36, 51);
            // The grey and red schizo tables are the plain fades; only the blue
            // one repeats, which is what makes that scheme flicker.
            self.gray_s_index[n - 1 - i] = step(i, 4, 19);
            self.red_s_index[n - 1 - i] = step(i, 20, 35);
        }

        let schizo_length = (n / 2).max(3);
        for i in 0..n {
            let v = 36.0 + 16.0 * (i % schizo_length) as f32 / (schizo_length - 1) as f32 + 0.5;
            self.blue_s_index[n - 1 - i] = (v as usize).min(51);
        }

        for i in 0..n {
            self.random_index[i] = 52 + rand_mod(self.num_random_colors);
        }
    }

    /// One trail segment, for every bug and every target.
    fn draw_segment(&self, d: &mut Dpy, j: usize, next: usize) {
        for b in &self.bugs[..self.nbugs] {
            d.win().draw_line(
                &self.gc,
                b.hist[j][0],
                b.hist[j][1],
                b.hist[next][0],
                b.hist[next][1],
            );
        }
    }

    fn draw_target_segment(&self, d: &mut Dpy, j: usize, next: usize) {
        for b in &self.targets[..self.ntargets] {
            d.win().draw_line(
                &self.gc,
                b.hist[j][0],
                b.hist[j][1],
                b.hist[next][0],
                b.hist[next][1],
            );
        }
    }

    /// `drawBugs`: rub out the oldest segment if the ring is full, then redraw
    /// every trail from tail to head.
    fn draw_bugs(&mut self, d: &mut Dpy, s: &Scheme) {
        if (self.head + 1) % self.trail_len == self.tail {
            let next = (self.tail + 1) % self.trail_len;
            self.gc.set_foreground(self.palette[0]);
            self.draw_segment(d, self.tail, next);
            self.draw_target_segment(d, self.tail, next);
            self.tail = next;
        }

        let mut ci = s.start;
        let mut j = self.tail;
        while j != self.head {
            let next = (j + 1) % self.trail_len;

            self.gc.set_foreground(self.palette[s.bug[ci]]);
            self.draw_segment(d, j, next);
            self.gc.set_foreground(self.palette[s.target[ci]]);
            self.draw_target_segment(d, j, next);

            ci = (ci + 1) % s.len;
            j = next;
        }
    }

    /// `clearBugs`: the same walk, all in the background colour.
    fn clear_bugs(&mut self, d: &mut Dpy) {
        self.gc.set_foreground(self.palette[0]);
        self.tail = if self.tail == 0 {
            self.trail_len - 1
        } else {
            self.tail - 1
        };

        if (self.head + 1) % self.trail_len == self.tail {
            let next = (self.tail + 1) % self.trail_len;
            self.draw_segment(d, self.tail, next);
            self.draw_target_segment(d, self.tail, next);
            self.tail = next;
        }

        let mut j = self.tail;
        while j != self.head {
            let next = (j + 1) % self.trail_len;
            self.draw_segment(d, j, next);
            self.draw_target_segment(d, j, next);
            j = next;
        }
    }

    fn update_state(&mut self) {
        self.head = (self.head + 1) % self.trail_len;

        // Five swarm members reconsider which target they are flying at. A new
        // one has to be inside twice the distance of the one they have, so a
        // knot of them stays together rather than everyone switching at once.
        for _ in 0..5 {
            self.check_index = (self.check_index + 1) % self.nbugs;
            let (bx, by) = {
                let b = &self.bugs[self.check_index];
                (b.pos[0], b.pos[1])
            };
            let mut c = self.bugs[self.check_index].closest;
            let mut temp = {
                let (ax, ay) = (self.targets[c].pos[0] - bx, self.targets[c].pos[1] - by);
                ax * ax + ay * ay
            };
            for i in 0..self.ntargets {
                if i == c {
                    continue;
                }
                let (ax, ay) = (self.targets[i].pos[0] - bx, self.targets[i].pos[1] - by);
                let theta = ax * ax + ay * ay;
                if theta < temp * 2.0 {
                    c = i;
                    temp = theta;
                }
            }
            self.bugs[self.check_index].closest = c;
        }

        let (dt, dt_inv, half_dt_sq) = (self.dt, self.dt_inv, self.half_dt_sq);
        let (maxx, maxy, xsize, head) = (self.maxx, self.maxy, self.xsize as f32, self.head);

        // Targets wander: a shove in a uniformly random direction every step.
        let (target_acc, target_vel, target_vel_sq) =
            (self.target_acc, self.target_vel, self.target_vel_sq);
        for i in 0..self.ntargets {
            // Upstream's own rounding of a whole turn, kept as it wrote it.
            #[allow(clippy::approx_constant)]
            let theta = frandf(6.28);
            let mut ax = target_acc * theta.cos();
            let mut ay = target_acc * theta.sin();
            let b = &mut self.targets[i];

            b.vel[0] += ax * dt;
            b.vel[1] += ay * dt;

            let mut temp = b.vel[0] * b.vel[0] + b.vel[1] * b.vel[1];
            if temp > target_vel_sq {
                temp = target_vel / temp.sqrt();
                // What the clamp did counts as acceleration too.
                ax = b.vel[0];
                ay = b.vel[1];
                b.vel[0] *= temp;
                b.vel[1] *= temp;
                ax = (b.vel[0] - ax) * dt_inv;
                ay = (b.vel[1] - ay) * dt_inv;
            }

            b.pos[0] += b.vel[0] * dt + ax * half_dt_sq;
            b.pos[1] += b.vel[1] * dt + ay * half_dt_sq;
            bounce(b, maxx, maxy);

            b.hist[head][0] = (b.pos[0] * xsize) as i32;
            b.hist[head][1] = (b.pos[1] * xsize) as i32;
        }

        // The swarm flies at its targets flat out, with nothing to brake it.
        let (max_acc, max_vel, max_vel_sq) = (self.max_acc, self.max_vel, self.max_vel_sq);
        let (min_vel, min_vel_sq, noise) = (self.min_vel, self.min_vel_sq, self.noise);
        for i in 0..self.nbugs {
            let target = self.targets[self.bugs[i].closest].pos;
            let b = &mut self.bugs[i];
            let theta =
                (target[1] - b.pos[1] + frandf(noise)).atan2(target[0] - b.pos[0] + frandf(noise));
            let mut ax = max_acc * theta.cos();
            let mut ay = max_acc * theta.sin();

            b.vel[0] += ax * dt;
            b.vel[1] += ay * dt;

            let mut temp = b.vel[0] * b.vel[0] + b.vel[1] * b.vel[1];
            if temp > max_vel_sq || temp < min_vel_sq {
                temp = if temp > max_vel_sq {
                    max_vel / temp.sqrt()
                } else {
                    min_vel / temp.sqrt()
                };
                ax = b.vel[0];
                ay = b.vel[1];
                b.vel[0] *= temp;
                b.vel[1] *= temp;
                ax = (b.vel[0] - ax) * dt_inv;
                ay = (b.vel[1] - ay) * dt_inv;
            }

            b.pos[0] += b.vel[0] * dt + ax * half_dt_sq;
            b.pos[1] += b.vel[1] * dt + ay * half_dt_sq;
            bounce(b, maxx, maxy);

            b.hist[head][0] = (b.pos[0] * xsize) as i32;
            b.hist[head][1] = (b.pos[1] * xsize) as i32;
        }
    }

    /// `mutateBug`: promote a swarm member to a target, or demote a target.
    fn mutate_bug(&mut self, to_bug: bool) {
        if !to_bug {
            if self.ntargets < MAX_TARGETS - 1 && self.nbugs > 1 {
                let i = rand_mod(self.nbugs);
                self.targets[self.ntargets] = self.bugs[i].clone();
                self.bugs[i] = self.bugs[self.nbugs - 1].clone();
                self.targets[self.ntargets].pos[0] = frandf(self.maxx);
                self.targets[self.ntargets].pos[1] = frandf(self.maxy);
                self.nbugs -= 1;
                self.ntargets += 1;

                // Give the new target a share of the swarm to start with.
                let mut i = 0;
                while i < self.nbugs {
                    self.bugs[i].closest = self.ntargets - 1;
                    i += self.ntargets;
                }
            }
        } else if self.ntargets > 1 && self.nbugs < MAX_BUGS - 1 {
            let i = rand_mod(self.ntargets);
            self.bugs[self.nbugs] = self.targets[i].clone();
            self.ntargets -= 1;

            self.bugs[self.nbugs].closest = rand_mod(self.ntargets);

            // Everyone who was flying at the target that just left, or at the
            // one about to be moved into its slot, needs a new one.
            for j in 0..self.nbugs {
                if self.bugs[j].closest == self.ntargets {
                    self.bugs[j].closest = i;
                } else if self.bugs[j].closest == i {
                    self.bugs[j].closest = rand_mod(self.ntargets);
                }
            }
            self.nbugs += 1;
            self.targets[i] = self.targets[self.ntargets].clone();
        }
    }

    /// `randomSmallChange`: nudge one thing, then put the parameters back
    /// inside the range that keeps the swarm watchable.
    fn random_small_change(&mut self) {
        self.rsc_call_depth += 1;
        if self.rsc_call_depth > 10 {
            self.rsc_call_depth -= 1;
            return;
        }

        match random() % 11 {
            0 => mutate_param(&mut self.max_acc),
            1 => mutate_param(&mut self.target_acc),
            2 => mutate_param(&mut self.max_vel),
            3 => mutate_param(&mut self.target_vel),
            4 => mutate_param(&mut self.noise),
            5 => mutate_param(&mut self.min_vel_multiplier),
            6 | 7 => {
                if self.ntargets >= 2 {
                    self.mutate_bug(true);
                }
            }
            8 => {
                if self.nbugs >= 2 {
                    self.mutate_bug(false);
                    if self.nbugs >= 2 {
                        self.mutate_bug(false);
                    }
                }
            }
            9 => {
                self.color_scheme = (random() % NUM_SCHEMES as u32) as i32;
                if self.color_scheme == RANDOM_SCHIZO || self.color_scheme == COLOR_SCHIZO {
                    // Draw these two less often than the rest.
                    self.color_scheme = (random() % NUM_SCHEMES as u32) as i32;
                }
            }
            _ => {
                for _ in 0..4 {
                    self.random_small_change();
                }
            }
        }

        self.min_vel_multiplier = self.min_vel_multiplier.clamp(0.3, 0.9);
        self.noise = self.noise.max(0.01);
        self.max_vel = self.max_vel.max(0.02);
        self.target_vel = self.target_vel.max(0.02);
        self.target_acc = self.target_acc.min(self.target_vel * 0.7);
        self.max_acc = self.max_acc.min(self.max_vel * 0.7);
        self.target_acc = self.target_acc.min(self.target_vel * 0.7);
        self.max_acc = self.max_acc.max(0.01);
        self.target_acc = self.target_acc.max(0.005);

        self.compute_constants();
        self.rsc_call_depth -= 1;
    }

    /// `randomBigChange`: throw the picture away and start again.
    fn random_big_change(&mut self, d: &mut Dpy) {
        self.rbc_call_depth += 1;
        if self.rbc_call_depth > 3 {
            self.rbc_call_depth -= 1;
            return;
        }

        // Upstream's fifth case, which nudges one target sideways, sits in the
        // `default:` arm of a switch on a value that cannot reach it.
        match random() % 4 {
            0 => {
                let temp = rand_mod(MAX_TRAIL_LEN - 25) + 25;
                self.clear_bugs(d);
                self.trail_len = temp;
                self.compute_color_indices();
                self.init_bugs(false);
            }
            1 => {
                for _ in 0..8 {
                    self.random_small_change();
                }
            }
            2 => {
                self.clear_bugs(d);
                self.init_bugs(false);
            }
            _ => self.pick_new_targets(),
        }

        self.rbc_call_depth -= 1;
    }

    /// `updateColorIndex`: which fade table each population gets, and where in
    /// it to start. The schizo schemes start at the ring head, so the colours
    /// crawl along a trail from one frame to the next.
    fn update_color_index(&self) -> Scheme {
        let n = self.trail_len;
        let h = self.head;
        let (target, bug, start) = match self.color_scheme {
            GRAY_SCHIZO => (self.gray_s_index, self.gray_s_index, h),
            COLOR_SCHIZO => (self.red_s_index, self.blue_s_index, h),
            GRAY_TRAILS => (self.gray_index, self.gray_index, 0),
            RANDOM_TRAILS => (self.red_index, self.random_index, 0),
            RANDOM_SCHIZO => (self.red_index, self.random_index, h),
            // COLOR_TRAILS, which is the scheme it starts on.
            _ => (self.red_index, self.blue_index, 0),
        };
        Scheme {
            target,
            bug,
            start,
            len: n,
        }
    }
}

/// Which fade table each population draws with this frame, and where in it to
/// start. Both populations always use the same starting offset and length.
struct Scheme {
    target: [usize; MAX_TRAIL_LEN],
    bug: [usize; MAX_TRAIL_LEN],
    start: usize,
    len: usize,
}

/// The walls, which every bug bounces off.
fn bounce(b: &mut Bug, maxx: f32, maxy: f32) {
    if b.pos[0] < 0.0 {
        b.pos[0] = -b.pos[0];
        b.vel[0] = -b.vel[0];
    } else if b.pos[0] >= maxx {
        b.pos[0] = 2.0 * maxx - b.pos[0];
        b.vel[0] = -b.vel[0];
    }
    if b.pos[1] < 0.0 {
        b.pos[1] = -b.pos[1];
        b.vel[1] = -b.vel[1];
    } else if b.pos[1] >= maxy {
        b.pos[1] = 2.0 * maxy - b.pos[1];
        b.vel[1] = -b.vel[1];
    }
}

/// `mutateParam`: scale by up to a quarter either way.
fn mutate_param(param: &mut f32) {
    *param *= 0.75 + frandf(0.5);
}

impl Screenhack for XraySwarm {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // Two half steps per frame when there is a delay to fill, one whole one
        // when there is not. Upstream sets the step size without recomputing
        // the constants derived from it, so until the first mutation calls for
        // them the acceleration correction uses the startup step size.
        let cnt = if self.delay > 0 {
            self.dt = DESIRED_DT / 2.0;
            2
        } else {
            self.dt = DESIRED_DT;
            1
        };

        for _ in 0..cnt {
            self.update_state();
            let scheme = self.update_color_index();
            self.draw_bugs(d, &scheme);
        }

        if frandf(1.0) < self.change_prob * 2.0 / 100.0 {
            self.random_small_change();
        }
        if frandf(1.0) < self.change_prob * 0.3 / 100.0 {
            self.random_big_change(d);
        }

        // Short delays are accumulated and spent in one go rather than asking
        // to be woken up more often than the frame loop can manage.
        let mut this_delay = self.delay;
        if self.delay <= 10000 {
            self.delay_accum += self.delay;
            if self.delay_accum > 10000 {
                this_delay = self.delay_accum;
                self.delay_accum = 0;
                self.sleep_count = 0;
            }
            self.sleep_count += 1;
            if self.sleep_count > 2 {
                self.sleep_count = 0;
                this_delay = 10000;
            }
        }
        this_delay.max(0) as u32
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.xsize = width;
        self.ysize = height;
        self.maxy = height as f32 / width as f32;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(XraySwarm::new(d))
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    "*delay: 20000",
    "*fpsSolid: true",
    "*ignoreRotation: True",
];

const OPTS: &[Opt] =
    &[Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted()];

pub static DEF: SaverDef = SaverDef {
    slug: "xrayswarm",
    label: "XRaySwarm",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Chris Leger",
        year: "2000",
        video: Some("https://www.youtube.com/watch?v=e_E-k37b4Vc"),
        blurb: "Worm-like swarms of particles with vapor trails.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
