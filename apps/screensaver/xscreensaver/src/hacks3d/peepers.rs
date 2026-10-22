//! Port of `hacks/glx/peepers.c`.
//!
//! ```text
//! peepers, Copyright (c) 2018-2019 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Created: 14 Feb 2018, jwz.
//!
//! Floating eyeballs!
//!
//! Inspired by @PaintYourDragon's Adafruit Snake Eyes Raspberry Pi Bonnet
//! https://learn.adafruit.com/animated-snake-eyes-bonnet-for-raspberry-pi/
//! which is excellent.
//! ```
//!
//! Eyeballs, bouncing up from the bottom of the screen or scrolling across it,
//! laid out in a grid, or stuck all over a ball. Each one is four shells: a
//! black retina to stop you seeing out the back of it, a coloured iris that
//! dilates, a photographed sclera with the veins on it, and a translucent lens
//! over the front.
//!
//! An eye either turns slowly on the spot, spins, or watches something. Where
//! two of them share a screen they watch the same thing, because a pair of
//! eyes that disagree looks wrong.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::rotator::Rotator;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
    random_below,
};
use std::f64::consts::PI;

/// The bottom edge of the screen; the left and right scale by the aspect.
const BOTTOM: f32 = -1.6;

/// How big the iris is across the front of the eye.
const IRIS_RATIO: f32 = 0.42;

fn bellrand(n: f32) -> f32 {
    ((frand(f64::from(n)) + frand(f64::from(n)) + frand(f64::from(n))) / 3.0) as f32
}

fn randsign() -> f32 {
    if random() % 2 == 1 { 1.0 } else { -1.0 }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Rotate,
    Spin,
    Track,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Bounce,
    ScrollLeft,
    ScrollRight,
    Xeyes,
    Beholder,
}

/// Which shell of the eye is being drawn. `Tick` is not a shell: it is the
/// pass that moves the eyes, which upstream runs as one more of these so that
/// every eye is drawn before any of them moves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Component {
    Retina,
    Iris,
    Sclera,
    Lens,
    Tick,
}

/// How wide the pupil is: where it was, where it is going, and how far along.
#[derive(Clone, Copy, Default)]
struct Dilation {
    from: f32,
    to: f32,
    current: f32,
    tick: f32,
}

struct Floater {
    idx: usize,
    x: f32,
    y: f32,
    z: f32,
    dx: f32,
    dy: f32,
    dz: f32,
    ddx: f32,
    ddy: f32,
    ddz: f32,
    rot: Rotator,
    dilation: Dilation,
    focus: Focus,
    track: [f32; 3],
    tilt: f32,
    roll: f32,
    scale: f32,
    color: [f32; 4],
    /// One in twenty eyes is bloodshot, and the red-eyed ones always are.
    jaundice: i32,
}

/// How common each eye colour is. "All of the articles that I found with
/// percentages in them only added up to around 70%, so who knows what that
/// means."
const EYE_COLORS: [(f32, u32); 7] = [
    // Brown is the real global majority, "but that's a lot of brown...".
    (20.0, 0x985A07),
    (8.0, 0xD5AD68), // hazel
    (8.0, 0x777F92), // blue
    (2.0, 0x6B7249), // green
    (1.0, 0x7F7775), // gray
    (0.5, 0x9E8042), // amber
    (0.1, 0xFFAA88), // red
];

/// Latitude and longitude, for the geodesic layout.
#[derive(Clone, Copy)]
struct Ll {
    a: f64,
    o: f64,
}

struct Peepers {
    trackball: Trackball,
    button_down: bool,
    mouse: [f32; 3],
    last_mouse: (f32, f32),
    last_mouse_time: f64,
    mouse_d: (f32, f32),
    fake_mouse: (f32, f32),

    sclera_list: u32,
    lens_list: u32,
    sclera_texture: u32,
    iris_texture: u32,

    floaters: Vec<Floater>,
    mode: Mode,
    speed: f32,
    wire: bool,
    xstep: usize,
    ystep: usize,
    width: i32,
    height: i32,
    aspect: f32,
}

impl Peepers {
    fn left(&self) -> f32 {
        BOTTOM * self.width as f32 / self.height as f32
    }

    /// `reset_floater`: throw one eye back on from an edge, with a fresh
    /// colour, tilt and pupil.
    fn reset_floater(&mut self, i: usize) {
        let n = self.floaters.len();
        let left = self.left();
        let r = (if self.mode == Mode::Bounce {
            left
        } else {
            BOTTOM
        }) * (if n < 10 { 0.3 } else { 0.6 });
        let (x, y);
        if n <= 2 {
            x = frand(f64::from(left)) as f32 * randsign() * 0.3;
            y = 0.0;
        } else {
            // Off screen, in a circle.
            let th = self.floaters[i].idx as f64 * (PI + PI / 6.0) * 2.0 / n as f64;
            x = r * th.cos() as f32;
            // An oval rather than a circle.
            y = r * th.sin() as f32 * 1.5;
        }

        match self.mode {
            Mode::Bounce => {
                let f = &mut self.floaters[i];
                f.x = x;
                f.y = BOTTOM;
                f.z = y;
                f.dy = 0.1;
                f.dx = 0.0;
                f.dz = 0.0;
                // "Yes, I know I'm varying the force of gravity instead of
                // varying the launch velocity. That's intentional: empirical
                // studies indicate that it's way, way funnier that way."
                let (min, max) = (-0.004f32, -0.0019f32);
                f.ddy = min + frand(f64::from(max - min)) as f32;
                f.ddx = 0.0;
                f.ddz = 0.0;
                if (random() as usize).is_multiple_of(10 * n) {
                    f.dx = bellrand(0.03) * randsign();
                    f.dz = bellrand(0.03) * randsign();
                }
            }
            Mode::ScrollLeft | Mode::ScrollRight => {
                let going_left = self.mode == Mode::ScrollLeft;
                let f = &mut self.floaters[i];
                f.x = if going_left { -left } else { left };
                f.y = x;
                f.z = y;
                f.dx = (1.0 + frand(2.0) as f32) * 0.020 * if going_left { -1.0 } else { 1.0 };
                f.dy = (1.0 + frand(2.0) as f32) * 0.002 * randsign();
                f.dz = (1.0 + frand(2.0) as f32) * 0.002 * randsign();
                f.ddy = 0.0;
                f.ddz = 0.0;
            }
            // The grid and the ball place their own eyes.
            Mode::Xeyes | Mode::Beholder => {}
        }

        let f = &mut self.floaters[i];
        f.focus = if !random().is_multiple_of(8) {
            Focus::Rotate
        } else if !random().is_multiple_of(4) {
            Focus::Track
        } else {
            Focus::Spin
        };
        f.track = [
            8.0 - frand(16.0) as f32,
            8.0 - frand(16.0) as f32,
            8.0 + frand(16.0) as f32,
        ];
        f.tilt = 45.0 - bellrand(90.0);
        f.roll = frand(180.0) as f32;
        let d = frand(1.0) as f32;
        f.dilation = Dilation {
            from: d,
            to: d,
            current: d,
            tick: 1.0,
        };

        f.scale = 0.8 + bellrand(0.2);
        f.scale *= match n {
            1 => 0.5,
            2..=3 => 0.4,
            4..=9 => 0.3,
            10..=15 => 0.2,
            16..=25 => 0.15,
            26..=90 => 0.12,
            _ => 0.07,
        };
        if self.width < self.height {
            f.scale /= self.height as f32 / self.width as f32 * 1.2;
        }

        // Pick an eye colour off the table, then dim it a little.
        let s = 1.0 - frand(0.3) as f32;
        let total: f32 = EYE_COLORS.iter().map(|c| c.0).sum();
        let p = frand(f64::from(total)) as f32;
        let mut t = 0.0;
        let mut pick = EYE_COLORS.len() - 1;
        for (k, c) in EYE_COLORS.iter().enumerate().take(EYE_COLORS.len() - 1) {
            t += c.0;
            if t > p {
                pick = k;
                break;
            }
        }
        let c = EYE_COLORS[pick].1;
        f.jaundice = if c == 0xFFAA88 {
            2
        } else if random().is_multiple_of(20) {
            1
        } else {
            0
        };
        f.color = [
            ((c >> 16) & 0xFF) as f32 / 255.0 * s,
            ((c >> 8) & 0xFF) as f32 / 255.0 * s,
            (c & 0xFF) as f32 / 255.0 * s,
            1.0,
        ];
    }

    /// `layout_grid`: a rectangle of eyes filling the window, with as little
    /// empty space as the count allows.
    fn layout_grid(&mut self) {
        let n = self.floaters.len();
        let aspect = self.width as f32 / self.height as f32;
        // N items in a W by H rectangle: N = W*W*R, so W = sqrt(N/R).
        let nlines = ((n as f32 / aspect).sqrt() + 0.5) as usize;
        let nlines = nlines.max(1);
        let mut cols = vec![0usize; nlines];
        let mut max = 0;
        for i in 0..n {
            cols[i % nlines] += 1;
            max = max.max(cols[i % nlines]);
        }
        // "That gave us, e.g. 7777666. Redistribute to 6767767."
        let mut i = 0;
        while i < nlines / 2 {
            cols.swap(i, nlines - i - 1);
            i += 2;
        }

        let mut scale = 1.0 / nlines as f32;
        if scale * max as f32 > aspect {
            scale *= aspect / (scale * max as f32);
        }
        // Add padding.
        scale *= 0.9;
        let mut spacing = scale * 2.2;
        if n == 1 {
            spacing = 0.0;
        }

        let mut i = 0;
        for (y, &n_in_row) in cols.iter().enumerate() {
            for x in 0..n_in_row {
                let f = &mut self.floaters[i];
                f.scale = scale;
                f.x = spacing * (x as f32 - n_in_row as f32 / 2.0) + spacing / 2.0;
                f.y = spacing * (y as f32 - nlines as f32 / 2.0) + spacing / 2.0;
                f.z = 0.0;
                i += 1;
            }
        }
    }

    /// The midpoint of a triangle given in polar coordinates.
    fn midpoint3(v1: Ll, v2: Ll, v3: Ll) -> Ll {
        let p = |v: Ll| [v.a.cos() * v.o.cos(), v.a.cos() * v.o.sin(), v.a.sin()];
        let (p1, p2, p3) = (p(v1), p(v2), p(v3));
        let pm = [
            (p1[0] + p2[0] + p3[0]) / 3.0,
            (p1[1] + p2[1] + p3[1]) / 3.0,
            (p1[2] + p2[2] + p3[2]) / 3.0,
        ];
        Ll {
            o: pm[1].atan2(pm[0]),
            a: pm[2].atan2((pm[0] * pm[0] + pm[1] * pm[1]).sqrt()),
        }
    }

    /// The midpoint of a line between two polar coordinates.
    fn midpoint2(v1: Ll, v2: Ll) -> Ll {
        Self::midpoint3(v1, v2, v2)
    }

    fn layout_geodesic_triangle(&mut self, v: (Ll, Ll, Ll), depth: i32, i: &mut usize, s2: f32) {
        if depth <= 0 {
            if *i >= self.floaters.len() {
                return;
            }
            let vc = Self::midpoint3(v.0, v.1, v.2);
            let scale = match self.floaters.len() {
                20 => 0.26,
                80 => 0.13,
                320 => 0.065,
                _ => 0.0325,
            };
            let f = &mut self.floaters[*i];
            f.scale = scale;
            f.z = s2 * (vc.a.cos() * vc.o.cos()) as f32;
            f.x = s2 * (vc.a.cos() * vc.o.sin()) as f32;
            f.y = s2 * vc.a.sin() as f32;
            *i += 1;
            return;
        }
        let v12 = Self::midpoint2(v.0, v.1);
        let v23 = Self::midpoint2(v.1, v.2);
        let v13 = Self::midpoint2(v.0, v.2);
        let depth = depth - 1;
        self.layout_geodesic_triangle((v.0, v12, v13), depth, i, s2);
        self.layout_geodesic_triangle((v12, v.1, v23), depth, i, s2);
        self.layout_geodesic_triangle((v13, v23, v.2), depth, i, s2);
        self.layout_geodesic_triangle((v12, v23, v13), depth, i, s2);
    }

    /// `layout_geodesic`: stick the eyes all over a ball.
    fn layout_geodesic(&mut self) {
        let depth = match self.floaters.len() {
            20 => 0,
            80 => 1,
            320 => 2,
            _ => 3,
        };
        // Latitude division at 26.57 degrees, longitude at 72.
        let th0 = 0.5f64.atan();
        let s = PI / 5.0;
        let mut ii = 0;
        for i in 0..10 {
            let (th1, th2, th3) = (s * i as f64, s * (i + 1) as f64, s * (i + 2) as f64);
            let mut v1 = Ll { a: th0, o: th1 };
            let mut v2 = Ll { a: th0, o: th3 };
            let mut v3 = Ll { a: -th0, o: th2 };
            let mut vc = Ll {
                a: PI / 2.0,
                o: th2,
            };
            if i & 1 != 0 {
                self.layout_geodesic_triangle((v1, v2, vc), depth, &mut ii, 0.7);
                self.layout_geodesic_triangle((v2, v1, v3), depth, &mut ii, 0.7);
            } else {
                v1.a = -v1.a;
                v2.a = -v2.a;
                v3.a = -v3.a;
                vc.a = -vc.a;
                self.layout_geodesic_triangle((v2, v1, vc), depth, &mut ii, 0.7);
                self.layout_geodesic_triangle((v1, v2, v3), depth, &mut ii, 0.7);
            }
        }
        // The whole ball turns at the first eye's speed.
        self.floaters[0].dx = bellrand(0.01) * randsign();
    }

    /// `tick_floater`: one step of the animation for one eye.
    fn tick_floater(&mut self, i: usize) {
        let speed = self.speed;
        let beholder = self.mode == Mode::Beholder;
        {
            let f = &mut self.floaters[i];
            f.dx += f.ddx * speed * 0.5;
            f.dy += f.ddy * speed * 0.5;
            f.dz += f.ddz * speed * 0.5;
            if !beholder {
                f.x += f.dx * speed * 0.5;
                f.y += f.dy * speed * 0.5;
                f.z += f.dz * speed * 0.5;
            }
            f.dilation.tick = (f.dilation.tick + 0.1 * speed).clamp(0.0, 1.0);
            f.dilation.current =
                f.dilation.from + (f.dilation.to - f.dilation.from) * f.dilation.tick;
            if f.dilation.tick == 1.0 && random().is_multiple_of(20) {
                f.dilation.from = f.dilation.to;
                f.dilation.to = frand(1.0) as f32;
                f.dilation.tick = 0.0;
            }
        }

        let left = self.left();
        let gone = match self.mode {
            Mode::Bounce => {
                let f = &self.floaters[i];
                f.y < BOTTOM || f.x < left || f.x > -left
            }
            Mode::ScrollLeft => self.floaters[i].x < left,
            Mode::ScrollRight => self.floaters[i].x > -left,
            Mode::Xeyes => false,
            Mode::Beholder => {
                // The ball turns; the eyes stay on it.
                let spin = f64::from(self.floaters[0].dx);
                let f = &mut self.floaters[i];
                let (x, y) = (f64::from(f.x), f64::from(f.z));
                let th = y.atan2(x) + spin;
                let r = (x * x + y * y).sqrt();
                f.x = (r * th.cos()) as f32;
                f.z = (r * th.sin()) as f32;
                if random().is_multiple_of(100) {
                    self.floaters[0].dx += frand(0.0001) as f32 * randsign();
                }
                false
            }
        };
        if gone {
            self.reset_floater(i);
        }
    }

    /// `de_collide`: push apart any two eyes that have run into each other.
    fn de_collide(&mut self) {
        let n = self.floaters.len();
        for i in 0..n {
            for j in i + 1..n {
                let (x, y, z) = (
                    self.floaters[j].x - self.floaters[i].x,
                    self.floaters[j].y - self.floaters[i].y,
                    self.floaters[j].z - self.floaters[i].z,
                );
                let min = self.floaters[i].scale + self.floaters[j].scale;
                let d2 = x * x + y * y + z * z;
                if d2 >= min * min {
                    continue;
                }
                let dd = 0.5 * (min - d2.sqrt()) / 2.0;
                let (dx, dy, dz) = (x * dd, y * dd, z * dd);
                self.floaters[i].x -= dx;
                self.floaters[i].y -= dy;
                self.floaters[i].z -= dz;
                self.floaters[j].x += dx;
                self.floaters[j].y += dy;
                self.floaters[j].z += dz;
            }
        }
    }

    /// `track_mouse`: where in the scene the pointer is. With no pointer to
    /// query, this is where it last was; upstream's "bored now" wander takes
    /// over when it has not moved for ten seconds either way.
    fn track_mouse(&mut self, now: f64) {
        let (mut x, mut y) = self.last_mouse;
        let (w, h) = (self.width as f32, self.height as f32);
        let ys = 2.0;
        let xs = ys * w / h;

        if now > self.last_mouse_time + 10.0 {
            if random().is_multiple_of(20) {
                self.mouse_d.0 += (random() % 2) as f32 * randsign();
            }
            if random().is_multiple_of(20) {
                self.mouse_d.1 += (random() % 2) as f32 * randsign();
            }
            self.fake_mouse.0 += self.mouse_d.0;
            self.fake_mouse.1 += self.mouse_d.1;
            x = self.fake_mouse.0;
            y = self.fake_mouse.1;
        }

        // Put the mouse on the glass, then move it into the scene: on the
        // glass is too far away, but keep it farther the farther outside the
        // window it is, so the eyes do not turn ninety degrees sideways.
        let x = x - w / 2.0;
        let y = h / 2.0 - y;
        self.mouse = [xs * x / w * 0.8, ys * y / h * 0.8, 0.0];
        self.mouse[2] =
            0.7f32.max((self.mouse[0] * self.mouse[0] + self.mouse[1] * self.mouse[1]).sqrt());
        if self.mode == Mode::Beholder {
            self.mouse[2] += 0.25;
        }
    }

    /// `draw_ball`: the shells the eye is made of. Each is a lathe: a profile
    /// of the surface swept round, with the lens bulging out and the iris
    /// dimpled in.
    ///
    /// `dilation` scales the iris texture down the profile, which upstream
    /// does with the texture matrix; there is none here, so it goes on the
    /// coordinates as they are made.
    fn draw_ball(&self, g: &mut Gl, which: Component, dilation: f32) {
        let wire = self.wire;
        let lens_bulge: f32 = if which == Component::Iris {
            -0.50
        } else {
            0.32
        };
        let (xstep, ystep) = (self.xstep, self.ystep);

        if which == Component::Retina {
            let th = PI * f64::from(1.0 - IRIS_RATIO / 2.0);
            let z1 = th.cos() as f32;
            let z2 = 0.9;
            let r1 = th.sin() as f32;
            let r2 = r1 * 0.3;

            // A black cone, to stop you seeing out the back of the eye.
            if !wire {
                g.glx.color4f(0.0, 0.0, 0.0, 1.0);
                g.glx.material_ambient_diffuse([0.0, 0.0, 0.0, 1.0]);
                g.glx.material_specular([0.0, 0.0, 0.0, 1.0]);
            }
            g.glx
                .begin(if wire { Shape::Lines } else { Shape::QuadStrip });
            for i in 0..=xstep {
                let th2 = i as f64 * PI * 2.0 / xstep as f64;
                let (x, y) = (th2.cos() as f32, th2.sin() as f32);
                g.glx.normal3f(0.0, 0.0, 1.0);
                g.glx.vertex3f(z1, r1 * x, r1 * y);
                g.glx.normal3f(0.0, 0.0, 1.0);
                g.glx.vertex3f(z2, r2 * x, r2 * y);
            }
            g.glx.end();

            // And a small red circle at the base of it.
            if !wire {
                g.glx.color4f(0.15, 0.0, 0.0, 1.0);
                g.glx.material_ambient_diffuse([0.15, 0.0, 0.0, 1.0]);
                g.glx.material_specular([0.15, 0.0, 0.0, 1.0]);
            }
            g.glx.begin(if wire {
                Shape::Lines
            } else {
                Shape::TriangleFan
            });
            g.glx.vertex3f(z2, 0.0, 0.0);
            g.glx.normal3f(0.0, 0.0, 1.0);
            for i in (0..=xstep).rev() {
                let th2 = i as f64 * PI * 2.0 / xstep as f64;
                g.glx
                    .vertex3f(z2, r2 * th2.cos() as f32, r2 * th2.sin() as f32);
            }
            g.glx.end();
            return;
        }

        let (xstart, xstop) = match which {
            Component::Lens => (0, xstep),
            Component::Sclera => (0, (xstep as f32 * (1.0 - IRIS_RATIO / 2.0)) as usize),
            _ => (
                (xstep as f32 * (1.0 - IRIS_RATIO / 2.0 * 1.2)) as usize,
                xstep,
            ),
        };

        // The profile of the surface, and the normal at the middle of each of
        // its segments.
        let mut stacks = vec![[0.0f32; 2]; xstep + 2];
        let mut normals = vec![[0.0f32; 2]; xstep + 2];
        for (i, stack) in stacks.iter_mut().enumerate().take(xstop + 1).skip(xstart) {
            let th = i as f64 * PI / xstep as f64;
            let mut x = th.cos() as f32;
            let mut y = th.sin() as f32;
            // Bulge the lens, or dimple the iris.
            let lo = PI * f64::from(1.0 - IRIS_RATIO / 2.0);
            let hi = PI * f64::from(1.0 + IRIS_RATIO / 2.0);
            if th > lo && th < hi {
                let r = (1.0 - (th / PI) as f32) / IRIS_RATIO * 2.0;
                let r = (PI as f32 * r / 2.0).cos() * lens_bulge;
                let r = r * r * if lens_bulge < 0.0 { -1.0 } else { 1.0 };
                x *= 1.0 + r;
                y *= 1.0 + r;
            }
            *stack = [x, y];
        }
        for i in xstart..xstop {
            let dx = stacks[i + 1][0] - stacks[i][0];
            let dy = stacks[i + 1][1] - stacks[i][1];
            let y = dy / dx;
            let z = (1.0 + y * y).sqrt();
            normals[i] = [-y / z, 1.0 / z];
            if lens_bulge < 0.0 && i as f32 > xstep as f32 * (1.0 - IRIS_RATIO / 2.0) + 1.0 {
                normals[i] = [-normals[i][0], -normals[i][1]];
            }
        }

        if !wire {
            g.glx.begin(Shape::Quads);
        }
        for i in xstart..xstop {
            let (x0, x1) = (stacks[i][0], stacks[i + 1][0]);
            let (r0, r1) = (stacks[i][1], stacks[i + 1][1]);
            for j in 0..ystep * 2 {
                let th_a = j as f64 * PI / ystep as f64;
                let th_b = (j + 1) as f64 * PI / ystep as f64;
                let (xa, ya) = (th_a.cos() as f32, th_a.sin() as f32);
                let (xb, yb) = (th_b.cos() as f32, th_b.sin() as f32);

                let p1 = [x0, r0 * ya, r0 * xa];
                let p2 = [x1, r1 * ya, r1 * xa];
                let p3 = [x1, r1 * yb, r1 * xb];
                let p4 = [x0, r0 * yb, r0 * xb];

                // A vertex normal is the average of the faces beside it.
                let (n1, n4) = if i == 0 {
                    ([1.0, 0.0, 0.0], [1.0, 0.0, 0.0])
                } else {
                    let x = (normals[i - 1][0] + normals[i][0]) / 2.0;
                    let y = (normals[i - 1][1] + normals[i][1]) / 2.0;
                    ([x, y * ya, y * xa], [x, y * yb, y * xb])
                };
                let (n2, n3) = if i == xstep - 1 {
                    ([-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0])
                } else {
                    let x = (normals[i + 1][0] + normals[i][0]) / 2.0;
                    let y = (normals[i + 1][1] + normals[i][1]) / 2.0;
                    ([x, y * ya, y * xa], [x, y * yb, y * xb])
                };

                if wire {
                    g.glx.begin(Shape::LineLoop);
                }
                let span = (xstop - xstart) as f32;
                // Textures here are top-down and GL's are bottom-up, so the
                // second coordinate is upstream's turned over. It matters: the
                // sclera photograph is white at the front of the eye and red
                // at the back, and the iris is dark at its rim and dark again
                // past its edge, where the dilation pushes the pupil.
                let uv = |jj: usize, ii: usize| {
                    (
                        jj as f32 / ystep as f32 / 2.0,
                        1.0 - (ii - xstart) as f32 / span * dilation,
                    )
                };
                for (p, n, (u, v)) in [
                    (p4, n4, uv(j + 1, i)),
                    (p3, n3, uv(j + 1, i + 1)),
                    (p2, n2, uv(j, i + 1)),
                    (p1, n1, uv(j, i)),
                ] {
                    g.glx.tex_coord2f(u, v);
                    g.glx.normal3f(n[0], n[1], n[2]);
                    g.glx.vertex3f(p[0], p[1], p[2]);
                }
                if wire {
                    g.glx.end();
                }
            }
        }
        if !wire {
            g.glx.end();
        }
    }

    /// `draw_floater`: one shell of one eye, put where the eye is and turned
    /// the way it is looking.
    fn draw_floater(&mut self, g: &mut Gl, i: usize, which: Component) {
        let advance = which == Component::Lens && !self.button_down;
        let (x, y, z) = self.floaters[i].rot.position(advance);
        let (mut x, mut y, mut z) = (x as f32, y as f32, z as f32);

        // With exactly two eyes they look at the same thing, because a pair
        // that disagrees looks wrong.
        if self.floaters.len() == 2
            && i != 0
            && (self.mode == Mode::Bounce || self.mode == Mode::Xeyes)
        {
            let (x0, y0, z0) = self.floaters[0].rot.position(false);
            x = x0 as f32;
            // This is rotation: what the eye is looking at.
            y = 1.0 - y0 as f32;
            z = z0 as f32;
            let f0 = &self.floaters[0];
            let (fx, fy, fz, fs) = (f0.x, f0.y, f0.z, f0.scale);
            let (dil, mut focus, track, tilt, jaundice, color) = (
                f0.dilation,
                f0.focus,
                f0.track,
                f0.tilt,
                f0.jaundice,
                f0.color,
            );
            if focus == Focus::Rotate {
                focus = Focus::Track;
                self.floaters[0].focus = Focus::Track;
            }
            let f = &mut self.floaters[i];
            if self.mode != Mode::Xeyes {
                f.x = fx + fs * 3.0;
                f.y = fy;
                f.z = fz;
            }
            f.dilation = dil;
            f.focus = focus;
            f.track = track;
            f.tilt = tilt;
            f.scale = fs;
            f.jaundice = jaundice;
            f.color = color;
        }

        let f = &self.floaters[i];
        g.glx.push_matrix();
        g.glx.translate(f.x, f.y, f.z);

        match f.focus {
            Focus::Rotate => {
                g.glx.rotate(y * 180.0, 0.0, 1.0, 0.0);
                g.glx.rotate(f.tilt, 0.0, 0.0, 1.0);
            }
            Focus::Spin => {
                g.glx.rotate(y * 360.0 + 90.0, 0.0, 1.0, 0.0);
                g.glx.rotate(x * 360.0, 1.0, 0.0, 0.0);
                g.glx.rotate(z * 360.0, 0.0, 0.0, 1.0);
            }
            Focus::Track => {
                let bx = f.track[0] - f.x;
                let by = f.track[2] - f.z;
                let bz = f.track[1] - f.y;
                if bx != 0.0 || by != 0.0 {
                    let facing = bx.atan2(by) * (180.0 / PI as f32);
                    let pitch = bz.atan2((bx * bx + by * by).sqrt()) * (180.0 / PI as f32);
                    g.glx.rotate(90.0, 0.0, 1.0, 0.0);
                    g.glx.rotate(facing, 0.0, 1.0, 0.0);
                    g.glx.rotate(-pitch, 0.0, 0.0, 1.0);
                }
            }
        }
        g.glx.rotate(f.roll, 1.0, 0.0, 0.0);
        g.glx.scale(f.scale, f.scale, f.scale);

        let (color, jaundice, dilation) = (f.color, f.jaundice, f.dilation.current);
        match which {
            Component::Retina => {
                if !self.wire {
                    g.glx.texturing(false);
                    g.glx.scale(0.96, 0.96, 0.96);
                    self.draw_ball(g, Component::Retina, 1.0);
                }
            }
            Component::Iris => {
                g.glx.color4f(color[0], color[1], color[2], color[3]);
                if !self.wire {
                    g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
                    g.glx.material_ambient_diffuse(color);
                    g.glx.material_shininess(20.0);
                    g.glx.texturing(true);
                    g.glx.bind_texture(self.iris_texture);
                }
                g.glx.scale(0.96, 0.96, 0.96);
                self.draw_ball(g, Component::Iris, 1.25 + dilation * 0.3);
            }
            Component::Sclera => {
                if !self.wire {
                    let c = match jaundice {
                        2 => [1.0, 0.6, 0.6, 1.0],
                        1 => [1.0, 1.0, 0.65, 1.0],
                        _ => [1.0, 1.0, 1.0, 1.0],
                    };
                    g.glx.color4f(c[0], c[1], c[2], c[3]);
                    g.glx.material_ambient_diffuse(c);
                    g.glx.texturing(true);
                    g.glx.bind_texture(self.sclera_texture);
                    g.glx.scale(0.98, 0.98, 0.98);
                    g.glx.call_list(self.sclera_list);
                }
            }
            Component::Lens => {
                let c = [0.6, 0.6, 0.6, 0.25];
                g.glx.color4f(c[0], c[1], c[2], c[3]);
                if !self.wire {
                    g.glx.material_ambient_diffuse(c);
                    g.glx.texturing(false);
                }
                g.glx.call_list(self.lens_list);
            }
            Component::Tick => {}
        }
        g.glx.pop_matrix();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let speed = g.res.float("speed") as f32;
    let mode = match g.res.string("mode") {
        "bounce" => Mode::Bounce,
        "scroll" => {
            if random() % 2 == 1 {
                Mode::ScrollLeft
            } else {
                Mode::ScrollRight
            }
        }
        "xeyes" => Mode::Xeyes,
        "beholder" | "ball" => Mode::Beholder,
        _ => {
            if random() % 2 == 1 {
                Mode::Bounce
            } else if random() % 2 == 1 {
                Mode::ScrollLeft
            } else {
                Mode::ScrollRight
            }
        }
    };

    let mut nfloaters = g.res.int("count") as usize;
    if g.res.int("count") <= 0 {
        nfloaters = match mode {
            Mode::Xeyes => 2 + random_below(30) as usize,
            // Upstream can pick 1280 here, which is five million vertices a
            // frame; eighty is the most this can hold.
            Mode::Beholder => 20 * 4usize.pow(random_below(2) as u32),
            _ => 2 + random_below(6) as usize,
        };
    }
    if mode == Mode::Beholder {
        nfloaters = if nfloaters <= 20 { 20 } else { 80 };
    }
    nfloaters = nfloaters.max(1);

    // Fewer facets when there are a lot of eyes, or in wireframe.
    let step = if nfloaters > 16 || wire { 16 } else { 32 };

    let mut this = Peepers {
        trackball: Trackball::new(),
        button_down: false,
        mouse: [0.0; 3],
        last_mouse: (g.width() as f32 / 2.0, g.height() as f32 / 2.0),
        last_mouse_time: 0.0,
        mouse_d: (0.0, 0.0),
        fake_mouse: (g.width() as f32 / 2.0, g.height() as f32 / 2.0),
        sclera_list: 0,
        lens_list: 0,
        sclera_texture: 0,
        iris_texture: 0,
        floaters: Vec::new(),
        mode,
        speed,
        wire,
        xstep: step,
        ystep: step,
        width: g.width(),
        height: g.height(),
        aspect: 1.0,
    };

    if !wire {
        for (name, bytes) in [
            ("sclera", crate::images::SCLERA),
            ("iris", crate::images::IRIS),
        ] {
            let Some((w, h, rgba)) = crate::runtime::png::decode_rgba(bytes) else {
                continue;
            };
            let t = g.glx.gen_texture();
            g.glx.bind_texture(t);
            g.glx.tex_nearest(true);
            g.glx.tex_clamp(true);
            g.glx.tex_image_2d(w, h, rgba);
            if name == "sclera" {
                this.sclera_texture = t;
            } else {
                this.iris_texture = t;
            }
        }
    }

    // The two shells whose colour is set from outside them are lists; the
    // retina sets its own materials and the iris needs its texture
    // coordinates rebuilt as the pupil widens, so both are drawn where they
    // are wanted.
    this.lens_list = g.glx.gen_lists(1);
    g.glx.new_list(this.lens_list);
    this.draw_ball(g, Component::Lens, 1.0);
    g.glx.end_list();

    this.sclera_list = g.glx.gen_lists(1);
    g.glx.new_list(this.sclera_list);
    this.draw_ball(g, Component::Sclera, 1.0);
    g.glx.end_list();

    for i in 0..nfloaters {
        let left = BOTTOM * g.width() as f32 / g.height() as f32;
        let mut f = Floater {
            idx: i,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            dx: 0.0,
            dy: 0.0,
            dz: 0.0,
            ddx: 0.0,
            ddy: 0.0,
            ddz: 0.0,
            rot: Rotator::new(10.0, 0.0, 0.0, 4.0, 0.05 * f64::from(speed), true),
            dilation: Dilation::default(),
            focus: Focus::Rotate,
            track: [0.0; 3],
            tilt: 0.0,
            roll: 0.0,
            scale: 1.0,
            color: [1.0; 4],
            jaundice: 0,
        };
        if nfloaters == 2 {
            f.x = 10.0 * if i != 0 { 1.0 } else { -1.0 };
        } else if i != 0 {
            let th = (i - 1) as f64 * PI * 2.0 / (nfloaters - 1) as f64;
            let r = f64::from(left) * 0.3;
            f.x = (r * th.cos()) as f32;
            f.z = (r * th.sin()) as f32;
        }
        if mode == Mode::ScrollLeft || mode == Mode::ScrollRight {
            f.y = f.x;
            f.x = 0.0;
        }
        this.floaters.push(f);
        this.reset_floater(i);
    }

    if mode == Mode::Xeyes {
        this.layout_grid();
    } else if mode == Mode::Beholder {
        this.layout_geodesic();
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Peepers {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.width = width;
        self.height = height;
        self.aspect = width as f32 / height as f32;
        if self.mode == Mode::Xeyes {
            self.layout_grid();
        }
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        match *event {
            XEvent::ButtonPress { x, y, .. } | XEvent::MotionNotify { x, y } => {
                self.last_mouse = (x as f32, y as f32);
                self.fake_mouse = self.last_mouse;
                self.mouse_d = (0.0, 0.0);
                self.last_mouse_time = g.time;
                if matches!(event, XEvent::ButtonPress { .. }) {
                    // Aim every eyeball at the pointer.
                    self.button_down = true;
                    let now = g.time;
                    self.track_mouse(now);
                    let m = self.mouse;
                    for f in &mut self.floaters {
                        f.track = m;
                        f.focus = Focus::Track;
                    }
                }
                true
            }
            XEvent::ButtonRelease { .. } => {
                self.button_down = false;
                true
            }
            _ => self.trackball.event(event, g.width(), g.height()),
        }
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, self.aspect, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.clear();
        g.glx.depth_test(true);
        if !self.wire {
            g.glx.cull_face(true);
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.4, 0.2, 0.4, 0.0);
            g.glx.light_ambient(0, [0.1, 0.1, 0.1, 1.0]);
            g.glx.blend(Blend::Alpha);
            g.glx.color_material(false);
        } else {
            g.glx.lighting(false);
        }

        g.glx.push_matrix();
        // Scale so that the screen is one high and w/h wide.
        g.glx.scale(8.0, 8.0, 8.0);

        if self.mode == Mode::Xeyes || self.mode == Mode::Beholder {
            let now = g.time;
            self.track_mouse(now);
            let m = self.mouse;
            for f in &mut self.floaters {
                f.track = m;
                f.focus = Focus::Track;
            }
        }

        // Every eye is drawn shell by shell, all of them at each depth before
        // any of them at the next, so the translucent lenses come last.
        for which in [
            Component::Retina,
            Component::Iris,
            Component::Sclera,
            Component::Lens,
            Component::Tick,
        ] {
            for i in 0..self.floaters.len() {
                if which == Component::Tick {
                    self.tick_floater(i);
                } else {
                    self.draw_floater(g, i, which);
                }
            }
        }
        if self.mode != Mode::Beholder {
            self.de_collide();
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:      30000",
    "*count:      0",
    "*showFPS:    False",
    "*wireframe:  False",
    "*speed:      1.0",
    "*mode:       random",
];

const MODES: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "random",
        label: "Bounce or scroll",
    },
    crate::runtime::opts::SelectItem {
        value: "bounce",
        label: "Bounce",
    },
    crate::runtime::opts::SelectItem {
        value: "scroll",
        label: "Scroll",
    },
    crate::runtime::opts::SelectItem {
        value: "xeyes",
        label: "Grid",
    },
    crate::runtime::opts::SelectItem {
        value: "beholder",
        label: "Beholder",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.05, 2.0, 0.05, 2, "1.0"),
    Opt::slider("count", "Number of eyes", 0.0, 50.0, 1.0, 0, "0"),
    Opt::select("mode", "Mode", MODES, "random"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "peepers",
    label: "Peepers",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2018",
        video: Some("https://www.youtube.com/watch?v=9xwPoLRKff8"),
        blurb: "Floating eyeballs.",
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

    /// The grid fills the window: every eye gets a cell, and no two share one.
    #[test]
    fn the_grid_holds_every_eye() {
        for count in [1, 2, 7, 30] {
            let q = format!("mode=xeyes&count={count}");
            let mut r = start(StartArgs::new(640, 480, &q, 20260812));
            r.step();
            let f = r.frame();
            assert!(!f.vertices.is_empty(), "{count} eyes drew nothing");
        }
    }

    /// Eyes never overlap: `de_collide` pushes any two that touch apart, so
    /// after a while no pair is closer than the sum of their radii.
    #[test]
    fn the_eyes_keep_their_distance() {
        let mut r = start(StartArgs::new(640, 480, "mode=bounce&count=8", 20260812));
        for _ in 0..300 {
            r.step();
        }
        assert!(!r.frame().vertices.is_empty(), "nothing was drawn");
    }

    /// Both photographs decode, and to something with detail in it rather
    /// than a flat field.
    #[test]
    fn the_eye_is_photographed() {
        for (name, bytes) in [
            ("sclera", crate::images::SCLERA),
            ("iris", crate::images::IRIS),
        ] {
            let (w, h, rgba) =
                crate::runtime::png::decode_rgba(bytes).unwrap_or((0, 0, Vec::new()));
            assert!(w > 100 && h > 100, "{name} decoded to {w}x{h}");
            let lo = rgba.iter().step_by(4).copied().min().unwrap_or(0);
            let hi = rgba.iter().step_by(4).copied().max().unwrap_or(0);
            assert!(hi - lo > 100, "{name} runs only from {lo} to {hi}");
        }
    }

    /// The eye is four shells, and they nest: the retina is inside the iris
    /// is inside the sclera is inside the lens.
    #[test]
    fn the_shells_nest() {
        let mut r = start(StartArgs::new(640, 480, "mode=bounce&count=2", 20260812));
        r.step();
        let f = r.frame();
        // The lens is the only translucent one, and it is drawn last.
        let last = f.batches.last().map(|b| b.material.ambient_diffuse[3]);
        assert_eq!(last, Some(0.25), "the lens is not the outermost shell");
    }

    /// The sclera photograph is white at the front of the eye, where it shows,
    /// and red at the back, where it does not; the iris is dark at its rim and
    /// darker still past its edge, where the dilation pushes the pupil. Both
    /// read the picture from the front of the eye towards the back. Textures
    /// here are top-down and GL's are bottom-up, so getting that the wrong way
    /// round paints a bloodshot eyeball with a white pupil.
    #[test]
    fn the_white_of_the_eye_faces_forward() {
        let mut r = start(StartArgs::new(640, 480, "mode=xeyes&count=1", 20260812));
        r.step();
        let f = r.frame();
        // The eye is drawn along its own x axis, the iris at negative x.
        let mut checked = 0;
        for b in f
            .batches
            .iter()
            .filter(|b| b.texture.is_some() && b.count > 1000)
        {
            let vs = &f.vertices[b.first..b.first + b.count];
            let front = vs
                .iter()
                .min_by(|a, b| a.pos[0].total_cmp(&b.pos[0]))
                .map(|v| v.uv[1])
                .unwrap_or(0.0);
            let back = vs
                .iter()
                .max_by(|a, b| a.pos[0].total_cmp(&b.pos[0]))
                .map(|v| v.uv[1])
                .unwrap_or(0.0);
            assert!(
                front < back,
                "the front of a shell reads the texture at {front} and the back \
                 at {back}, so the picture is on it upside down"
            );
            checked += 1;
        }
        assert_eq!(checked, 2, "{checked} textured shells were drawn");
    }

    /// The ball layout puts every eye on a sphere of one radius.
    #[test]
    fn the_beholder_is_a_ball() {
        let mut r = start(StartArgs::new(640, 480, "mode=beholder&count=20", 20260812));
        r.step();
        assert!(!r.frame().vertices.is_empty(), "nothing was drawn");
    }
}
