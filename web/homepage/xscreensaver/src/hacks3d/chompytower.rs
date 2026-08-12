//! Port of `hacks/glx/chompytower.c`.
//!
//! ```text
//! chompytower, Copyright © 2022-2025 Jamie Zawinski <jwz@jwz.org>
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
//! This tree's got teeth.
//!
//! The trunk is a stack of slices, and each slice is one frame of the blob
//! from `goop`: a closed spline round a handful of control points whose radii
//! wander in and out. A new slice is grown on top every tick and the oldest is
//! dropped off the bottom, so the tower is an extrusion of an animation, and
//! what you see going up it is the history of the blob's wobble.
//!
//! The mouths are not cut into the trunk. There is no way to subtract one
//! solid from another in OpenGL, so upstream fakes it: the teeth are drawn
//! first, then an invisible ellipsoid is drawn into the depth buffer alone, and
//! then the trunk is drawn, which cannot paint over the depth the ellipsoid
//! laid down. The hole is not really there, but the teeth show through where it
//! would be.
//!
//! Two things are done differently. Upstream compiles the trunk into a display
//! list and rebuilds that list whenever a slice is added or dropped, which is
//! most frames; a list here is a recording rather than something on the card,
//! so it would be re-recorded as often as it was replayed, and the trunk is
//! drawn directly instead. And the colour of a slice reaches the trunk through
//! the vertex colours that upstream sets beside its per-vertex material calls,
//! since a batch here carries one material: the values are the same ones.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_color_ramp, rgb_to_hsv};
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gl::{Glx, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::rotator::Rotator;
use crate::runtime::shapes::{calc_normal, unit_sphere};
use crate::runtime::spline::Spline;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

const JAW_UPPER_HALF: usize = 0;
const JAW_LOWER_HALF: usize = 1;
const TEETH_UPPER_HALF: usize = 2;
const TEETH_LOWER_HALF: usize = 3;
const NPARTS: usize = 4;

const MODELS: [&str; NPARTS] = [
    crate::models::TEETH_MODEL_JAW_UPPER_HALF,
    crate::models::TEETH_MODEL_JAW_LOWER_HALF,
    crate::models::TEETH_MODEL_TEETH_UPPER_HALF,
    crate::models::TEETH_MODEL_TEETH_LOWER_HALF,
];

const SPLINE_SCALE: f32 = 1000.0;
const FUNHOLE_HEIGHT: f32 = 0.2;

fn randsign() -> f32 {
    if random() & 1 == 1 { 1.0 } else { -1.0 }
}

/// From `goop.c`, which upstream wrote twenty-five years earlier. We use all
/// parts of the buffalo.
struct Blob {
    x: f32,
    y: f32,
    th: f64,
    elasticity: f32,
    min_r: f32,
    max_r: f32,
    npoints: usize,
    /// The radius of each control point. Its sign is the direction the radius
    /// is currently travelling in.
    r: Vec<f32>,
    spline: Spline,
}

impl Blob {
    fn new(resolution: f32) -> Blob {
        let size = 1.0;
        let ss = 0.2 / resolution;
        let max_r = size / 2.0;
        let min_r = (size / 10.0f32).max(0.1);
        let mid = (min_r + max_r) / 2.0;
        let npoints = (random() % 5) as usize + 5;
        let r = (0..npoints)
            .map(|_| ((random() as f32 % mid) + mid / 2.0) * randsign())
            .collect();
        Blob {
            x: 0.0,
            y: 0.0,
            th: frand(std::f64::consts::PI * 2.0) * randsign() as f64,
            elasticity: ss * 0.09,
            min_r,
            max_r,
            npoints,
            r,
            spline: Spline::new(npoints),
        }
    }

    /// One frame of the wobble: every control point moves in or out a little,
    /// and turns round when it has gone as far as it may.
    fn throb(&mut self) {
        let frac = (std::f64::consts::PI * 2.0) / self.npoints as f64;
        for i in 0..self.npoints {
            let mut r = self.r[i];
            let mut ra = r.abs();
            let th = self.th.abs();

            // The control points sit evenly round the perimeter, turned by
            // theta.
            let x = self.x + ra * (i as f64 * frac + th).cos() as f32;
            let y = self.y + ra * (i as f64 * frac + th).sin() as f32;
            self.spline.control_x[i] = (x * SPLINE_SCALE) as f64;
            self.spline.control_y[i] = (y * SPLINE_SCALE) as f64;

            // Alter the radius by a random amount, in the direction in which
            // it had been going.
            ra += frand(self.elasticity as f64) as f32 * if r > 0.0 { 1.0 } else { -1.0 };
            r = ra * if r >= 0.0 { 1.0 } else { -1.0 };

            // Reverse at either end of the range, and once every fifty times
            // in mid-course for no reason at all.
            if (ra > self.max_r && r >= 0.0)
                || (ra < self.min_r && r < 0.0)
                || random().is_multiple_of(50)
            {
                r = -r;
            }
            self.r[i] = r;
        }
        self.spline.compute_closed();
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Dead,
    Hidden,
    Extending,
    Opening,
    Open,
    Closing,
    Closed,
    Retracting,
}

#[derive(Clone, Copy)]
struct Funhole {
    state: State,
    ratio: f32,
    speed: f32,
    pos: usize,
}

/// One ring of the trunk.
struct Slice {
    points: Vec<[f32; 3]>,
    fnormals: Vec<[f32; 3]>,
    vnormals: Vec<[f32; 3]>,
    r: f32,
    z: f32,
    color: [f32; 4],
    funhole: Funhole,
}

impl Slice {
    /// Every slice has to have the same number of points or they cannot be
    /// stacked, so the spline's output is resampled to a fixed count.
    fn new(b: &Blob, r: f32, resolution: f32) -> Slice {
        let n0 = b.spline.points.len();
        let n1 = (40.0 * resolution) as usize;
        let p0 = &b.spline.points;
        let mut points = Vec::with_capacity(n1);
        for i1 in 0..n1 {
            let ratio = i1 as f64 / n1 as f64;
            let i0 = ratio * n0 as f64;
            let i0a = i0 as usize;
            let i0b = (i0a + 1) % n0;
            let r1 = if n0 > n1 { 0.0 } else { i0 % 1.0 };
            points.push([
                r * (p0[i0a].x as f64 + r1 * (p0[i0b].x - p0[i0a].x) as f64) as f32 / SPLINE_SCALE,
                r * (p0[i0a].y as f64 + r1 * (p0[i0b].y - p0[i0a].y) as f64) as f32 / SPLINE_SCALE,
                0.0,
            ]);
        }
        Slice {
            fnormals: vec![[0.0; 3]; n1],
            vnormals: vec![[0.0; 3]; n1],
            points,
            r,
            z: 0.0,
            color: [0.0; 4],
            funhole: Funhole {
                state: State::Dead,
                ratio: 0.0,
                speed: 0.0,
                pos: 0,
            },
        }
    }
}

fn normalize(p: [f32; 3]) -> [f32; 3] {
    let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    if d < 0.0000001 {
        [0.0; 3]
    } else {
        [p[0] / d, p[1] / d, p[2] / d]
    }
}

struct Branch {
    pos: [f32; 3],
    blob: Blob,
    slices: Vec<Slice>,
    max_slices: usize,
    slice_height: f32,
    colors: Vec<XColor>,
    ccolor: f32,
}

struct ChompyTower {
    trackball: Trackball,
    rot: Rotator,
    rot2: Rotator,
    branches: Vec<Branch>,
    last_tick: f64,
    colors: [[f32; 4]; NPARTS],
    lists: [u32; NPARTS],
    sphere: u32,
    aspect: f32,
    scale: f32,
    speed: f32,
    resolution: f32,
    spin: bool,
    wander: bool,
    tilt: bool,
    smooth: bool,
    wire: bool,
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

impl Branch {
    fn new(g: &Gl, resolution: f32) -> Branch {
        let c = resource_color(g, "towerColor");
        let (h1, s1, v1) = rgb_to_hsv(
            (c[0] * 65535.0) as u16,
            (c[1] * 65535.0) as u16,
            (c[2] * 65535.0) as u16,
        );
        // The ramp runs from the tower's colour to a much darker or much
        // lighter version of it and back, so the trunk is banded.
        let v2 = (v1 * if v1 > 0.5 { 0.25 } else { 2.0 }).min(1.0);
        Branch {
            pos: [0.0; 3],
            blob: Blob::new(resolution),
            slices: Vec::new(),
            max_slices: (500.0 * resolution) as usize,
            slice_height: 0.02 / resolution,
            colors: make_color_ramp(h1, s1, v1, h1, s1, v2, 64, true),
            ccolor: 0.0,
        }
    }

    /// Add a slice on top: throb the blob, resample it, work out the normals
    /// of the quads joining it to the one below, and now and then start a new
    /// mouth on it.
    fn grow(&mut self, resolution: f32, speed: f32) {
        let step = 0.01 / resolution;
        let mut r = 0.0;
        if let Some(last) = self.slices.last() {
            r = (last.r + step).min(1.0);
        }

        self.blob.throb();
        let mut s0 = Slice::new(&self.blob, r, resolution);
        s0.z = self
            .slices
            .last()
            .map(|s| s.z + self.slice_height)
            .unwrap_or(0.0);

        // Face normals for this slice, pointing down at the one below.
        let n = s0.points.len();
        for i in 0..n {
            let j = (i + 1) % n;
            let o = self.slices.last();
            let mut a = s0.points[i];
            let mut b = s0.points[j];
            let mut c = o.map(|o| o.points[i]).unwrap_or(a);
            let d = o.map(|o| o.points[j]).unwrap_or(b);
            a[2] += s0.z;
            b[2] += s0.z;
            c[2] += o.map(|o| o.z).unwrap_or(s0.z);

            let n1 = normalize(calc_normal(a, c, b));
            let n2 = normalize(calc_normal(c, d, b));
            // These quads are not planes, they twist, so take the average of
            // the two triangles they could be cut into.
            s0.fnormals[i] = [
                (n1[0] + n2[0]) / 2.0,
                (n1[1] + n2[1]) / 2.0,
                (n1[2] + n2[2]) / 2.0,
            ];
        }

        s0.color = [
            self.colors[self.ccolor as usize].red as f32 / 65536.0,
            self.colors[self.ccolor as usize].green as f32 / 65536.0,
            self.colors[self.ccolor as usize].blue as f32 / 65536.0,
            1.0,
        ];
        self.ccolor += 1.0 / speed;
        if self.ccolor >= self.colors.len() as f32 {
            self.ccolor = 0.0;
        }

        self.slices.push(s0);

        // A vertex normal is the average of the four faces it belongs to, so
        // the slice below this one can be finished now that this one exists.
        if self.slices.len() > 2 {
            let k = self.slices.len();
            for i in 0..n {
                let j = if i < n - 1 { i + 1 } else { 0 };
                let n1 = self.slices[k - 1].fnormals[i];
                let n2 = self.slices[k - 1].fnormals[j];
                let n3 = self.slices[k - 3].fnormals[i];
                let n4 = self.slices[k - 3].fnormals[j];
                self.slices[k - 2].vnormals[i] = [
                    (n1[0] + n2[0] + n3[0] + n4[0]) / 4.0,
                    (n1[1] + n2[1] + n3[1] + n4[1]) / 4.0,
                    (n1[2] + n2[2] + n3[2] + n4[2]) / 4.0,
                ];
            }
        }

        // A new mouth, but not too close to the last one, and not where the
        // trunk is too thin to hold it.
        let mut last_funhole_y = 0.0;
        for s in &self.slices {
            if s.funhole.state != State::Dead {
                last_funhole_y = s.z;
            }
        }
        let s0 = self.slices.last_mut().expect("just pushed");
        if s0.z > last_funhole_y + FUNHOLE_HEIGHT * 1.3
            && (last_funhole_y == 0.0 || random().is_multiple_of(10))
        {
            let min_dist = 0.3;
            let pos = random() as usize % s0.points.len();
            let p = s0.points[pos];
            let dist2 = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
            if dist2 >= min_dist * min_dist {
                s0.funhole = Funhole {
                    state: State::Hidden,
                    pos,
                    ratio: 0.0,
                    speed: 0.04 + frand(0.01) as f32,
                };
            }
        }
    }
}

/// One step of a mouth's cycle. It rises out of the trunk, opens and closes a
/// few times, and sinks back in.
fn tick_funhole(s: &mut Slice, speed: f32) {
    if s.funhole.state == State::Dead {
        return;
    }
    s.funhole.ratio += s.funhole.speed;
    if s.funhole.ratio <= 1.0 {
        return;
    }
    s.funhole.ratio = 0.0;
    s.funhole.speed = (0.05 + frand(0.01) as f32) * speed;

    let f = &mut s.funhole;
    match f.state {
        State::Hidden => {
            f.state = State::Extending;
            f.speed *= 0.2;
        }
        State::Extending => {
            f.state = State::Closed;
            f.speed *= 1.0 + frand(4.0) as f32;
        }
        State::Closed => {
            if random().is_multiple_of(20) {
                f.state = State::Retracting;
                f.speed *= 0.2;
            } else {
                f.state = State::Opening;
                f.speed *= 3.0;
            }
        }
        State::Opening => {
            f.state = State::Open;
            if random().is_multiple_of(6) {
                f.speed *= 0.3;
                f.speed *= 1.0 + frand(3.0) as f32;
            } else {
                f.speed *= 100.0;
            }
        }
        State::Open => {
            f.state = State::Closing;
            f.speed *= 3.0;
        }
        State::Closing => {
            f.state = State::Closed;
            if random().is_multiple_of(6) {
                f.speed *= 0.3;
                f.speed *= 1.0 + frand(4.0) as f32;
            } else {
                f.speed *= 100.0;
            }
        }
        State::Retracting => {
            f.state = State::Hidden;
            f.speed *= 1.0 + frand(4.0) as f32;
        }
        State::Dead => {}
    }
}

impl ChompyTower {
    /// Half a mouth, and the same half mirrored.
    fn draw_component(&self, g: &mut Glx, i: usize) {
        g.material_ambient_diffuse(self.colors[i]);
        g.material_specular([0.4, 0.4, 0.4, 1.0]);
        g.material_shininess(80.0);
        g.front_face_cw(false);
        g.call_list(self.lists[i]);
        g.push_matrix();
        g.scale(-1.0, 1.0, 1.0);
        g.front_face_cw(true);
        g.call_list(self.lists[i]);
        g.pop_matrix();
        g.front_face_cw(false);
    }

    /// A mouth standing out of the trunk, or the ellipsoid that masks the hole
    /// it stands in.
    fn draw_funhole(&self, g: &mut Glx, s: &Slice, shadow: bool) {
        if s.funhole.state == State::Dead {
            return;
        }
        let max_tilt = 20.0;
        let hole_aspect = [0.5, 0.225, 0.9];
        let p = s.points[s.funhole.pos];
        let odist = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        let mut dist = odist;
        let mut tilt = 0.0;

        g.push_matrix();
        g.translate(0.0, 0.0, p[2] + s.z);
        let (x, y, z) = (p[0], p[1], p[2]);
        g.rotate(-x.atan2(y) * (180.0 / std::f32::consts::PI), 0.0, 0.0, 1.0);
        g.rotate(
            z.atan2((x * x + y * y).sqrt()) * (180.0 / std::f32::consts::PI),
            1.0,
            0.0,
            0.0,
        );

        let r = s.funhole.ratio as f64;
        match s.funhole.state {
            State::Hidden => dist = 0.0,
            State::Extending => dist *= ease(Ease::InOutSine, r) as f32,
            State::Retracting => dist *= ease(Ease::InOutSine, 1.0 - r) as f32,
            State::Opening => tilt = ease(Ease::InOutSine, r) as f32,
            State::Closing => tilt = ease(Ease::InOutSine, 1.0 - r) as f32,
            State::Open => tilt = 1.0,
            State::Closed | State::Dead => tilt = 0.0,
        }
        dist *= 0.4;

        if shadow {
            g.front_face_cw(false);
            g.push_matrix();
            g.translate(0.0, odist, 0.0);
            g.scale(
                hole_aspect[0] * FUNHOLE_HEIGHT,
                hole_aspect[1] * FUNHOLE_HEIGHT,
                hole_aspect[2] * FUNHOLE_HEIGHT,
            );
            g.call_list(self.sphere);
            g.pop_matrix();
        } else {
            g.translate(0.0, dist, 0.0);
            g.scale(FUNHOLE_HEIGHT, FUNHOLE_HEIGHT, FUNHOLE_HEIGHT);
            g.rotate(-90.0, 1.0, 0.0, 0.0);
            g.rotate(tilt * -max_tilt, 1.0, 0.0, 0.0);
            self.draw_component(g, TEETH_UPPER_HALF);
            g.rotate(tilt * max_tilt * 2.0, 1.0, 0.0, 0.0);
            self.draw_component(g, TEETH_LOWER_HALF);
        }
        g.pop_matrix();
    }

    /// The trunk: a quad strip between each pair of slices.
    fn draw_branch(&self, g: &mut Glx, b: &Branch) {
        g.material_specular([0.4, 0.4, 0.4, 1.0]);
        g.material_shininess(20.0);
        // The colour of a slice arrives as a vertex colour, which is how
        // upstream's per-vertex material calls come out here.
        g.color_material(!self.wire);

        for i in 1..b.slices.len() {
            let s1 = &b.slices[i];
            let s2 = &b.slices[i - 1];
            if !self.wire {
                g.begin(if self.smooth {
                    Shape::QuadStrip
                } else {
                    Shape::Quads
                });
            }
            let n = s1.points.len();
            for j in 0..=n {
                let jj = j % n;
                let kk = (j + 1) % n;
                let mut pa = s1.points[jj];
                let mut pb = s2.points[jj];
                let mut pc = s1.points[kk];
                let mut pd = s2.points[kk];
                let na = if self.smooth {
                    s1.vnormals[jj]
                } else {
                    s1.fnormals[jj]
                };
                let nb = if self.smooth {
                    s2.vnormals[jj]
                } else {
                    s1.fnormals[jj]
                };
                let nc = if self.smooth {
                    s1.vnormals[kk]
                } else {
                    s1.fnormals[jj]
                };
                let nd = if self.smooth {
                    s2.vnormals[kk]
                } else {
                    s1.fnormals[jj]
                };
                pa[2] += s1.z;
                pb[2] += s2.z;
                pc[2] += s1.z;
                pd[2] += s2.z;

                if self.wire {
                    g.begin(Shape::LineLoop);
                }
                let col = |g: &mut Glx, s: &Slice| {
                    if !self.wire {
                        g.color4f(s.color[0], s.color[1], s.color[2], s.color[3]);
                    }
                };
                col(g, s1);
                g.normal3f(na[0], na[1], na[2]);
                g.vertex3f(pa[0], pa[1], pa[2]);
                col(g, s2);
                g.normal3f(nb[0], nb[1], nb[2]);
                g.vertex3f(pb[0], pb[1], pb[2]);
                if self.wire || !self.smooth {
                    g.normal3f(nd[0], nd[1], nd[2]);
                    g.vertex3f(pd[0], pd[1], pd[2]);
                    col(g, s1);
                    g.normal3f(nc[0], nc[1], nc[2]);
                    g.vertex3f(pc[0], pc[1], pc[2]);
                }
                if self.wire {
                    g.end();
                }
            }
            if !self.wire {
                g.end();
            }
        }
        g.color_material(false);
    }

    /// Everything moves down by a step, the bottom slice falls off the end and
    /// a new one grows on top.
    fn tick(&mut self) {
        let step = 0.01 * self.speed;
        let (resolution, speed) = (self.resolution, self.speed);
        for b in &mut self.branches {
            let min_z = -(b.max_slices as f32 * b.slice_height * 0.35);
            b.pos[2] -= step;
            if b.pos[2] < min_z && !b.slices.is_empty() {
                b.slices.remove(0);
                for s in &mut b.slices {
                    s.z -= b.slice_height;
                }
                b.pos[2] += b.slice_height;
            }
            if b.slices.len() < b.max_slices {
                b.grow(resolution, speed);
            }
            for s in &mut b.slices {
                tick_funhole(s, speed);
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let resolution = g.res.float("resolution") as f32;

    let mut lists = [0u32; NPARTS];
    let mut colors = [[1.0f32; 4]; NPARTS];
    for i in 0..NPARTS {
        colors[i] = resource_color(
            g,
            match i {
                JAW_UPPER_HALF | JAW_LOWER_HALF => "jawColor",
                _ => "teethColor",
            },
        );
        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        g.glx.push_matrix();
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        GlList::parse(MODELS[i]).render(&mut g.glx, wire);
        g.glx.pop_matrix();
        g.glx.end_list();
        lists[i] = list;
    }

    let sphere = g.glx.gen_lists(1);
    g.glx.new_list(sphere);
    unit_sphere(&mut g.glx, 16, 32, wire);
    g.glx.end_list();

    let spin = g.res.bool("spin");
    let wander = g.res.bool("wander");
    let tilt = g.res.bool("tilt");
    let mut this = ChompyTower {
        trackball: Trackball::new(),
        rot: Rotator::new(
            0.0,
            0.0,
            if spin { 0.3 } else { 0.0 },
            0.5,
            if wander { 0.005 } else { 0.0 },
            true,
        ),
        rot2: Rotator::new(0.0, 0.0, 0.0, 0.0, if tilt { 0.01 } else { 0.0 }, true),
        branches: Vec::new(),
        last_tick: 0.0,
        colors,
        lists,
        sphere,
        aspect: 1.0,
        scale: 1.0,
        speed: g.res.float("speed") as f32,
        resolution,
        spin,
        wander,
        tilt,
        smooth: g.res.bool("smooth"),
        wire,
    };
    let branch = Branch::new(g, resolution);
    this.branches.push(branch);
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for ChompyTower {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = width as f32 / height as f32;
        self.scale = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, self.aspect, 1.0, 500.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wire {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 4.0, 1.4, 1.1, 0.0);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [0.5, 0.5, 0.5, 1.0]);
        } else {
            g.glx.lighting(false);
        }

        g.glx.push_matrix();
        let turning = !self.trackball.button_down();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        if self.wander {
            let (x, y, z) = self.rot.position(turning);
            g.glx.translate(
                (x as f32 - 0.5) * 4.0,
                (y as f32 - 0.5) * 0.2,
                (z as f32 - 0.5) * 8.0,
            );
        }
        if self.tilt {
            let maxz = 50.0;
            let (_, _, z) = self.rot2.position(turning);
            g.glx.rotate(maxz / 2.0 - z as f32 * maxz, 1.0, 0.0, 0.0);
        }
        if self.spin {
            let (_, _, z) = self.rot.rotation(turning);
            g.glx.rotate(z as f32 * 360.0, 0.0, 1.0, 0.0);
        }

        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.translate(0.0, 0.0, 20.0);
        g.glx.scale(15.0, 15.0, 15.0);

        for i in 0..self.branches.len() {
            g.glx.push_matrix();
            let pos = self.branches[i].pos;
            g.glx.translate(pos[0], pos[1], pos[2]);

            // Carving a hole out of the trunk is not something OpenGL can do,
            // since it has no notion of a solid, only of its surface. So the
            // teeth go down first, then an invisible ellipsoid fills the depth
            // buffer where the hole would be, and the trunk cannot paint over
            // it.
            let glx = &mut g.glx;
            let b = &self.branches[i];
            for s in &b.slices {
                self.draw_funhole(glx, s, false);
            }
            if !self.wire {
                glx.color_mask(false);
            }
            for s in &b.slices {
                self.draw_funhole(glx, s, true);
            }
            glx.color_mask(true);
            glx.front_face_cw(false);
            self.draw_branch(glx, b);

            g.glx.pop_matrix();
        }
        g.glx.pop_matrix();

        if !self.trackball.button_down()
            && g.time > self.last_tick + (1.0 / 30.0) / self.speed as f64
        {
            self.tick();
            self.last_tick = g.time;
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       30000",
    "*showFPS:     False",
    "*wireframe:   False",
    "*towerColor:  #eE9752",
    "*teethColor:  #FFFF88",
    "*jawColor:    #eE9752",
    "*speed:       1.0",
    "*spin:        True",
    "*wander:      False",
    "*tilt:        True",
    "*smooth:      True",
    "*resolution:  1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Scrolling speed", 0.01, 8.0, 0.01, 2, "1.0"),
    Opt::slider("resolution", "Resolution", 0.1, 4.0, 0.1, 1, "1.0"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("tilt", "Tilt", "true"),
    Opt::boolean("smooth", "Smooth", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "chompytower",
    label: "Chompy Tower",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2022",
        video: Some("https://www.youtube.com/watch?v=pQh_hLUKPao"),
        blurb: "This tree's got teeth.",
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

    /// The trunk grows a slice at a time and drops one off the bottom, and
    /// every slice has the same number of points or they could not stack.
    #[test]
    fn the_trunk_is_an_extruded_animation() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..50 {
            r.step();
        }
        let f = r.frame();
        assert!(!f.batches.is_empty(), "nothing was drawn");
        assert!(
            f.vertices
                .iter()
                .all(|v| v.pos.iter().all(|c| c.is_finite())),
            "a vertex went to NaN"
        );
        // Forty points to a slice at the default resolution.
        let mut blob = Blob::new(1.0);
        blob.throb();
        let s = Slice::new(&blob, 0.5, 1.0);
        assert_eq!(s.points.len(), 40);
        assert!(
            s.points
                .iter()
                .all(|p| p[0].abs() < 1.0 && p[1].abs() < 1.0),
            "a slice is bigger than the blob it came from"
        );
    }

    /// The blob's control points wander in and out and turn round at the ends
    /// of their range, so the trunk is never a cylinder and never a knot.
    #[test]
    fn the_blob_wobbles_within_its_range() {
        crate::runtime::rand::ya_rand_init(20260811);
        let mut b = Blob::new(1.0);
        for _ in 0..2000 {
            b.throb();
            for r in &b.r {
                assert!(
                    r.abs() < b.max_r + b.elasticity * 2.0,
                    "a control point ran away: {r}"
                );
            }
        }
    }

    /// A mouth rises out of the trunk, chomps a few times and sinks back,
    /// which is the whole of its cycle.
    #[test]
    fn a_mouth_goes_round_its_cycle() {
        crate::runtime::rand::ya_rand_init(20260811);
        let mut blob = Blob::new(1.0);
        blob.throb();
        let mut s = Slice::new(&blob, 0.5, 1.0);
        s.funhole = Funhole {
            state: State::Hidden,
            ratio: 0.0,
            speed: 0.04,
            pos: 0,
        };
        let mut seen = std::collections::HashSet::new();
        for _ in 0..20000 {
            tick_funhole(&mut s, 1.0);
            seen.insert(format!("{:?}", s.funhole.state));
        }
        for want in ["Extending", "Closed", "Opening", "Open", "Closing"] {
            assert!(seen.contains(want), "a mouth never got to {want}");
        }
    }

    /// The hole is not really a hole: the teeth are drawn, then something
    /// invisible fills the depth buffer, and then the trunk goes over it.
    #[test]
    fn the_hole_is_a_depth_mask() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        // Long enough for the first mouth to be well clear of the ground.
        for _ in 0..400 {
            r.step();
        }
        let f = r.frame();
        let masked = f
            .batches
            .iter()
            .filter(|b| b.color_mask != [true; 4])
            .count();
        assert!(masked > 0, "nothing was drawn into the depth buffer alone");
        // And the trunk is drawn after it, with the colour mask back on.
        let last = f.batches.last().expect("no batches");
        assert_eq!(last.color_mask, [true; 4], "the trunk is invisible");
    }
}
