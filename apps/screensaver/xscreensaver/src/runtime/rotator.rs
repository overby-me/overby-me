//! Port of `hacks/glx/rotator.c` and `hacks/glx/gltrackball.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1998-2016 Jamie Zawinski <jwz@jwz.org>
//! gltrackball, Copyright © 2002-2026 Jamie Zawinski <jwz@jwz.org>
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
//! Two things nearly every OpenGL saver has: something that turns the object
//! over slowly on its own, and something that lets you turn it over yourself.
//!
//! The [`Rotator`] is the first. It is not a constant spin: each axis has a
//! velocity and an acceleration, the acceleration flips sign now and then, and
//! when an axis stops it usually stays stopped for a while. That is why an
//! object rotating under it never quite repeats. Upstream's comments in
//! `rotate_1` are worth keeping, because the sign convention it settled on is
//! genuinely confusing and the comments say so.
//!
//! The [`Trackball`] is the second: the SGI virtual trackball from 1993, a
//! sphere in the middle of the window deforming into a hyperbolic sheet
//! further out, so a drag near the edge spins rather than tumbles. Letting go
//! keeps it going and damps it out over about three seconds.

use super::rand::{frand, random};

/// Three samples averaged, so values near the middle are commoner than values
/// near the ends. Upstream's `BELLRAND`.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

fn randsign() -> f64 {
    if random() & 1 != 0 { 1.0 } else { -1.0 }
}

/// Stay in the range [0-1). `1.01` becomes `0.01`, `-0.01` becomes `0.99`.
fn clamp(i: &mut f64) {
    while *i < 0.0 {
        *i += 1.0;
    }
    while *i >= 1.0 {
        *i -= 1.0;
    }
}

/// One axis of the rotation: where it is, how fast, and how that is changing.
struct Axis {
    /// Current rotation, -1 to +1. The *sign* is the direction of motion, not
    /// part of the angle: 0.25 means +90 degrees going forwards and -0.25
    /// means +90 degrees going backwards. Upstream's comment on this is "Yes,
    /// this is stupid", and it is kept because the arithmetic depends on it.
    pos: f64,
    /// Velocity, always positive.
    v: f64,
    /// Acceleration, either sign.
    dv: f64,
    speed: f64,
}

impl Axis {
    fn tick(&mut self, max_v: f64) {
        if self.speed == 0.0 {
            return;
        }
        let mut ppos = self.pos;

        /* tick position */
        if ppos < 0.0 {
            /* Ignore but preserve the sign on ppos. */
            ppos = -(ppos + self.v);
        } else {
            ppos += self.v;
        }
        clamp(&mut ppos);
        self.pos = if self.pos > 0.0 { ppos } else { -ppos };

        /* accelerate */
        self.v += self.dv;

        /* clamp velocity */
        if self.v > max_v || self.v < -max_v {
            self.dv = -self.dv;
        } else if self.v < 0.0 {
            /* If it stops, start it going in the other direction. */
            if !random().is_multiple_of(4) {
                self.v = 0.0; /* don't let velocity be negative */
                if random().is_multiple_of(2) {
                    /* stay stopped, and kill acceleration */
                    self.dv = 0.0;
                } else if self.dv < 0.0 {
                    /* was decelerating, accelerate instead */
                    self.dv = -self.dv;
                }
            } else {
                self.v = -self.v; /* switch to tiny positive velocity, or zero */
                self.dv = -self.dv; /* toggle acceleration */
                self.pos = -self.pos; /* reverse direction of motion */
            }
        }

        /* Alter direction of rotational acceleration randomly. */
        if random().is_multiple_of(120) {
            self.dv = -self.dv;
        }

        /* Change acceleration very occasionally. */
        if random().is_multiple_of(200) {
            if self.dv == 0.0 {
                self.dv = 0.00001;
            } else if random() & 1 != 0 {
                self.dv *= 1.2;
            } else {
                self.dv *= 0.8;
            }
        }
    }
}

/// Rotation and motion state: how an object turns over when nobody is touching
/// it, and where it wanders to.
pub struct Rotator {
    x: Axis,
    y: Axis,
    z: Axis,
    d_max: f64,
    wander_speed: f64,
    wander_frame: u32,
}

impl Rotator {
    /// `make_rotator`.
    ///
    /// The spin speeds are relative, and zero means that axis does not turn.
    /// `spin_accel` scales how fast the speed itself changes.
    /// `randomize` starts it at a random attitude rather than square on.
    pub fn new(
        spin_x: f64,
        spin_y: f64,
        spin_z: f64,
        spin_accel: f64,
        wander_speed: f64,
        randomize: bool,
    ) -> Self {
        let (rotx, roty, rotz, wander_frame) = if randomize {
            /* Sign on position is direction of travel. Stripped before returned. */
            (
                frand(1.0) * randsign(),
                frand(1.0) * randsign(),
                frand(1.0) * randsign(),
                random() % 0xFFFF,
            )
        } else {
            (0.0, 0.0, 0.0, 0)
        };

        let d = 0.006;
        let dd = 0.00006;
        let dx = bellrand(d * spin_x);
        let x = Axis {
            pos: rotx,
            v: dx,
            dv: (dd + frand(dd + dd)) * spin_x * spin_accel,
            speed: spin_x,
        };
        let y = Axis {
            pos: roty,
            v: bellrand(d * spin_y),
            dv: (dd + frand(dd + dd)) * spin_y * spin_accel,
            speed: spin_y,
        };
        let z = Axis {
            pos: rotz,
            v: bellrand(d * spin_z),
            dv: (dd + frand(dd + dd)) * spin_z * spin_accel,
            speed: spin_z,
        };
        Rotator {
            // The ceiling comes from the x axis alone, even for savers that
            // only spin about y or z. Upstream's, and it means an axis whose
            // speed is zero gives every other axis a ceiling of zero too.
            d_max: dx * 2.0,
            x,
            y,
            z,
            wander_speed,
            wander_frame,
        }
    }

    /// `get_rotation`: where the object is now, as three fractions of a turn.
    /// Pass `update` false while the user is dragging it.
    pub fn rotation(&mut self, update: bool) -> (f64, f64, f64) {
        if update {
            self.x.tick(self.d_max);
            self.y.tick(self.d_max);
            self.z.tick(self.d_max);
        }
        (self.x.pos.abs(), self.y.pos.abs(), self.z.pos.abs())
    }

    /// `get_position`: where the object has wandered to, each 0 to 1. Dead
    /// centre if it was made with no wander speed.
    pub fn position(&mut self, update: bool) -> (f64, f64, f64) {
        if self.wander_speed == 0.0 {
            return (0.5, 0.5, 0.5);
        }
        if update {
            self.wander_frame += 1;
        }
        let sinoid = |f: f64| {
            (1.0 + (f64::from(self.wander_frame) * f / 2.0 * std::f64::consts::PI).sin()) / 2.0
        };
        (
            sinoid(0.71 * self.wander_speed),
            sinoid(0.53 * self.wander_speed),
            sinoid(0.37 * self.wander_speed),
        )
    }
}

/// A unit quaternion, in `hacks/glx/quaternion.c`'s field order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    fn normalize(self) -> Quat {
        let s = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if s == 0.0 {
            return Quat::IDENTITY;
        }
        Quat {
            x: self.x / s,
            y: self.y / s,
            z: self.z / s,
            w: self.w / s,
        }
    }

    /// `quat_mult`. Not the textbook Hamilton product: upstream's signs are its
    /// own, and the trackball is built on them.
    fn mult(self, o: Quat) -> Quat {
        Quat {
            x: self.w * o.x + self.x * o.w - self.y * o.z - self.z * o.y,
            y: self.w * o.y + self.x * o.z + self.y * o.w - self.z * o.x,
            z: self.w * o.z - self.x * o.y + self.y * o.x - self.z * o.w,
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
        }
    }

    /// `axis_to_quat`: a rotation of `phi` radians about an axis.
    fn from_axis(a: [f64; 3], phi: f64) -> Quat {
        let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        if len == 0.0 {
            return Quat::IDENTITY;
        }
        let s = (phi / 2.0).sin() / len;
        Quat {
            x: a[0] * s,
            y: a[1] * s,
            z: a[2] * s,
            w: (phi / 2.0).cos(),
        }
    }

    /// `quat_to_rotmatrix`, in the column-major order `glMultMatrixf` wants.
    pub fn to_matrix(self) -> crate::runtime::gl::Mat4 {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        crate::runtime::gl::Mat4([
            (1.0 - 2.0 * (y * y + z * z)) as f32,
            (2.0 * (x * y - z * w)) as f32,
            (2.0 * (z * x + y * w)) as f32,
            0.0,
            (2.0 * (x * y + z * w)) as f32,
            (1.0 - 2.0 * (z * z + x * x)) as f32,
            (2.0 * (y * z - x * w)) as f32,
            0.0,
            (2.0 * (z * x - y * w)) as f32,
            (2.0 * (y * z + x * w)) as f32,
            (1.0 - 2.0 * (y * y + x * x)) as f32,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }
}

/// How big the virtual ball is, in units where the window is 2 across.
const TRACKBALL_SIZE: f64 = 0.8;

/// Project x,y onto a sphere of radius r, or onto a hyperbolic sheet if we are
/// away from the centre of it. This is what makes a drag near the edge of the
/// window spin the object in the plane of the screen rather than tumble it.
fn project_to_sphere(r: f64, x: f64, y: f64) -> f64 {
    let d = (x * x + y * y).sqrt();
    if d < r * std::f64::consts::FRAC_1_SQRT_2 {
        /* Inside sphere */
        (r * r - d * d).sqrt()
    } else {
        /* On hyperbola */
        let t = r / std::f64::consts::SQRT_2;
        t * t / d
    }
}

/// `trackball`: the rotation that dragging from one point to another means.
/// All four coordinates are in the range -1 to 1.
fn trackball(p1x: f64, p1y: f64, p2x: f64, p2y: f64) -> Quat {
    if p1x == p2x && p1y == p2y {
        return Quat::IDENTITY;
    }
    let p1 = [p1x, p1y, project_to_sphere(TRACKBALL_SIZE, p1x, p1y)];
    let p2 = [p2x, p2y, project_to_sphere(TRACKBALL_SIZE, p2x, p2y)];

    /* Axis of rotation is the cross product of the two. */
    let a = [
        p2[1] * p1[2] - p2[2] * p1[1],
        p2[2] * p1[0] - p2[0] * p1[2],
        p2[0] * p1[1] - p2[1] * p1[0],
    ];

    let d = [p1[0] - p2[0], p1[1] - p2[1], p1[2] - p2[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let t = (len / (2.0 * TRACKBALL_SIZE)).clamp(-1.0, 1.0);
    Quat::from_axis(a, 2.0 * t.asin())
}

/// Dragging the mouse turns the object over, and letting go keeps it going.
pub struct Trackball {
    ow: i32,
    oh: i32,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    ddx: f64,
    ddy: f64,
    q: Quat,
    button_down: bool,
}

impl Default for Trackball {
    fn default() -> Self {
        Self::new()
    }
}

impl Trackball {
    pub fn new() -> Self {
        Trackball {
            ow: 0,
            oh: 0,
            x: 0.0,
            y: 0.0,
            dx: 0.0,
            dy: 0.0,
            ddx: 0.0,
            ddy: 0.0,
            q: Quat::IDENTITY,
            button_down: false,
        }
    }

    /// `gltrackball_reset`, with an optional initial rotation.
    pub fn reset(&mut self, x: f64, y: f64) {
        let down = self.button_down;
        *self = Trackball::new();
        self.button_down = down;
        self.q = trackball(0.0, 0.0, x, y);
    }

    fn start(&mut self, x: i32, y: i32) {
        self.x = f64::from(x);
        self.y = f64::from(y);
        self.button_down = true;
        self.dx = 0.0;
        self.ddx = 0.0;
        self.dy = 0.0;
        self.ddy = 0.0;
    }

    fn stop(&mut self) {
        self.button_down = false;
    }

    fn track_1(&mut self, x: f64, y: f64, w: i32, h: i32) {
        let (w, h) = (f64::from(w.max(1)), f64::from(h.max(1)));
        let (ox, oy) = (self.x, self.y);
        self.x = x;
        self.y = y;
        let q2 = trackball(
            (2.0 * ox - w) / w,
            (h - 2.0 * oy) / h,
            (2.0 * x - w) / w,
            (h - 2.0 * y) / h,
        );
        self.q = q2.mult(self.q).normalize();
    }

    fn track(&mut self, x: i32, y: i32, w: i32, h: i32) {
        /* This keeps it going for about 3 sec */
        let dampen = 0.01;
        self.dx = f64::from(x) - self.x;
        self.dy = f64::from(y) - self.y;
        self.ddx = self.dx * dampen;
        self.ddy = self.dy * dampen;
        self.ow = w;
        self.oh = h;
        self.track_1(f64::from(x), f64::from(y), w, h);
    }

    /// Keep moving in the same direction as the last drag, slowing to a stop.
    fn inertia(&mut self) {
        if self.button_down || (self.ddx == 0.0 && self.ddy == 0.0) {
            return;
        }
        let (x, y) = (self.x + self.dx, self.y + self.dy);
        let (w, h) = (self.ow, self.oh);
        self.track_1(x, y, w, h);
        dampen(&mut self.dx, &mut self.ddx);
        dampen(&mut self.dy, &mut self.ddy);
    }

    /// `gltrackball_rotate`: the matrix to multiply in, which is the analogue
    /// of a `glRotatef` for wherever the user has dragged it to.
    pub fn matrix(&mut self) -> crate::runtime::gl::Mat4 {
        self.inertia();
        self.q.to_matrix()
    }

    /// `gltrackball_get_quaternion`: where the user has dragged it to, as a
    /// quaternion rather than a matrix.
    ///
    /// The savers that turn a thing in four dimensions want this: `polytopes`
    /// builds a 4x4 rotation out of *two* quaternions, which is not something
    /// a 3x3 matrix can be taken apart into again.
    pub fn quaternion(&mut self) -> Quat {
        self.inertia();
        self.q
    }

    /// `gltrackball_event_handler`. True if the event was one of ours.
    pub fn event(&mut self, event: &super::XEvent, w: i32, h: i32) -> bool {
        match event {
            super::XEvent::ButtonPress { x, y, button: 1 } => {
                self.start(*x, *y);
                true
            }
            super::XEvent::ButtonRelease { button: 1, .. } => {
                self.stop();
                true
            }
            super::XEvent::MotionNotify { x, y } if self.button_down => {
                self.track(*x, *y, w, h);
                true
            }
            _ => false,
        }
    }

    pub fn button_down(&self) -> bool {
        self.button_down
    }
}

/// Wind a value down towards zero by its own step, and stop dead when it would
/// overshoot rather than reversing.
fn dampen(n: &mut f64, dn: &mut f64) {
    let pos = *n > 0.0;
    *n -= *dn;
    if pos != (*n > 0.0) {
        *n = 0.0;
        *dn = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{XEvent, ya_rand_init};

    /// Whatever it does, it has to stay in range: these feed a rotation
    /// straight into an angle.
    #[test]
    fn a_rotation_stays_between_zero_and_one() {
        ya_rand_init(20260811);
        let mut r = Rotator::new(1.0, 1.0, 1.0, 1.0, 0.0, true);
        for _ in 0..20_000 {
            let (x, y, z) = r.rotation(true);
            for v in [x, y, z] {
                assert!((0.0..1.0).contains(&v), "{v}");
            }
        }
    }

    /// And it has to actually turn, or the object sits there.
    #[test]
    fn a_rotation_moves() {
        ya_rand_init(20260811);
        let mut r = Rotator::new(1.0, 1.0, 1.0, 1.0, 0.0, false);
        let first = r.rotation(false);
        let mut moved = false;
        for _ in 0..500 {
            if r.rotation(true) != first {
                moved = true;
                break;
            }
        }
        assert!(moved, "the rotator never turned");
    }

    /// An axis given no speed must not drift.
    #[test]
    fn an_axis_with_no_speed_stays_put() {
        ya_rand_init(20260811);
        let mut r = Rotator::new(0.0, 1.0, 0.0, 1.0, 0.0, false);
        for _ in 0..1000 {
            let (x, _, z) = r.rotation(true);
            assert_eq!((x, z), (0.0, 0.0));
        }
    }

    #[test]
    fn wandering_is_off_unless_asked_for() {
        ya_rand_init(20260811);
        let mut r = Rotator::new(1.0, 1.0, 1.0, 1.0, 0.0, true);
        assert_eq!(r.position(true), (0.5, 0.5, 0.5));
        let mut w = Rotator::new(1.0, 1.0, 1.0, 1.0, 1.0, true);
        let a = w.position(true);
        let b = w.position(true);
        assert_ne!(a, b);
    }

    /// An untouched trackball must be the identity, or every saver starts
    /// slightly crooked.
    #[test]
    fn an_untouched_trackball_does_nothing() {
        let mut t = Trackball::new();
        assert_eq!(t.matrix(), crate::runtime::gl::Mat4::IDENTITY);
    }

    /// A drag across the middle of the window turns the object about the
    /// vertical axis, and dragging back undoes it.
    #[test]
    fn a_drag_turns_it_and_dragging_back_returns_it() {
        let (w, h) = (400, 300);
        let mut t = Trackball::new();
        t.event(
            &XEvent::ButtonPress {
                x: 200,
                y: 150,
                button: 1,
            },
            w,
            h,
        );
        t.event(&XEvent::MotionNotify { x: 300, y: 150 }, w, h);
        let turned = t.matrix();
        assert_ne!(turned, crate::runtime::gl::Mat4::IDENTITY);
        // The x axis should have swung towards the viewer, not up or down.
        let p = turned.transform([1.0, 0.0, 0.0]);
        assert!(p[1].abs() < 1e-5, "drifted vertically: {p:?}");

        t.event(&XEvent::MotionNotify { x: 200, y: 150 }, w, h);
        let back = t.matrix();
        for (a, b) in back.0.iter().zip(crate::runtime::gl::Mat4::IDENTITY.0) {
            assert!((a - b).abs() < 1e-4, "{back:?}");
        }
    }

    /// Letting go keeps it spinning, and it stops on its own rather than
    /// spinning for ever.
    #[test]
    fn inertia_runs_down() {
        let (w, h) = (400, 300);
        let mut t = Trackball::new();
        t.event(
            &XEvent::ButtonPress {
                x: 200,
                y: 150,
                button: 1,
            },
            w,
            h,
        );
        t.event(&XEvent::MotionNotify { x: 260, y: 150 }, w, h);
        t.event(
            &XEvent::ButtonRelease {
                x: 260,
                y: 150,
                button: 1,
            },
            w,
            h,
        );
        let a = t.matrix();
        let b = t.matrix();
        assert_ne!(a, b, "it should have kept going");
        for _ in 0..1000 {
            t.matrix();
        }
        let c = t.matrix();
        assert_eq!(c, t.matrix(), "it should have stopped");
    }
}
