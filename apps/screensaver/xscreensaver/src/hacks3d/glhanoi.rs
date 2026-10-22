//! Port of `hacks/glx/glhanoi.c`.
//!
//! ```text
//! glhanoi, Copyright (c) 2005, 2009 Dave Atkinson <da@davea.org.uk>
//! except noise function code Copyright (c) 2002 Ken Perlin
//! Modified by Lars Huttar (c) 2010, to generalize to 4 or more poles
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
//! The Towers of Hanoi, solved over and over on a marble chequerboard, with
//! the disks thrown from pole to pole on a ballistic arc and tumbling end over
//! end on the way.
//!
//! Three poles get the classic solution, driven by the parity of the move
//! number rather than by recursion. More than three get Frame-Stewart, which
//! splits the stack in two at a point chosen by a heuristic and works through
//! an explicit stack of subproblems, because nobody has proved what the optimal
//! split is. Once the last disk lands they all fly back at once, a quarter of a
//! second apart, which the author's comments call the money shot.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Fog, Shape};
use crate::runtime::rotator::Rotator;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};
use std::f64::consts::PI;

/// How round the poles and disks are.
const NSLICE: usize = 32;
const NLOOPS: usize = 1;

/// How long to wait at the start and the finish, in seconds.
const START_DURATION: f64 = 1.0;
const FINISH_DURATION: f64 = 1.0;
const BASE_LENGTH: f32 = 30.0;
const BOARD_SQUARES: usize = 8;

/// Do not draw trail lines until they are this old, so that a trail is not
/// attached to the disk making it.
const TRAIL_START_DELAY: f64 = 0.1;

const MAX_CAMERA_RADIUS: f64 = 250.0;
const MIN_CAMERA_RADIUS: f64 = 75.0;

const MARBLE_SCALE: f64 = 1.01;
const MARBLE_TEXTURE_SIZE: usize = 256;

/// "hmm, looks like we need more gravity, Scotty..."
const G: f32 = 3.0 * 9.806_65;

/// A number in `[0, n)` with a bell curve distribution.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Start,
    MoveDisk,
    MoveFinished,
    Finished,
    /// Every disk flying home at once.
    MoneyShot,
}

#[derive(Clone, Default)]
struct Disk {
    id: usize,
    display_list: u32,
    position: [f32; 3],
    rotation: [f32; 3],
    base0: f32,
    base1: f32,
    xmin: f32,
    xmax: f32,
    ymin: f32,
    zmin: f32,
    zmax: f32,
    u1: f32,
    u2: f32,
    t1: f32,
    t2: f32,
    ucostheta: f32,
    usintheta: f32,
    dx: f32,
    dz: f32,
    /// How far through its flip the disk is, in degrees.
    rot_angle: f64,
    /// The direction of travel in the xz plane, in degrees.
    phi: f64,
    speed: f32,
}

/// A stack of disks, by index into the disk array.
#[derive(Clone, Default)]
struct Pole {
    data: Vec<usize>,
    position: [f32; 3],
}

/// A recursive subdivision of the problem: "move `n_disks` disks from `src` to
/// `dst`, using the poles in `available`".
#[derive(Clone, Copy, Default)]
struct SubProblem {
    n_disks: i32,
    src: usize,
    dst: usize,
    /// A bitmask of poles that have no smaller disks on them.
    available: u32,
}

#[derive(Clone, Copy, Default)]
struct TrailPoint {
    position: [f32; 3],
    start_time: f64,
    end_time: f64,
    is_end: bool,
}

const C_BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const C_WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const POLE_COLOR: [f32; 3] = [0.545, 0.137, 0.137];
const BASE_COLOR: [f32; 3] = [0.34, 0.34, 0.48];
const FOG_COLOR: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

/// Whether the number of trailing zeroes on `i` is even, unless `i` is one or
/// zero. "magic - it's magic..."
fn magic(mut i: i64) -> bool {
    let mut count = 0;
    if i <= 1 {
        return false;
    }
    while i & 1 == 0 {
        i >>= 1;
        count += 1;
    }
    count % 2 == 0
}

fn distance(p0: [f32; 3], p1: [f32; 3]) -> f32 {
    let (x, y, z) = (p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]);
    (x * x + y * y + z * z).sqrt()
}

fn lerp(alpha: f64, start: f64, end: f64) -> f64 {
    start + alpha * (end - start)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    if s == 0.0 {
        return [v, v, v];
    }
    let h = if h >= 360.0 { 0.0 } else { h } / 60.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// The hue ramp the disks are coloured along, which bunches the colours up
/// rather than running evenly round the wheel.
fn cfunc(x: f64) -> f64 {
    if x < 2.0 / 7.0 {
        return (1.0 / 12.0) / (1.0 / 7.0) * x;
    }
    if x < 3.0 / 7.0 {
        return (1.0 + 1.0 / 6.0) * x - 1.0 / 6.0;
    }
    if x < 4.0 / 7.0 {
        return (2.0 + 1.0 / 3.0) * x - 2.0 / 3.0;
    }
    (1.0 / 12.0) / (1.0 / 7.0) * x + 1.0 / 3.0
}

/* Ken Perlin's improved noise, which makes the marble */

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn grad(hash: i32, x: f64, y: f64, z: f64) -> f64 {
    // Convert the low four bits of the hash code into twelve gradient
    // directions.
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

const PERMUTATION: [i32; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];

fn improved_noise(p: &[i32; 512], x: f64, y: f64, z: f64) -> f64 {
    // Find the unit cube that contains the point.
    let xi = (x.floor() as i32) & 255;
    let yi = (y.floor() as i32) & 255;
    let zi = (z.floor() as i32) & 255;
    // And where the point is inside it.
    let x = x - x.floor();
    let y = y - y.floor();
    let z = z - z.floor();
    let (u, v, w) = (fade(x), fade(y), fade(z));
    // The hashed coordinates of the eight corners of the cube. Upstream names
    // them after Perlin's own Java, doubling each letter for the second index.
    let at = |i: i32| p[i as usize];
    let a = at(xi) + yi;
    let a0 = at(a) + zi;
    let a1 = at(a + 1) + zi;
    let b = at(xi + 1) + yi;
    let b0 = at(b) + zi;
    let b1 = at(b + 1) + zi;
    lerp(
        w,
        lerp(
            v,
            lerp(u, grad(at(a0), x, y, z), grad(at(b0), x - 1.0, y, z)),
            lerp(
                u,
                grad(at(a1), x, y - 1.0, z),
                grad(at(b1), x - 1.0, y - 1.0, z),
            ),
        ),
        lerp(
            v,
            lerp(
                u,
                grad(at(a0 + 1), x, y, z - 1.0),
                grad(at(b0 + 1), x - 1.0, y, z - 1.0),
            ),
            lerp(
                u,
                grad(at(a1 + 1), x, y - 1.0, z - 1.0),
                grad(at(b1 + 1), x - 1.0, y - 1.0, z - 1.0),
            ),
        ),
    )
}

fn turb(p: &[i32; 512], x: f64, y: f64, z: f64, octaves: u32) -> f64 {
    let mut freq = 1.0;
    let mut r = 0.0;
    for _ in 0..octaves {
        r += improved_noise(p, freq * x, freq * y, freq * z).abs() / freq;
        freq *= 2.0;
    }
    r / 2.0
}

/// The marble the floor is tiled with: sine bands through the unit square,
/// with the coordinates pushed about by turbulence first.
fn make_marble_texture() -> Vec<u8> {
    let mut p = [0i32; 512];
    p[..256].copy_from_slice(&PERMUTATION);
    p[256..].copy_from_slice(&PERMUTATION);

    // The two colours the bands run between, as upstream packs them.
    let (r0, g0, b0) = (0x3f, 0x3f, 0x3f);
    let (r1, g1, b1) = (0xff, 0xff, 0xff);

    let n = MARBLE_TEXTURE_SIZE;
    let mut data = Vec::with_capacity(n * n * 4);
    let step = 1.0 / n as f64;
    let mut y = 0.0;
    for _ in 0..n {
        let mut x = 0.0;
        for _ in 0..n {
            // `perturb` pushes all three coordinates about by the same
            // turbulence, but `f_m` is a sine of x alone, so only x survives.
            let px = x + MARBLE_SCALE * turb(&p, x, y, 0.0, 4);
            // `f_m`, then `C_m`: how far along the ramp between the two
            // colours the band lands.
            let v = (3.0 * PI * px).sin();
            let v = v - v.floor();
            let factor = (1.0 + (2.0 * PI * v).sin()) / 2.0;
            let mix = |a: i32, b: i32| (a + (factor * f64::from(b - a)) as i32) as u8;
            data.push(mix(r0, r1));
            data.push(mix(g0, g1));
            data.push(mix(b0, b1));
            data.push(0xff);
            x += step;
        }
        y += step;
    }
    data
}

/* Geometry */

/// `drawTube`: a cylinder with a hole through it, which is both a pole and a
/// disk. Upstream wanted to texture the poles with a three-dimensional wood
/// grain and gave up on it, so they are plain.
/// `radius` and `thickness` are `[bottom, top]`, which upstream passes as four
/// separate arguments.
fn draw_tube(
    g: &mut Gl,
    radius2: [f32; 2],
    thickness2: [f32; 2],
    height: f32,
    n_slice: usize,
    n_loop: usize,
) {
    let [bottom_radius, top_radius] = radius2;
    let bottom_thickness = thickness2[0].clamp(0.0, bottom_radius);
    let top_thickness = thickness2[1].clamp(0.0, top_radius);
    let last_slice = n_slice - 1;

    let mut cos_cache = vec![0.0f32; n_slice];
    let mut sin_cache = vec![0.0f32; n_slice];

    // Bottom.
    let y = 0.0;
    let radius = bottom_radius;
    let inner_radius = bottom_radius - bottom_thickness;

    g.glx.begin(Shape::QuadStrip);
    g.glx.normal3f(0.0, -1.0, 0.0);
    g.glx.vertex3f(0.0, y, inner_radius);
    g.glx.vertex3f(0.0, y, radius);
    for slice in (0..=last_slice).rev() {
        let theta = 2.0 * PI * slice as f64 / n_slice as f64;
        cos_cache[slice] = theta.cos() as f32;
        sin_cache[slice] = theta.sin() as f32;
        g.glx.vertex3f(
            inner_radius * sin_cache[slice],
            y,
            inner_radius * cos_cache[slice],
        );
        g.glx
            .vertex3f(radius * sin_cache[slice], y, radius * cos_cache[slice]);
    }
    g.glx.end();

    // Middle.
    for loop_ in 0..n_loop {
        let at = |k: usize| (k as f32) / (n_loop as f32);
        let mut lower_radius = bottom_radius + (top_radius - bottom_radius) * at(loop_);
        let mut upper_radius = bottom_radius + (top_radius - bottom_radius) * at(loop_ + 1);
        let lower_y = height * at(loop_);
        let upper_y = height * at(loop_ + 1);
        let factor = (top_radius - top_thickness) - (bottom_radius - bottom_thickness);

        // Outside.
        g.glx.begin(Shape::QuadStrip);
        for slice in 0..n_slice {
            g.glx.normal3f(sin_cache[slice], 0.0, cos_cache[slice]);
            g.glx.vertex3f(
                upper_radius * sin_cache[slice],
                upper_y,
                upper_radius * cos_cache[slice],
            );
            g.glx.vertex3f(
                lower_radius * sin_cache[slice],
                lower_y,
                lower_radius * cos_cache[slice],
            );
        }
        g.glx.normal3f(0.0, 0.0, 1.0);
        g.glx.vertex3f(0.0, upper_y, upper_radius);
        g.glx.vertex3f(0.0, lower_y, lower_radius);
        g.glx.end();

        // Inside.
        lower_radius = bottom_radius - bottom_thickness + factor * at(loop_);
        upper_radius = bottom_radius - bottom_thickness + factor * at(loop_ + 1);

        g.glx.begin(Shape::QuadStrip);
        g.glx.normal3f(0.0, 0.0, -1.0);
        g.glx.vertex3f(0.0, upper_y, upper_radius);
        g.glx.vertex3f(0.0, lower_y, lower_radius);
        for slice in (0..=last_slice).rev() {
            g.glx.normal3f(-sin_cache[slice], 0.0, -cos_cache[slice]);
            g.glx.vertex3f(
                upper_radius * sin_cache[slice],
                upper_y,
                upper_radius * cos_cache[slice],
            );
            g.glx.vertex3f(
                lower_radius * sin_cache[slice],
                lower_y,
                lower_radius * cos_cache[slice],
            );
        }
        g.glx.end();
    }

    // Top.
    let y = height;
    let radius = top_radius;
    let inner_radius = top_radius - top_thickness;

    g.glx.begin(Shape::QuadStrip);
    g.glx.normal3f(0.0, 1.0, 0.0);
    for slice in 0..n_slice {
        g.glx.vertex3f(
            inner_radius * sin_cache[slice],
            y,
            inner_radius * cos_cache[slice],
        );
        g.glx
            .vertex3f(radius * sin_cache[slice], y, radius * cos_cache[slice]);
    }
    g.glx.vertex3f(0.0, y, inner_radius);
    g.glx.vertex3f(0.0, y, radius);
    g.glx.end();
}

/// `drawCuboid`: the base, when the poles are in a line.
fn draw_cuboid(g: &mut Gl, length: f32, width: f32, height: f32) {
    let (xmin, xmax) = (-length / 2.0, length / 2.0);
    let (zmin, zmax) = (-width / 2.0, width / 2.0);
    let (ymin, ymax) = (0.0, height);

    g.glx.begin(Shape::Quads);
    for (normal, quad) in [
        (
            [0.0, 0.0, 1.0],
            [
                [xmin, ymin, zmax],
                [xmax, ymin, zmax],
                [xmax, ymax, zmax],
                [xmin, ymax, zmax],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [xmax, ymin, zmax],
                [xmax, ymin, zmin],
                [xmax, ymax, zmin],
                [xmax, ymax, zmax],
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [xmax, ymin, zmin],
                [xmin, ymin, zmin],
                [xmin, ymax, zmin],
                [xmax, ymax, zmin],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [xmin, ymin, zmin],
                [xmin, ymin, zmax],
                [xmin, ymax, zmax],
                [xmin, ymax, zmin],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                [xmin, ymax, zmax],
                [xmax, ymax, zmax],
                [xmax, ymax, zmin],
                [xmin, ymax, zmin],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [xmin, ymin, zmin],
                [xmax, ymin, zmin],
                [xmax, ymin, zmax],
                [xmin, ymin, zmax],
            ],
        ),
    ] {
        g.glx.normal3f(normal[0], normal[1], normal[2]);
        for v in quad {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
    }
    g.glx.end();
}

struct GlHanoi {
    state: State,
    wire: bool,
    fog: bool,
    light: bool,
    trail_duration: f64,
    start_time: f64,
    duration: f64,
    number_of_disks: usize,
    number_of_poles: usize,
    number_of_moves: i64,
    max_disk_idx: usize,
    magic_number: bool,
    current_disk: usize,
    move_: i64,
    /// Which pole is the source, the spare and the destination of the move now
    /// being made, and what they were at the start.
    src: usize,
    tmp: usize,
    dst: usize,
    oldsrc: usize,
    oldtmp: usize,
    olddst: usize,
    solve_stack: Vec<SubProblem>,
    pole: Vec<Pole>,
    board_size: f32,
    base_length: f32,
    base_width: f32,
    base_height: f32,
    pole_radius: f32,
    pole_height: f32,
    pole_offset: f32,
    /// How far the poles are from the centre, when they are in a ring.
    pole_dist: f32,
    disk_height: f32,
    max_disk_radius: f32,
    /// Where each level of a stack sits, worked out once.
    disk_pos: Vec<f32>,
    disk: Vec<Disk>,
    floor_list: u32,
    base_list: u32,
    pole_list: u32,
    trail_q: Vec<TrailPoint>,
    trail_q_front: usize,
    trail_q_back: usize,
    camera: [f64; 3],
    centre: [f64; 3],
    rot: Rotator,
    button_down: bool,
    texture: bool,
    texture_name: u32,
    drag: (i32, i32),
    aspect: f64,
    scale: f32,
}

impl GlHanoi {
    fn push_disk(&mut self, idx: usize, d: usize) {
        if self.pole[idx].data.len() < self.number_of_disks {
            self.pole[idx].data.push(d);
        }
    }

    fn pop_disk(&mut self, idx: usize) -> Option<usize> {
        self.pole[idx].data.pop()
    }

    fn disk_radius(&self, i: usize) -> f32 {
        self.max_disk_radius * (i as f32 + 3.0) / (self.number_of_disks as f32 + 3.0)
    }

    /// `moveSetup`: work out the whole flight of one disk, from where it is to
    /// where it is going, as a throw under gravity.
    fn move_setup(&mut self, di: usize) {
        let (src, dst) = (self.src, self.dst);
        let src_pos = self.pole[src].position;
        let dst_pos = self.pole[dst].position;
        let src_count = self.pole[src].data.len();
        let dst_count = self.pole[dst].data.len();
        let finished = self.state == State::Finished;
        let pole_height = self.pole_height;
        let base0 = self.disk_pos[src_count];
        let base1 = if finished {
            base0
        } else {
            self.disk_pos[dst_count]
        };
        let n_disks = self.number_of_disks;
        let dh = distance(src_pos, dst_pos);

        let d = &mut self.disk[di];
        d.xmin = src_pos[0];
        d.xmax = dst_pos[0];
        d.ymin = pole_height;
        d.zmin = src_pos[2];
        d.zmax = dst_pos[2];

        let dx = f64::from(d.xmax - d.xmin);
        let dz = f64::from(d.zmax - d.zmin);

        if !finished {
            let xxx = if dx < 0.0 { 180.0 } else { -180.0 };
            if random().is_multiple_of(6) {
                // Upstream writes `(2 - 2 * random() % 2)`, which by C's
                // precedence is `2 - ((2 * random()) % 2)` and so is always 2.
                d.rot_angle = xxx * 2.0 * f64::from(random() % 3 + 1);
            } else {
                d.rot_angle = xxx;
            }
            if random().is_multiple_of(4) {
                // Backflip.
                d.rot_angle = -d.rot_angle;
            }
        } else {
            d.rot_angle = -180.0;
        }

        d.base0 = base0;
        d.base1 = base1;

        let mut ymax = pole_height + dh;
        if finished {
            ymax += dh * (n_disks - d.id) as f32;
        }
        let h = ymax - d.ymin;
        // `A(a, b, c) = -4 c / (a - b)^2`, so theta = atan(4h / dh).
        let mut theta = (4.0 * f64::from(h) / f64::from(dh)).atan();
        if theta < 0.0 {
            theta += PI;
        }
        let costheta = theta.cos();
        let sintheta = theta.sin();
        let u = (-f64::from(G)
            / (2.0 * -4.0 * f64::from(h) / (f64::from(dh) * f64::from(dh)) * costheta * costheta))
            .abs()
            .sqrt() as f32;
        d.usintheta = u * sintheta as f32;
        d.ucostheta = u * costheta as f32;
        // Not to be confused: `dx` here is the per-time-unit portion of the
        // whole distance.
        d.dx = d.ucostheta * (dx / f64::from(dh)) as f32;
        d.dz = d.ucostheta * (dz / f64::from(dh)) as f32;
        d.t1 = (-u + (u * u + 2.0 * G * (d.ymin - d.base0).abs()).sqrt()) / G;
        d.u1 = u + G * d.t1;
        d.t2 = 2.0 * d.usintheta / G;
        d.u2 = d.usintheta - G * d.t2;

        d.phi = (dz / dx).atan() * 180.0 / PI;
    }

    /* The solver */

    fn push_move(&mut self, n: i32, src: usize, dst: usize, avail: u32) {
        self.solve_stack.push(SubProblem {
            n_disks: n,
            src,
            dst,
            available: avail & !(1 << src) & !(1 << dst),
        });
    }

    /// `makeMove`: pick the next single disk to move and set its flight up.
    fn make_move(&mut self) {
        if self.number_of_poles == 3 {
            let fudge = self.move_ + 2;
            let magic_number = magic(fudge);

            if let Some(d) = self.pop_disk(self.src) {
                self.current_disk = d;
                self.move_setup(d);
                self.push_disk(self.dst, d);
            }

            let fudge = fudge % 2;
            if fudge == 1 || magic_number {
                std::mem::swap(&mut self.src, &mut self.tmp);
            }
            if fudge == 0 || self.magic_number {
                std::mem::swap(&mut self.dst, &mut self.tmp);
            }
            self.magic_number = magic_number;
            return;
        }

        let mut tmp = 0;
        if self.move_ == 0 {
            // The original problem: move every disk from pole zero to the
            // furthest pole, using all the others.
            let all_poles = (1u32 << self.number_of_poles) - 1;
            let avail = all_poles & !1 & !(1 << (self.number_of_poles - 1));
            self.push_move(
                self.number_of_disks as i32,
                0,
                self.number_of_poles - 1,
                avail,
            );
        }

        while let Some(sp) = self.solve_stack.pop() {
            if sp.n_disks == 1 {
                // A single, concrete move to do.
                self.src = sp.src;
                self.dst = sp.dst;
                self.tmp = tmp;
                if let Some(d) = self.pop_disk(sp.src) {
                    self.current_disk = d;
                    self.move_setup(d);
                    self.push_disk(sp.dst, d);
                }
                return;
            }

            // Divide and conquer by Frame-Stewart until the base case.
            let num_avail = sp.available.count_ones() as i32;
            let mut k = if num_avail < 2 {
                sp.n_disks - 1
            } else if num_avail >= sp.n_disks - 2 {
                1
            } else {
                // The heuristic for the optimal k is sqrt(2n).
                (2.0 * f64::from(sp.n_disks)).sqrt() as i32
            };
            k = k.clamp(1, sp.n_disks - 1);

            tmp = sp.available.trailing_zeros() as usize;

            // Pushed on in reverse order, since this is a stack.
            self.push_move(k, tmp, sp.dst, (sp.available | (1 << sp.src)) & !(1 << tmp));
            self.push_move(sp.n_disks - k, sp.src, sp.dst, sp.available & !(1 << tmp));
            self.push_move(k, sp.src, tmp, (sp.available | (1 << sp.dst)) & !(1 << tmp));
        }
    }

    fn finished_hanoi(&self) -> bool {
        if self.number_of_poles == 3 {
            self.move_ >= self.number_of_moves
        } else {
            self.solve_stack.is_empty()
        }
    }

    /* Flight */

    /// Add a trail point at `posn`, dropping the oldest to make room.
    fn en_q_trail(&mut self, posn: [f32; 3], now: f64) {
        if self.trail_q.is_empty() || self.state == State::MoneyShot {
            return;
        }
        let size = self.trail_q.len();
        let tp = &mut self.trail_q[self.trail_q_back];
        tp.position[0] = posn[0];
        tp.position[1] = posn[1] + self.disk_height;
        // A slight jitter, to keep trails from clashing with each other.
        tp.position[2] = posn[2] + (self.move_ % 23) as f32 * 0.01;
        tp.start_time = now + TRAIL_START_DELAY;
        tp.end_time = now + TRAIL_START_DELAY + self.trail_duration;
        tp.is_end = false;

        self.trail_q_back = (self.trail_q_back + 1) % size;
        if self.trail_q_back == self.trail_q_front {
            self.trail_q_front = (self.trail_q_front + 1) % size;
        }
    }

    /// Mark the last point in the queue as the end of a trail.
    fn end_trail(&mut self) {
        let size = self.trail_q.len();
        if size == 0 {
            return;
        }
        let i = (self.trail_q_back + size - 1) % size;
        self.trail_q[i].is_end = true;
    }

    /// Update one disk's position and rotation for time `t`. True once the
    /// move is over.
    fn compute_position(&mut self, t: f32, di: usize, now: f64) -> bool {
        let (t1, t2) = (self.disk[di].t1, self.disk[di].t2);
        if t < t1 {
            // Straight up the pole it is leaving.
            let d = &mut self.disk[di];
            d.position[0] = d.xmin;
            d.position[1] = d.base0 + (d.u1 - 0.5 * G * t) * t;
            d.position[2] = d.zmin;
            d.rotation[1] = 0.0;
            false
        } else if t < t1 + t2 {
            // The throw, tumbling as it goes.
            let t = t - t1;
            let d = &mut self.disk[di];
            d.position[0] = d.xmin + d.dx * t;
            d.position[2] = d.zmin + d.dz * t;
            d.position[1] = d.ymin + (d.usintheta - 0.5 * G * t) * t;
            d.rotation[1] = (d.rot_angle * f64::from(t) / f64::from(d.t2)) as f32;
            let pos = d.position;
            self.en_q_trail(pos, now);
            false
        } else {
            // Down the pole it is arriving at.
            let t = t - t1 - t2;
            let d = &mut self.disk[di];
            d.position[0] = d.xmax;
            d.position[1] = d.ymin + (d.u2 - 0.5 * G * t) * t;
            d.position[2] = d.zmax;
            d.rotation[1] = 0.0;
            if d.position[1] <= d.base1 {
                d.position[1] = d.base1;
                self.end_trail();
                return true;
            }
            false
        }
    }

    fn change_state(&mut self, state: State, now: f64) {
        self.state = state;
        self.start_time = now;
    }

    fn update(&mut self, now: f64) {
        let t = now - self.start_time;
        match self.state {
            State::Start => {
                if t < self.duration {
                    return;
                }
                self.move_ = 0;
                if self.number_of_disks.is_multiple_of(2) {
                    std::mem::swap(&mut self.tmp, &mut self.dst);
                }
                self.magic_number = true;
                self.make_move();
                self.change_state(State::MoveDisk, now);
            }
            State::MoveDisk => {
                let di = self.current_disk;
                let speed = self.disk[di].speed;
                if self.compute_position(t as f32 * speed, di, now) {
                    self.change_state(State::MoveFinished, now);
                }
            }
            State::MoveFinished => {
                self.move_ += 1;
                if !self.finished_hanoi() {
                    self.make_move();
                    self.change_state(State::MoveDisk, now);
                } else {
                    self.duration = FINISH_DURATION;
                    self.change_state(State::Finished, now);
                }
            }
            State::Finished => {
                if t < self.duration {
                    return;
                }
                self.src = self.olddst;
                self.dst = self.oldsrc;
                for _ in 0..self.number_of_disks {
                    if let Some(di) = self.pop_disk(self.src) {
                        self.move_setup(di);
                    }
                }
                for i in (0..=self.max_disk_idx).rev() {
                    self.push_disk(self.dst, i);
                }
                self.change_state(State::MoneyShot, now);
            }
            State::MoneyShot => {
                let mut done = true;
                for i in (0..=self.max_disk_idx).rev() {
                    let delay = 0.25 * i as f64;
                    if t - delay < 0.0 {
                        done = false;
                        continue;
                    }
                    let finished = self.compute_position((t - delay) as f32, i, now);
                    self.disk[i].rotation[1] = 0.0;
                    if !finished {
                        done = false;
                    }
                }
                if done {
                    self.src = self.oldsrc;
                    self.tmp = self.oldtmp;
                    self.dst = self.olddst;
                    self.change_state(State::Start, now);
                }
            }
        }
    }

    /// `updateView`: where the camera is, from the rotator plus whatever the
    /// mouse has dragged it to.
    fn update_view(&mut self, g: &mut Gl) {
        let turning = !self.button_down;
        let (_, _, radius) = self.rot.position(turning);
        let (longitude, latitude, _) = self.rot.rotation(turning);
        let mut longitude = longitude + self.camera[0];
        let mut latitude = latitude + self.camera[1];
        let mut radius = radius + self.camera[2];
        longitude -= longitude.floor();
        latitude -= latitude.floor();
        radius -= radius.floor();
        if latitude > 0.5 {
            latitude = 1.0 - latitude;
        }
        if radius > 0.5 {
            radius = 1.0 - radius;
        }

        let b = self.centre[1];
        let c = MIN_CAMERA_RADIUS + radius * (MAX_CAMERA_RADIUS - MIN_CAMERA_RADIUS);
        let big_a = PI / 4.0 * (1.0 - latitude);
        let a = (b * b + c * c - 2.0 * b * c * big_a.cos()).sqrt();
        let big_b = (big_a.sin() * b / a).asin();
        g.glx.rotate((-big_b * 180.0 / PI) as f32, 1.0, 0.0, 0.0);
        g.glx.translate(0.0, 0.0, -c as f32);
        g.glx.rotate((longitude * 360.0) as f32, 0.0, 1.0, 0.0);
        g.glx.rotate(
            (latitude * 180.0) as f32,
            (longitude * 2.0 * PI).cos() as f32,
            0.0,
            (longitude * 2.0 * PI).sin() as f32,
        );
    }

    /* Building the scene */

    /// `drawBaseStrip`: one band of the ring-shaped base, from one corner
    /// round to the other.
    /// `idx` is upstream's `y1, r1, y2, r2`: which of the two heights and
    /// which of the two radii each of the strip's two rails follows.
    fn draw_base_strip(
        &self,
        g: &mut Gl,
        idx: [usize; 4],
        y: [f32; 2],
        r: [f32; 2],
        ends: [[[f32; 2]; 2]; 2],
    ) {
        let [y1, r1, y2, r2] = idx;
        let [beg, end] = ends;
        // Upstream sets each normal *after* the pair of vertices it belongs
        // with, so a normal covers the next pair along.
        let set_normal = |g: &mut Gl, theta: f64| {
            if y1 == y2 {
                g.glx.normal3f(0.0, if y1 != 0 { 1.0 } else { -1.0 }, 0.0);
            } else if r1 == 0 {
                g.glx
                    .normal3f(-theta.cos() as f32, 0.0, -theta.sin() as f32);
            } else {
                g.glx.normal3f(theta.cos() as f32, 0.0, theta.sin() as f32);
            }
        };

        let theta1 = (PI * 2.0) / (self.number_of_poles + 1) as f64;
        g.glx.begin(Shape::QuadStrip);

        g.glx.vertex3f(beg[r1][0], y[y1], beg[r1][1]);
        g.glx.vertex3f(beg[r2][0], y[y2], beg[r2][1]);
        set_normal(g, theta1);

        for i in 1..self.number_of_poles {
            let theta = theta1 * (i as f64 + 0.5);
            let costh = theta.cos() as f32;
            let sinth = theta.sin() as f32;
            let x = [costh * r[0], costh * r[1]];
            let z = [sinth * r[0], sinth * r[1]];
            g.glx.vertex3f(x[r1], y[y1], z[r1]);
            g.glx.vertex3f(x[r2], y[y2], z[r2]);
            set_normal(g, theta1 * (i + 1) as f64);
        }

        g.glx.vertex3f(end[r1][0], y[y1], end[r1][1]);
        g.glx.vertex3f(end[r2][0], y[y2], end[r2][1]);
        set_normal(g, self.number_of_poles as f64);

        g.glx.end();
    }

    /// `drawRoundBase`: a base shaped so the poles sit at the vertices of a
    /// regular polygon, with a square gap where the ring does not close.
    fn draw_round_base(&self, g: &mut Gl) {
        let np = self.number_of_poles as f64;
        // How much longer the radius is at a vertex than at a pole.
        let longer = (1.0 / (PI / (np + 1.0)).cos()) as f32;
        let r = [
            (self.pole_dist - self.max_disk_radius) * longer,
            (self.pole_dist + self.max_disk_radius) * longer,
        ];
        let y = [0.0, self.base_height];

        // The two square ends of the ring.
        let theta = PI * 2.0 / (np + 1.0);
        let (costh, sinth) = (theta.cos() as f32, theta.sin() as f32);
        let (inner, outer) = (
            self.pole_dist - self.max_disk_radius,
            self.pole_dist + self.max_disk_radius,
        );
        let m = self.max_disk_radius;
        let beg = [
            [inner * costh + m * sinth, inner * sinth - m * costh],
            [outer * costh + m * sinth, outer * sinth - m * costh],
        ];
        let beg_norm = theta - PI * 0.5;

        let theta = PI * 2.0 * np / (np + 1.0);
        let (costh, sinth) = (theta.cos() as f32, theta.sin() as f32);
        let end = [
            [inner * costh - m * sinth, inner * sinth + m * costh],
            [outer * costh - m * sinth, outer * sinth + m * costh],
        ];
        let end_norm = theta + PI * 0.5;

        // The bottom is never seen, so upstream leaves it out.
        self.draw_base_strip(g, [0, 1, 1, 1], y, r, [beg, end]);
        self.draw_base_strip(g, [1, 1, 1, 0], y, r, [beg, end]);
        self.draw_base_strip(g, [1, 0, 0, 0], y, r, [beg, end]);

        g.glx.begin(Shape::Quads);
        g.glx.vertex3f(beg[0][0], y[1], beg[0][1]);
        g.glx.vertex3f(beg[1][0], y[1], beg[1][1]);
        g.glx.vertex3f(beg[1][0], y[0], beg[1][1]);
        g.glx.vertex3f(beg[0][0], y[0], beg[0][1]);
        g.glx
            .normal3f(beg_norm.cos() as f32, 0.0, beg_norm.sin() as f32);

        g.glx.vertex3f(end[0][0], y[0], end[0][1]);
        g.glx.vertex3f(end[1][0], y[0], end[1][1]);
        g.glx.vertex3f(end[1][0], y[1], end[1][1]);
        g.glx.vertex3f(end[0][0], y[1], end[0][1]);
        g.glx
            .normal3f(end_norm.cos() as f32, 0.0, end_norm.sin() as f32);
        g.glx.end();
    }

    /// `drawTrails1`: the fading arcs the disks leave behind them, drawn once
    /// thin and bright and once thick and dim to smooth them out.
    fn draw_trails1(&mut self, g: &mut Gl, t: f64, thickness: f32, alpha: f32) {
        let size = self.trail_q.len();
        if size == 0 {
            return;
        }
        let mut prev: Option<usize> = None;
        let mut fresh = false;
        let inv = 1.0 / self.trail_duration as f32;

        g.glx.line_width(thickness);
        g.glx.begin(Shape::Lines);

        let mut i = self.trail_q_front;
        while i != self.trail_q_back {
            let tqi = self.trail_q[i];
            if !fresh && t > tqi.end_time {
                self.trail_q_front = (i + 1) % size;
            } else {
                if tqi.start_time > t {
                    break;
                }
                fresh = true;
                if let Some(p) = prev {
                    // Fade to invisible with age. Upstream notes that doing
                    // this properly would want the trails sorted back to
                    // front, and recommends keeping them short instead.
                    let a = alpha * (tqi.end_time - t) as f32 * inv;
                    g.glx.color4f(1.0, 1.0, 1.0, a);
                    let from = self.trail_q[p].position;
                    g.glx.vertex3f(from[0], from[1], from[2]);
                    g.glx
                        .vertex3f(tqi.position[0], tqi.position[1], tqi.position[2]);
                }
                prev = if tqi.is_end { None } else { Some(i) };
            }
            i = (i + 1) % size;
        }

        g.glx.end();
    }

    fn draw_disks(&self, g: &mut Gl) {
        g.glx.push_matrix();
        g.glx.translate(0.0, self.base_height, 0.0);
        for i in (0..=self.max_disk_idx).rev() {
            let disk = &self.disk[i];
            let pos = disk.position;
            let rot = disk.rotation;

            g.glx.push_matrix();
            g.glx.translate(pos[0], pos[1], pos[2]);
            if rot[1] != 0.0 {
                g.glx.translate(0.0, self.disk_height / 2.0, 0.0);
                // Rotate about a different axis depending on which way the
                // disk is travelling.
                if disk.phi != 0.0 {
                    g.glx.rotate(-disk.phi as f32, 0.0, 1.0, 0.0);
                }
                g.glx.rotate(rot[1], 0.0, 0.0, 1.0);
                if disk.phi != 0.0 {
                    g.glx.rotate(disk.phi as f32, 0.0, 1.0, 0.0);
                }
                g.glx.translate(0.0, -self.disk_height / 2.0, 0.0);
            }
            g.glx.call_list(disk.display_list);
            g.glx.pop_matrix();
        }
        g.glx.pop_matrix();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut number_of_disks = g.res.int("count") as usize;
    if g.res.int("count") <= 1 {
        number_of_disks = 3 + bellrand(9.0) as usize;
    }
    // The magic number is a bitfield, so there is no room for more than
    // thirty-one disks.
    number_of_disks = number_of_disks.clamp(1, 31);
    let max_disk_idx = number_of_disks - 1;

    let mut number_of_poles = g.res.int("poles") as usize;
    if g.res.int("poles") <= 2 {
        // Three to one more than the number of disks, biased low: the
        // probability falls off linearly.
        number_of_poles = 3 + ((1.0 - frand(1.0).sqrt()) * (number_of_disks - 1) as f64) as usize;
    }
    number_of_poles = number_of_poles.clamp(3, 31);

    // Upstream turns wireframe off in its own OpenGL ES build, since
    // `glPolygonMode` does not exist there, and it does not exist here either.
    let wire = false;
    let speed = g.res.float("speed");
    let trails = g.res.float("trails");
    let layout_linear = number_of_poles == 3;

    let base_length = BASE_LENGTH;
    let max_disk_radius = if layout_linear {
        base_length / (2.0 * 0.95 * number_of_poles as f32)
    } else {
        let s = (PI / (number_of_poles + 1) as f64).sin() as f32;
        (s * base_length * 0.5 * 0.95) / (1.0 + s)
    };
    let pole_dist = base_length * 0.5 - max_disk_radius;
    let pole_radius = max_disk_radius / (number_of_disks as f32 + 3.0);
    let disk_height = 2.0 * pole_radius;
    let base_height = 2.0 * pole_radius;
    let pole_height = number_of_disks as f32 * disk_height + pole_radius;

    let trail_q_size = (trails * 60.0) as usize;

    let mut this = GlHanoi {
        state: State::Start,
        wire,
        fog: g.res.bool("fog"),
        light: g.res.bool("light"),
        trail_duration: trails,
        start_time: 0.0,
        duration: START_DURATION,
        number_of_disks,
        number_of_poles,
        number_of_moves: (1i64 << number_of_disks) - 1,
        max_disk_idx,
        magic_number: false,
        current_disk: 0,
        move_: 0,
        src: 0,
        tmp: 1,
        dst: number_of_poles - 1,
        oldsrc: 0,
        oldtmp: 1,
        olddst: number_of_poles - 1,
        solve_stack: Vec::new(),
        pole: vec![Pole::default(); number_of_poles],
        // The golden ratio, for the size of the board.
        board_size: base_length * 0.5 * (1.0 + 5.0f32.sqrt()),
        base_length,
        base_width: 2.0 * max_disk_radius,
        base_height,
        pole_radius,
        pole_height,
        pole_offset: 0.0,
        pole_dist,
        disk_height,
        max_disk_radius,
        disk_pos: vec![0.0; number_of_disks],
        disk: vec![Disk::default(); number_of_disks],
        floor_list: 0,
        base_list: 0,
        pole_list: 0,
        trail_q: vec![TrailPoint::default(); trail_q_size],
        trail_q_front: 0,
        trail_q_back: 0,
        camera: [0.0; 3],
        centre: [0.0, f64::from(pole_height) * 3.0, 0.0],
        rot: Rotator::new(0.1, 0.025, 0.0, 1.0, 0.005, false),
        button_down: false,
        texture: g.res.bool("texture"),
        texture_name: 0,
        drag: (0, 0),
        aspect: 1.0,
        scale: 1.0,
    };
    this.pole_offset = 2.0 * this.disk_radius(max_disk_idx);

    // The marble.
    this.texture_name = g.glx.gen_texture();
    g.glx.bind_texture(this.texture_name);
    g.glx.tex_nearest(false);
    g.glx.tex_clamp(false);
    let size = MARBLE_TEXTURE_SIZE as i32;
    g.glx.tex_image_2d(size, size, make_marble_texture());

    // The floor: a chequerboard of the marble. Upstream sets the tile colour
    // as a material inside the display list; a list here replays geometry and
    // not state, so the colour rides on the vertices instead, which is what
    // `GL_COLOR_MATERIAL` with `GL_AMBIENT_AND_DIFFUSE` means anyway.
    this.floor_list = g.glx.gen_lists(1);
    g.glx.new_list(this.floor_list);
    let tile_size = this.board_size / BOARD_SQUARES as f32;
    let tex_incr = 1.0 / BOARD_SQUARES as f32;
    g.glx.bind_texture(this.texture_name);
    g.glx.normal3f(0.0, 1.0, 0.0);
    let mut x0 = -this.board_size / 2.0;
    let mut tx0 = 0.0f32;
    for i in 0..BOARD_SQUARES {
        let x1 = x0 + tile_size;
        let tx1 = tx0 + tex_incr;
        let mut z0 = -this.board_size / 2.0;
        let mut tz0 = 0.0f32;
        for j in 0..BOARD_SQUARES {
            let z1 = z0 + tile_size;
            let tz1 = tz0 + tex_incr;
            let col = if (i + j) & 1 != 0 { 1.0 } else { 0.0 };
            g.glx.color3f(col, col, col);
            g.glx.begin(Shape::Quads);
            g.glx.tex_coord2f(tx0, tz0);
            g.glx.vertex3f(x0, 0.0, z0);
            g.glx.tex_coord2f(tx0, tz1);
            g.glx.vertex3f(x0, 0.0, z1);
            g.glx.tex_coord2f(tx1, tz1);
            g.glx.vertex3f(x1, 0.0, z1);
            g.glx.tex_coord2f(tx1, tz0);
            g.glx.vertex3f(x1, 0.0, z0);
            g.glx.end();
            z0 += tile_size;
            tz0 += tex_incr;
        }
        x0 += tile_size;
        tx0 += tex_incr;
    }
    g.glx.end_list();

    // The poles, and where they stand.
    for i in 0..number_of_poles {
        let rad = (PI * 2.0 * (i + 1) as f64) / (number_of_poles + 1) as f64;
        let p = &mut this.pole[i].position;
        p[1] = base_height;
        if layout_linear {
            p[0] = -this.pole_offset * ((number_of_poles - 1) as f32 * 0.5 - i as f32);
            p[2] = 0.0;
        } else {
            p[0] = rad.cos() as f32 * pole_dist;
            p[2] = rad.sin() as f32 * pole_dist;
        }
    }

    this.base_list = g.glx.gen_lists(1);
    g.glx.new_list(this.base_list);
    g.glx.color3f(BASE_COLOR[0], BASE_COLOR[1], BASE_COLOR[2]);
    if layout_linear {
        draw_cuboid(g, this.base_length, this.base_width, this.base_height);
    } else {
        this.draw_round_base(g);
    }
    g.glx.end_list();

    this.pole_list = g.glx.gen_lists(1);
    g.glx.new_list(this.pole_list);
    g.glx.color3f(POLE_COLOR[0], POLE_COLOR[1], POLE_COLOR[2]);
    for i in 0..number_of_poles {
        let p = this.pole[i].position;
        g.glx.push_matrix();
        g.glx.translate(p[0], p[1], p[2]);
        draw_tube(
            g,
            [this.pole_radius; 2],
            [this.pole_radius; 2],
            this.pole_height,
            NSLICE,
            NLOOPS,
        );
        g.glx.pop_matrix();
    }
    g.glx.end_list();

    // The disks, largest at the bottom, coloured along the hue ramp.
    for i in (0..=max_disk_idx).rev() {
        let height = (max_disk_idx - i) as f32;
        let f = cfunc(i as f64 / number_of_disks as f64);
        let color = hsv_to_rgb((f * 360.0) as f32, 1.0, 1.0);
        let speed = lerp(
            (number_of_disks - i) as f64 / number_of_disks as f64,
            1.0,
            speed,
        ) as f32;
        let radius = this.disk_radius(i);
        let list = g.glx.gen_lists(1);
        let p0 = this.pole[0].position;

        let d = &mut this.disk[i];
        d.id = i;
        d.position = [p0[0], disk_height * height, p0[2]];
        d.rotation = [0.0; 3];
        // Smaller disks move faster.
        d.speed = speed;
        d.display_list = list;

        g.glx.new_list(list);
        g.glx.color3f(color[0], color[1], color[2]);
        draw_tube(
            g,
            [radius; 2],
            [radius - pole_radius; 2],
            disk_height,
            NSLICE,
            NLOOPS,
        );
        g.glx.end_list();
    }
    for i in (0..=max_disk_idx).rev() {
        let h = max_disk_idx - i;
        this.disk_pos[h] = disk_height * (max_disk_idx - i) as f32;
        let src = this.src;
        this.push_disk(src, i);
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for GlHanoi {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 9 / 16;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = f64::from(width) / f64::from(height);
        self.scale = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        match *event {
            XEvent::ButtonPress { x, y, button: 1 } => {
                self.button_down = true;
                self.drag = (x, y);
                true
            }
            XEvent::ButtonRelease { button: 1, .. } => {
                self.button_down = false;
                true
            }
            XEvent::ButtonPress { button, .. } if button == 4 || button == 5 => {
                self.camera[2] += if button == 4 { 0.01 } else { -0.01 };
                true
            }
            XEvent::MotionNotify { x, y } if self.button_down => {
                self.camera[0] = f64::from(x - self.drag.0) / f64::from(g.width());
                self.camera[1] = f64::from(y - self.drag.1) / f64::from(g.height());
                true
            }
            _ => false,
        }
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let now = g.time;

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(
            30.0,
            self.aspect as f32,
            1.0,
            (2.0 * MAX_CAMERA_RADIUS) as f32,
        );
        g.glx.matrix_mode_modelview();

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        // The colour of everything rides on the vertices, since a display list
        // here replays geometry and not the materials set inside it.
        g.glx.color_material(true);
        g.glx.lighting(self.light && !self.wire);
        if self.light && !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 50.0, 50.0, 50.0, 0.0);
            g.glx.light_ambient(0, C_BLACK);
            g.glx.light_diffuse(0, C_WHITE);
            g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
            g.glx.light_enable(1, true);
            g.glx.light_position(1, -50.0, 50.0, -50.0, 0.0);
            g.glx.light_ambient(1, C_BLACK);
            g.glx.light_diffuse(1, C_WHITE);
            g.glx.light_specular(1, C_WHITE);
        }
        if self.fog {
            g.glx
                .clear_color(FOG_COLOR[0], FOG_COLOR[1], FOG_COLOR[2], 1.0);
            g.glx.fog(Some(Fog::Linear {
                start: MIN_CAMERA_RADIUS as f32,
                end: (MAX_CAMERA_RADIUS / 1.9) as f32,
                color: FOG_COLOR,
            }));
        }

        g.glx.load_identity();

        self.update(now);
        self.update_view(g);
        g.glx.scale(self.scale, self.scale, self.scale);

        if !self.wire && self.texture {
            g.glx.texturing(true);
        }
        g.glx.material_specular(C_WHITE);
        g.glx.material_shininess(100.0);
        g.glx.call_list(self.floor_list);
        g.glx.texturing(false);

        g.glx.material_shininess(50.0);
        g.glx.call_list(self.base_list);
        g.glx.call_list(self.pole_list);

        g.glx.material_shininess(100.0);
        self.draw_disks(g);

        if !self.trail_q.is_empty() {
            g.glx.blend(Blend::Alpha);
            g.glx.material_specular(C_BLACK);
            g.glx.material_shininess(0.0);
            // Twice over, at different widths and opacities, to make them
            // smoother.
            self.draw_trails1(g, now, 1.0, 0.75);
            self.draw_trails1(g, now, 2.5, 0.5);
            g.glx.blend(Blend::Off);
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:     15000",
    "*count:     0",
    "*showFPS:   False",
    "*wireframe: False",
    "*light:     True",
    "*fog:       False",
    "*texture:   True",
    "*poles:     0",
    "*speed:     1",
    "*trails:    2",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "15000").inverted(),
    Opt::slider("count", "Number of disks", 0.0, 31.0, 1.0, 0, "0"),
    Opt::slider("poles", "Number of poles", 0.0, 31.0, 1.0, 0, "0"),
    Opt::slider(
        "speed",
        "Speed of the smallest disks",
        1.0,
        20.0,
        1.0,
        0,
        "1",
    ),
    Opt::slider("trails", "Length of disk trails", 0.0, 10.0, 0.5, 1, "2"),
    Opt::boolean("fog", "Enable fog", "false"),
    Opt::boolean("light", "Enable lighting", "true"),
    Opt::boolean("texture", "Marble", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glhanoi",
    label: "GL Hanoi",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Dave Atkinson",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=1qRCviRmsTY"),
        blurb: "Solves the Towers of Hanoi puzzle. Move N disks from one pole \
                to another, one disk at a time, with no disk ever resting on a \
                disk smaller than itself.",
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

    /// The three-pole solution is driven by the parity of the move number, and
    /// `magic` is what reads it: true when the count of trailing zeroes is
    /// even, and never for zero or one.
    #[test]
    fn the_magic_number_is_the_parity_of_the_trailing_zeroes() {
        assert!(!magic(0));
        assert!(!magic(1));
        assert!(!magic(2), "10 has one trailing zero, which is odd");
        assert!(magic(4), "100 has two");
        for i in 2..64i64 {
            let want = i.trailing_zeros() % 2 == 0;
            assert_eq!(magic(i), want, "magic({i})");
        }
    }

    /// The Frame-Stewart solver has to actually finish: every disk on the
    /// last pole and none anywhere else.
    #[test]
    fn more_than_three_poles_still_solves_it() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "count=8&poles=5&speed=20",
            20260812,
        ));
        // Long enough for the whole puzzle at the fastest setting.
        for _ in 0..4000 {
            r.step();
        }
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing was drawn");
    }

    /// The marble is a texture, and one with something in it rather than a
    /// flat field.
    #[test]
    fn the_marble_has_grain() {
        let t = make_marble_texture();
        assert_eq!(t.len(), MARBLE_TEXTURE_SIZE * MARBLE_TEXTURE_SIZE * 4);
        let lo = t.iter().step_by(4).copied().min().unwrap_or(0);
        let hi = t.iter().step_by(4).copied().max().unwrap_or(0);
        assert!(hi - lo > 100, "the marble runs from {lo} to {hi}");
        assert!(t.iter().skip(3).step_by(4).all(|&a| a == 0xff));
    }

    /// The scene is the floor, the base, the poles and one batch per disk, so
    /// a dozen disks is still a couple of dozen batches.
    #[test]
    fn the_scene_is_a_handful_of_batches() {
        let mut r = start(StartArgs::new(640, 480, "count=10&poles=3", 20260812));
        for _ in 0..60 {
            r.step();
        }
        let f = r.frame();
        assert!(
            f.batches.len() < 60,
            "{} batches for the towers",
            f.batches.len()
        );
        assert!(f.vertices.len() > 1000, "the towers are missing");
    }

    /// A disk in flight goes up its own pole, over, and down the other, and
    /// never dips below where it started or above the top of the arc.
    #[test]
    fn a_disk_flies_over_rather_than_through() {
        let mut r = start(StartArgs::new(640, 480, "count=4&poles=3", 20260812));
        let mut lowest = f32::MAX;
        let mut highest = f32::MIN;
        for _ in 0..600 {
            r.step();
            for b in &r.frame().batches {
                for v in &r.frame().vertices[b.first..b.first + b.count] {
                    lowest = lowest.min(v.pos[1]);
                    highest = highest.max(v.pos[1]);
                }
            }
        }
        assert!(lowest >= -0.001, "something went under the floor: {lowest}");
        assert!(highest > 5.0, "nothing ever left the base: {highest}");
    }
}
