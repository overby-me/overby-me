//! Port of `hacks/nerverot.c`.
//!
//! ```text
//! nerverot, nervous rotation of random thingies, v1.4
//! by Dan Bornstein, danfuzz@milk.com
//! Copyright (c) 2000-2001 Dan Bornstein. All rights reserved.
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! The goal of this screensaver is to be interesting and compelling to
//! watch, yet induce a state of nervous edginess in the viewer.
//! ```
//!
//! A few hundred points sit on the surface of some three-dimensional figure,
//! and the figure turns. That much is ordinary. What is not is that no point is
//! ever drawn where it is: each carries nine offsets of its own, redrawn every
//! frame as eight line segments joining them in a ring, and every offset is
//! nudged a little at random every frame and reflected back when it would leave
//! its box. So each point is a small scribble that will not hold still, and the
//! figure is made of a few hundred of them.
//!
//! The figures are nine kinds, picked at random: a sphere, a cube, a cube's
//! corners smeared out, a cylinder, a tetrahedron's faces, a sheet, a spiral
//! cone, a random walk of a squiggle, and two of any of those set side by side,
//! which is the case that recurses and is three times as likely as any other.
//! Whichever it is, it is scaled to a unit radius, its points are shuffled, and
//! it is turned to a random attitude before it is ever seen.
//!
//! The motion is all interpolation towards targets. Three rotations, a scale
//! and a light position each move one per cent of the way to their target every
//! frame, and every so often one or several targets are thrown somewhere new,
//! so the figure is always easing towards a pose it will not reach before the
//! pose changes. The light is only a point to measure distance from: it picks
//! each blot's colour out of a ramp, so the figure is shaded by where it is
//! rather than by any surface it has.
//!
//! Two paths in the C are not here because a normal build never takes them. The
//! erase buffer is only drawn on macOS, where the window is not cleared between
//! frames; everywhere else the window is cleared and only the current segments
//! are drawn, so the second segment array exists to be swapped and never read.
//! And the double buffer is off by default and has no switch in the config XML.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, make_color_ramp, rgb_to_hsv};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint, random,
};

/// The path a blot's nine offsets are joined in: a ring around the middle,
/// starting and ending at the centre.
const BLOT_SHAPE: [XPoint; 9] = [
    XPoint { x: 0, y: 0 },
    XPoint { x: 1, y: 0 },
    XPoint { x: 1, y: 1 },
    XPoint { x: 0, y: 1 },
    XPoint { x: -1, y: 1 },
    XPoint { x: -1, y: 0 },
    XPoint { x: -1, y: -1 },
    XPoint { x: 0, y: -1 },
    XPoint { x: 1, y: -1 },
];

/// A random float in the range -1 to 1.
fn rand_pm1() -> f64 {
    ((random() >> 8) & 0xffff) as f64 / 65536.0 * 2.0 - 1.0
}

/// A random float in the range 0 to 1.
fn rand_01() -> f64 {
    ((random() >> 8) & 0xffff) as f64 / 65536.0
}

/// One point of the figure, with the nine offsets it is actually drawn at.
#[derive(Clone, Copy, Default)]
struct Blot {
    x: f64,
    y: f64,
    z: f64,
    xoff: [[f64; 3]; 3],
    yoff: [[f64; 3]; 3],
}

impl Blot {
    fn new(x: f64, y: f64, z: f64) -> Self {
        let mut b = Self {
            x,
            y,
            z,
            ..Self::default()
        };
        for i in 0..3 {
            for j in 0..3 {
                b.xoff[i][j] = rand_pm1();
                b.yoff[i][j] = rand_pm1();
            }
        }
        b
    }
}

/// One line of one blot's scribble.
#[derive(Clone, Copy, Default)]
struct Seg {
    color: usize,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
}

struct NerveRot {
    gc: Gc,
    /// The background, then the ramp the light picks colours out of.
    colors: Vec<Pixel>,
    color_count: usize,
    line_width: i32,

    requested_blot_count: i32,
    max_iters: i32,
    nervousness: f64,
    max_nerve_radius: f64,
    event_chance: f64,
    iter_amt: f64,
    min_scale: f64,
    max_scale: f64,
    min_radius: i32,
    max_radius: i32,
    delay: u32,

    base_scale: i32,
    center_x: i32,
    center_y: i32,

    blots: Vec<Blot>,
    segs: Vec<Seg>,

    x_rot: f64,
    y_rot: f64,
    z_rot: f64,
    cur_scale: f64,
    light_x: f64,
    light_y: f64,
    light_z: f64,

    x_rot_target: f64,
    y_rot_target: f64,
    z_rot_target: f64,
    scale_target: f64,
    light_x_target: f64,
    light_y_target: f64,
    light_z_target: f64,

    center_x_off: i32,
    center_y_off: i32,
    iters_till_next: i32,
}

impl NerveRot {
    fn new(d: &mut Dpy) -> Self {
        let (width, height) = (d.width(), d.height());
        let mut line_width = d.res.int("lineWidth");
        if line_width <= 0 {
            line_width = 1;
        }
        if width > 2560 || height > 2560 {
            // Retina displays.
            line_width *= 3;
        }

        let mut st = Self {
            gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
            colors: Vec::new(),
            color_count: d.res.int("colors").max(1) as usize,
            line_width,
            requested_blot_count: d.res.int("count").max(1),
            max_iters: d.res.int("maxIters").max(0),
            nervousness: d.res.float("nervousness").clamp(0.0, 1.0),
            max_nerve_radius: d.res.float("maxNerveRadius").clamp(0.0, 1.0),
            event_chance: d.res.float("eventChance").clamp(0.0, 1.0),
            iter_amt: d.res.float("iterAmt").clamp(0.0, 1.0),
            min_scale: d.res.float("minScale").clamp(0.0, 10.0),
            max_scale: d.res.float("maxScale").clamp(0.0, 10.0),
            min_radius: d.res.int("minRadius").clamp(1, 100),
            max_radius: d.res.int("maxRadius").clamp(1, 100),
            delay: d.res.int("delay").max(0) as u32,

            base_scale: height.min(width),
            center_x: width / 2,
            center_y: height / 2,

            blots: Vec::new(),
            segs: Vec::new(),

            x_rot: 0.0,
            y_rot: 0.0,
            z_rot: 0.0,
            cur_scale: 1.0,
            light_x: 0.0,
            light_y: 0.0,
            light_z: 0.0,
            x_rot_target: 0.0,
            y_rot_target: 0.0,
            z_rot_target: 0.0,
            scale_target: 1.0,
            light_x_target: 0.0,
            light_y_target: 0.0,
            light_z_target: 0.0,
            center_x_off: 0,
            center_y_off: 0,
            iters_till_next: 0,
        };
        if st.max_scale < st.min_scale {
            st.max_scale = st.min_scale;
        }
        if st.max_radius < st.min_radius {
            st.max_radius = st.min_radius;
        }
        st.gc.set_line_width(st.line_width);
        st.setup(d);
        st
    }

    /// The colour ramp: a fully saturated hue at one end, a muted one at the
    /// other, with the background in front.
    fn setup_colormap(&mut self, d: &mut Dpy) {
        let (h1, _, _) = rgb_to_hsv(
            (rand_01() * 0x10000 as f64) as u16,
            (rand_01() * 0x10000 as f64) as u16,
            (rand_01() * 0x10000 as f64) as u16,
        );
        let (h2, _, _) = rgb_to_hsv(
            (rand_01() * 0x10000 as f64) as u16,
            (rand_01() * 0x10000 as f64) as u16,
            (rand_01() * 0x10000 as f64) as u16,
        );
        let ramp = make_color_ramp(h1, 1.0, 1.0, h2, 0.7, 0.7, self.color_count, false);
        self.colors = Vec::with_capacity(self.color_count + 1);
        self.colors.push(d.res.pixel("background"));
        self.colors.extend(ramp.iter().map(|c| c.pixel));
    }

    // ---- the figures ------------------------------------------------------

    /// Scale the blots to have a maximum distance of one from the centre.
    fn scale_blots_to_radius1(&mut self) {
        let mut max = 0.0f64;
        for b in &self.blots {
            let d = b.x * b.x + b.y * b.y + b.z * b.z;
            if d > max {
                max = d;
            }
        }
        if max == 0.0 {
            return;
        }
        let max = max.sqrt();
        for b in &mut self.blots {
            b.x /= max;
            b.y /= max;
            b.z /= max;
        }
    }

    fn randomly_reorder_blots(&mut self) {
        let n_blots = self.blots.len();
        for n in 0..n_blots {
            let m = (rand_01() * (n_blots - n) as f64) as usize + n;
            self.blots.swap(n, m.min(n_blots - 1));
        }
    }

    fn randomly_rotate_blots(&mut self) {
        let x_rot = rand_pm1() * std::f64::consts::PI;
        let y_rot = rand_pm1() * std::f64::consts::PI;
        let z_rot = rand_pm1() * std::f64::consts::PI;
        let (sin_x, cos_x) = (x_rot.sin(), x_rot.cos());
        let (sin_y, cos_y) = (y_rot.sin(), y_rot.cos());
        let (sin_z, cos_z) = (z_rot.sin(), z_rot.cos());

        for b in &mut self.blots {
            let (x, y, z) = rotate(b.x, b.y, b.z, sin_x, cos_x, sin_y, cos_y, sin_z, cos_z);
            b.x = x;
            b.y = y;
            b.z = z;
        }
    }

    fn setup_blots_sphere(&mut self) {
        let count = self.requested_blot_count.max(0) as usize;
        self.blots = Vec::with_capacity(count);
        for _ in 0..count {
            // Reject anything too near the centre, to avoid scaling problems.
            let (x, y, z, radius) = loop {
                let (x, y, z) = (rand_pm1(), rand_pm1(), rand_pm1());
                let radius = (x * x + y * y + z * z).sqrt();
                if (0.2..=1.0).contains(&radius) {
                    break (x, y, z, radius);
                }
            };
            self.blots
                .push(Blot::new(x / radius, y / radius, z / radius));
        }
    }

    fn setup_blots_cube(&mut self) {
        let blots_per_edge = (((self.requested_blot_count - 8) / 12) + 2).max(2);
        let dist = 2.0 / (blots_per_edge - 1) as f64;
        self.blots = Vec::new();

        // The corners.
        for i in [-1.0, 1.0] {
            for j in [-1.0, 1.0] {
                for k in [-1.0, 1.0] {
                    self.blots.push(Blot::new(i, j, k));
                }
            }
        }
        // The edges.
        for i in 1..(blots_per_edge - 1) {
            let v = dist * i as f64 - 1.0;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                self.blots.push(Blot::new(v, a, b));
            }
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                self.blots.push(Blot::new(a, v, b));
            }
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                self.blots.push(Blot::new(a, b, v));
            }
        }

        self.scale_blots_to_radius1();
        self.randomly_reorder_blots();
        self.randomly_rotate_blots();
    }

    fn setup_blots_cylinder(&mut self) {
        let req_root = (self.requested_blot_count as f64).sqrt();
        let blots_per_ring = (((rand_pm1() * req_root).ceil() / 2.0 + req_root) as i32).max(2);
        let blots_per_edge = (self.requested_blot_count / blots_per_ring).max(2);
        let dist = 2.0 / (blots_per_edge - 1) as f64;

        self.blots = Vec::new();
        for i in 0..blots_per_ring {
            let a = 2.0 * std::f64::consts::PI / blots_per_ring as f64 * i as f64;
            let (x, y) = (a.sin(), a.cos());
            for j in 0..blots_per_edge {
                self.blots.push(Blot::new(x, y, j as f64 * dist - 1.0));
            }
        }

        self.scale_blots_to_radius1();
        self.randomly_reorder_blots();
        self.randomly_rotate_blots();
    }

    /// A random walk that stays inside a box, which draws as a tangle.
    fn setup_blots_squiggle(&mut self) {
        let count = self.requested_blot_count.max(0) as usize;
        self.blots = Vec::with_capacity(count);

        let max_coor = (rand_01() * 5.0) as i32 as f64 + 1.0;
        let min_coor = -max_coor;

        let (mut x, mut y, mut z) = (rand_pm1(), rand_pm1(), rand_pm1());
        let (mut xv, mut yv, mut zv) = (rand_pm1(), rand_pm1(), rand_pm1());
        let len = (xv * xv + yv * yv + zv * zv).sqrt();
        xv /= len;
        yv /= len;
        zv /= len;

        for _ in 0..count {
            self.blots.push(Blot::new(x, y, z));
            loop {
                xv += rand_pm1() * 0.1;
                yv += rand_pm1() * 0.1;
                zv += rand_pm1() * 0.1;
                let len = (xv * xv + yv * yv + zv * zv).sqrt();
                xv /= len;
                yv /= len;
                zv /= len;

                let (nx, ny, nz) = (x + xv * 0.1, y + yv * 0.1, z + zv * 0.1);
                if (min_coor..=max_coor).contains(&nx)
                    && (min_coor..=max_coor).contains(&ny)
                    && (min_coor..=max_coor).contains(&nz)
                {
                    x = nx;
                    y = ny;
                    z = nz;
                    break;
                }
            }
        }

        self.scale_blots_to_radius1();
        self.randomly_reorder_blots();
    }

    fn setup_blots_cube_corners(&mut self) {
        let count = self.requested_blot_count.max(0) as usize;
        self.blots = Vec::with_capacity(count);
        for _ in 0..count {
            let x = rand_01().round() * 2.0 - 1.0 + rand_pm1() * 0.3;
            let y = rand_01().round() * 2.0 - 1.0 + rand_pm1() * 0.3;
            let z = rand_01().round() * 2.0 - 1.0 + rand_pm1() * 0.3;
            self.blots.push(Blot::new(x, y, z));
        }
        self.scale_blots_to_radius1();
        self.randomly_rotate_blots();
    }

    fn setup_blots_tetrahedron(&mut self) {
        const COR: [[f64; 3]; 4] = [
            [0.0, 1.0, 0.0],
            [-0.75, -0.5, -0.433013],
            [0.0, -0.5, 0.866025],
            [0.75, -0.5, -0.433013],
        ];
        let per_surface = self.requested_blot_count / 4;
        self.blots = Vec::with_capacity((per_surface * 4).max(0) as usize);

        for _ in 0..per_surface {
            // A random point on a unit right triangle, folded into place.
            let (mut rawx, mut rawy) = (rand_01(), rand_01());
            if rawx + rawy > 1.0 {
                let t = 1.0 - rawx;
                rawx = 1.0 - rawy;
                rawy = t;
            }
            for (c, corner) in COR.iter().enumerate() {
                let (c1, c2) = (&COR[(c + 1) % 4], &COR[(c + 2) % 4]);
                let x = (c1[0] - corner[0]) * rawx + (c2[0] - corner[0]) * rawy + corner[0];
                let y = (c1[1] - corner[1]) * rawx + (c2[1] - corner[1]) * rawy + corner[1];
                let z = (c1[2] - corner[2]) * rawx + (c2[2] - corner[2]) * rawy + corner[2];
                self.blots.push(Blot::new(x, y, z));
            }
        }
        self.randomly_rotate_blots();
    }

    fn setup_blots_sheet(&mut self) {
        let per_dim = ((self.requested_blot_count as f64).sqrt().floor() as i32).max(2);
        let space = 2.0 / (per_dim - 1) as f64;
        self.blots = vec![Blot::default(); (per_dim * per_dim) as usize];

        for x in 0..per_dim {
            for y in 0..per_dim {
                let x1 = x as f64 * space - 1.0 + rand_pm1() * space / 3.0;
                let y1 = y as f64 * space - 1.0 + rand_pm1() * space / 3.0;
                let z1 = rand_pm1() * space / 2.0;
                self.blots[(x + y * per_dim) as usize] = Blot::new(x1, y1, z1);
            }
        }

        self.scale_blots_to_radius1();
        self.randomly_reorder_blots();
        self.randomly_rotate_blots();
    }

    fn setup_blots_swirly_cone(&mut self) {
        let count = self.requested_blot_count.max(1);
        let rad_space = 1.0 / (count - 1).max(1) as f64;
        let z_space = rad_space * 2.0;
        let rot_amt = rand_pm1() * std::f64::consts::PI / 10.0;

        self.blots = Vec::with_capacity(count as usize);
        let mut rot = 0.0f64;
        for n in 0..count {
            let radius = n as f64 * rad_space;
            self.blots.push(Blot::new(
                rot.cos() * radius,
                rot.sin() * radius,
                n as f64 * z_space - 1.0,
            ));
            rot += rot_amt;
        }

        self.scale_blots_to_radius1();
        self.randomly_reorder_blots();
        self.randomly_rotate_blots();
    }

    /// Two of any of the others, set side by side. This is the case that
    /// recurses, and it is three times as likely as any other.
    fn setup_blots_duo(&mut self) {
        let orig = self.requested_blot_count;
        if orig < 15 {
            // Special case bottom-out.
            self.setup_blots_sphere();
            return;
        }

        let (mut tx, mut ty, mut tz) = (rand_pm1(), rand_pm1(), rand_pm1());
        let radius = (tx * tx + ty * ty + tz * tz).sqrt();
        tx /= radius;
        ty /= radius;
        tz /= radius;

        self.requested_blot_count = orig / 2;
        self.setup_blots();
        if self.blots.len() as i32 >= orig {
            // That satisfied the original request on its own.
            self.requested_blot_count = orig;
            return;
        }

        let mut blots1 = std::mem::take(&mut self.blots);
        for b in &mut blots1 {
            b.x += tx;
            b.y += ty;
            b.z += tz;
        }

        self.requested_blot_count = orig - blots1.len() as i32;
        self.setup_blots();
        let mut blots2 = std::mem::take(&mut self.blots);
        for b in &mut blots2 {
            b.x -= tx;
            b.y -= ty;
            b.z -= tz;
        }

        blots1.append(&mut blots2);
        self.blots = blots1;
        self.scale_blots_to_radius1();
        self.randomly_reorder_blots();
        self.requested_blot_count = orig;
    }

    fn setup_blots(&mut self) {
        match (rand_01() * 11.0) as i32 {
            0 => self.setup_blots_cube(),
            1 => self.setup_blots_sphere(),
            2 => self.setup_blots_cylinder(),
            3 => self.setup_blots_squiggle(),
            4 => self.setup_blots_cube_corners(),
            5 => self.setup_blots_tetrahedron(),
            6 => self.setup_blots_sheet(),
            7 => self.setup_blots_swirly_cone(),
            _ => self.setup_blots_duo(),
        }
    }

    /// Eight segments per blot, one per pair of adjacent offsets.
    fn setup_segs(&mut self) {
        self.segs = vec![Seg::default(); self.blots.len() * (BLOT_SHAPE.len() - 1)];
    }

    fn setup(&mut self, d: &mut Dpy) {
        self.center_x = d.width() / 2;
        self.center_y = d.height() / 2;
        self.base_scale = d.height().min(d.width());

        self.setup_colormap(d);
        self.setup_blots();
        self.setup_segs();

        // Start somewhere random, with the targets where it already is.
        self.x_rot = rand_01() * std::f64::consts::PI;
        self.x_rot_target = self.x_rot;
        self.y_rot = rand_01() * std::f64::consts::PI;
        self.y_rot_target = self.y_rot;
        self.z_rot = rand_01() * std::f64::consts::PI;
        self.z_rot_target = self.z_rot;
        self.cur_scale = rand_01() * (self.max_scale - self.min_scale) + self.min_scale;
        self.scale_target = self.cur_scale;
        self.light_x = rand_pm1();
        self.light_x_target = self.light_x;
        self.light_y = rand_pm1();
        self.light_y_target = self.light_y;
        self.light_z = rand_pm1();
        self.light_z_target = self.light_z;

        self.iters_till_next = (rand_01() * self.max_iters as f64) as i32;
    }

    // ---- the simulation ---------------------------------------------------

    /// Turn the blots into line segments at the current attitude.
    fn render_segs(&mut self) {
        let (sin_x, cos_x) = (self.x_rot.sin(), self.x_rot.cos());
        let (sin_y, cos_y) = (self.y_rot.sin(), self.y_rot.cos());
        let (sin_z, cos_z) = (self.z_rot.sin(), self.z_rot.cos());
        let mut m = 0;

        for n in 0..self.blots.len() {
            let b = self.blots[n];
            let (x2, y2, z2) = rotate(b.x, b.y, b.z, sin_x, cos_x, sin_y, cos_y, sin_z, cos_z);

            // The colour is the distance from the light, once the blot has
            // been turned: the figure is lit by where it is, not by any
            // surface it has.
            let (x1, y1, z1) = (x2 - self.light_x, y2 - self.light_y, z2 - self.light_z);
            let color =
                (1.0 + (x1 * x1 + y1 * y1 + z1 * z1) / 4.0 * self.color_count as f64) as usize;
            let color = color.min(self.color_count);

            let base_x = (x2 / 2.0 * self.base_scale as f64 * self.cur_scale) as i32
                + self.center_x
                + self.center_x_off;
            let base_y = (y2 / 2.0 * self.base_scale as f64 * self.cur_scale) as i32
                + self.center_y
                + self.center_y_off;
            let radius = (z2 + 1.0) / 2.0 * (self.max_radius - self.min_radius) as f64
                + self.min_radius as f64;

            let mut x = [[0i32; 3]; 3];
            let mut y = [[0i32; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    x[i][j] = base_x
                        + (((i as f64 - 1.0) + b.xoff[i][j] * self.max_nerve_radius) * radius)
                            as i32;
                    y[i][j] = base_y
                        + (((j as f64 - 1.0) + b.yoff[i][j] * self.max_nerve_radius) * radius)
                            as i32;
                }
            }

            for i in 1..BLOT_SHAPE.len() {
                let (a, b) = (BLOT_SHAPE[i - 1], BLOT_SHAPE[i]);
                self.segs[m] = Seg {
                    color,
                    x1: x[(a.x + 1) as usize][(a.y + 1) as usize],
                    y1: y[(a.x + 1) as usize][(a.y + 1) as usize],
                    x2: x[(b.x + 1) as usize][(b.y + 1) as usize],
                    y2: y[(b.x + 1) as usize][(b.y + 1) as usize],
                };
                m += 1;
            }
        }
    }

    /// Jitter every offset, ease everything towards its target, and now and
    /// then throw a target somewhere new.
    fn update_with_feeling(&mut self) {
        self.iters_till_next -= 1;
        if self.iters_till_next < 0 {
            self.iters_till_next = (rand_01() * self.max_iters as f64) as i32;
            self.setup_blots();
            self.setup_segs();
        }

        self.x_rot += (self.x_rot_target - self.x_rot) * self.iter_amt;
        self.y_rot += (self.y_rot_target - self.y_rot) * self.iter_amt;
        self.z_rot += (self.z_rot_target - self.z_rot) * self.iter_amt;
        self.cur_scale += (self.scale_target - self.cur_scale) * self.iter_amt;
        self.light_x += (self.light_x_target - self.light_x) * self.iter_amt;
        self.light_y += (self.light_y_target - self.light_y) * self.iter_amt;
        self.light_z += (self.light_z_target - self.light_z) * self.iter_amt;

        let nervousness = self.nervousness;
        for b in &mut self.blots {
            for i in 0..3 {
                for j in 0..3 {
                    b.xoff[i][j] = jitter(b.xoff[i][j], nervousness);
                    b.yoff[i][j] = jitter(b.yoff[i][j], nervousness);
                }
            }
        }

        if rand_01() > self.event_chance {
            return;
        }
        let pi2 = std::f64::consts::PI * 2.0;
        let max_radius = self.max_radius as f64;
        match (rand_01() * 14.0) as i32 {
            0 => self.x_rot_target = rand_pm1() * pi2,
            1 => self.y_rot_target = rand_pm1() * pi2,
            2 => self.z_rot_target = rand_pm1() * pi2,
            3 => {
                self.x_rot_target = rand_pm1() * pi2;
                self.y_rot_target = rand_pm1() * pi2;
            }
            4 => {
                self.x_rot_target = rand_pm1() * pi2;
                self.z_rot_target = rand_pm1() * pi2;
            }
            5 => {
                self.y_rot_target = rand_pm1() * pi2;
                self.z_rot_target = rand_pm1() * pi2;
            }
            6 => {
                self.x_rot_target = rand_pm1() * pi2;
                self.y_rot_target = rand_pm1() * pi2;
                self.z_rot_target = rand_pm1() * pi2;
            }
            7 => self.center_x_off = (rand_pm1() * max_radius) as i32,
            8 => self.center_y_off = (rand_pm1() * max_radius) as i32,
            9 => {
                self.center_x_off = (rand_pm1() * max_radius) as i32;
                self.center_y_off = (rand_pm1() * max_radius) as i32;
            }
            10 => {
                self.scale_target = rand_01() * (self.max_scale - self.min_scale) + self.min_scale
            }
            11 => self.cur_scale = rand_01() * (self.max_scale - self.min_scale) + self.min_scale,
            12 => {
                self.light_x = rand_pm1();
                self.light_y = rand_pm1();
                self.light_z = rand_pm1();
            }
            _ => {
                self.light_x_target = rand_pm1();
                self.light_y_target = rand_pm1();
                self.light_z_target = rand_pm1();
            }
        }
    }
}

/// The three rotations, in the order the C applies them: z, then x, then y.
#[allow(clippy::too_many_arguments)]
fn rotate(
    x1: f64,
    y1: f64,
    z1: f64,
    sin_x: f64,
    cos_x: f64,
    sin_y: f64,
    cos_y: f64,
    sin_z: f64,
    cos_z: f64,
) -> (f64, f64, f64) {
    let x2 = x1 * cos_z - y1 * sin_z;
    let y2 = x1 * sin_z + y1 * cos_z;
    let z2 = z1;

    let y1 = y2 * cos_x - z2 * sin_x;
    let z1 = y2 * sin_x + z2 * cos_x;
    let x1 = x2;

    let z2 = z1 * cos_y - x1 * sin_y;
    let x2 = z1 * sin_y + x1 * cos_y;
    (x2, y1, z2)
}

/// Nudge an offset, reflecting it back off the walls of its box.
fn jitter(off: f64, nervousness: f64) -> f64 {
    let v = off + rand_pm1() * nervousness;
    if v < -1.0 {
        -(v + 1.0) - 1.0
    } else if v > 1.0 {
        -(v - 1.0) + 1.0
    } else {
        v
    }
}

impl Screenhack for NerveRot {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.update_with_feeling();
        self.render_segs();

        d.clear_window();
        for n in 0..self.segs.len() {
            let s = self.segs[n];
            self.gc
                .set_foreground(self.colors[s.color.min(self.colors.len() - 1)]);
            d.win().draw_line(&self.gc, s.x1, s.y1, s.x2, s.y2);
        }
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        // Upstream ignores this, which on a resize leaves the figure drawn
        // around the old centre. Re-centring costs nothing.
        self.center_x = width / 2;
        self.center_y = height / 2;
        self.base_scale = height.min(width);
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(NerveRot::new(d))
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*count: 250",
    "*colors: 4",
    "*delay: 10000",
    "*maxIters: 1200",
    "*doubleBuffer: false",
    "*eventChance: 0.2",
    "*iterAmt: 0.01",
    "*lineWidth: 0",
    "*minScale: 0.6",
    "*maxScale: 1.75",
    "*minRadius: 3",
    "*maxRadius: 25",
    "*maxNerveRadius: 0.7",
    "*nervousness: 0.3",
    "*ignoreRotation: True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("maxIters", "Duration", 100.0, 8000.0, 100.0, 0, "1200"),
    Opt::slider("count", "Blot count", 1.0, 1000.0, 1.0, 0, "250"),
    Opt::slider("colors", "Colors", 1.0, 255.0, 1.0, 0, "4"),
    Opt::slider("eventChance", "Changes", 0.0, 1.0, 0.05, 2, "0.2"),
    Opt::slider("nervousness", "Nervousness", 0.0, 1.0, 0.05, 2, "0.3"),
    Opt::slider("maxNerveRadius", "Crunchiness", 0.0, 1.0, 0.05, 2, "0.7"),
    Opt::spin("lineWidth", "Line thickness", 0.0, 100.0, "0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "nerverot",
    label: "Nerve Rot",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Dan Bornstein",
        year: "2000",
        video: Some("https://www.youtube.com/watch?v=oUfgDnyGqHM"),
        blurb: "Nervously vibrating squiggles.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
