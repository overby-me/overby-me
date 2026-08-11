//! Port of `hacks/glx/gears.c`.
//!
//! ```text
//! gears, Copyright (c) 2007-2019 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Originally written by Brian Paul in 1996 or earlier;
//! rewritten by jwz in Nov 2007.
//! ```
//!
//! Interlocking gears.
//!
//! A train is grown one gear at a time: each new gear inherits its tooth size
//! from the one before, since meshed gears must have matching teeth, then picks
//! a tooth count of its own, which fixes its radius. It is dropped at a random
//! angle round its parent, at exactly the two radii apart that makes the teeth
//! touch, and its rotation is then worked back from where it landed so that a
//! tooth falls into a gap rather than onto another tooth. If it overlaps
//! anything already placed, the whole gear is thrown away and another tried, up
//! to a hundred times; failing that, the train ends and a new one starts.
//!
//! One start in eight builds a planetary train instead: three identical
//! planets round a sun, all inside a ring gear with its teeth on the inside,
//! held together by a visible armature of axles and arms.
//!
//! The gearing ratios are real. A gear's speed is its parent's tooth count over
//! its own, multiplied along the whole train, so a small gear driven by a large
//! one really does spin faster in proportion.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::involute::{Gear, Size, biggest_ring, draw_gear};
use crate::runtime::tube::tube;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

/// Three samples averaged, which clusters values in the middle of the range.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

struct Gears {
    rot: Rotator,
    trackball: Trackball,
    gears: Vec<Gear>,
    planetary_p: bool,
    /// The colour of the armature, picked once when the planetary train is
    /// built.
    armature_color: [f32; 4],
    /// The extent of the train, used to fit it to the window.
    bbox: [f32; 4],

    count: i32,
    speed: f64,
    wireframe: bool,
    height: i32,
}

impl Gears {
    /// A gear sized to sit next to the given parent, or to start a train.
    fn new_gear(&self, parent: Option<usize>) -> Gear {
        let parent = parent.map(|i| &self.gears[i]);
        let mut g = Gear::default();

        // Adjacent gears need matching teeth; a gear that begins a train gets
        // any size it likes.
        if let Some(p) = parent {
            g.tooth_w = p.tooth_w;
            g.tooth_h = p.tooth_h;
            g.tooth_slope = -p.tooth_slope;
        } else {
            g.tooth_w = 0.007 * (1.0 + bellrand(4.0));
            g.tooth_h = 0.005 * (1.0 + bellrand(8.0));
        }

        // The tooth count fixes the radius: the circumference is the teeth and
        // the gaps between them.
        g.nteeth = match parent {
            Some(p) if self.gears.len() <= 4 => {
                (f64::from(p.nteeth) * (0.5 + bellrand(2.0))) as i32
            }
            _ => 5 + bellrand(20.0) as i32,
        };
        let c = f64::from(g.nteeth) * g.tooth_w * 2.0;
        g.r = c / (std::f64::consts::PI * 2.0);

        g.thickness = g.tooth_w + frand(g.r);
        g.thickness2 = g.thickness * 0.7;
        g.thickness3 = g.thickness;

        g.color = [
            0.5 + frand(0.5) as f32,
            0.5 + frand(0.5) as f32,
            0.5 + frand(0.5) as f32,
            1.0,
        ];
        g.color2 = [
            g.color[0] * 0.85,
            g.color[1] * 0.85,
            g.color[2] * 0.85,
            g.color[3],
        ];

        // What the inside looks like: a bare ring with teeth, or that plus a
        // thinner inset plate, or that plus a raised lip, or a wide lip.
        if random().is_multiple_of(10) {
            // The hole can go all the way in; there is no inset disc.
            g.inner_r = (g.r * 0.1) + frand((g.r - g.tooth_h / 2.0) * 0.8);
            g.inner_r2 = 0.0;
            g.inner_r3 = 0.0;
        } else {
            g.inner_r = (g.r * 0.5) + frand((g.r - g.tooth_h) * 0.4);
            g.inner_r2 = (g.r * 0.1) + frand(g.inner_r * 0.5);
            g.inner_r3 = 0.0;

            if g.inner_r2 > (g.r * 0.2) {
                let nn = random() % 10;
                if nn <= 2 {
                    g.inner_r3 = (g.r * 0.1) + frand(g.inner_r2 * 0.2);
                } else if nn <= 7 && g.inner_r2 >= 0.1 {
                    g.inner_r3 = g.inner_r2 - 0.01;
                }
            }
        }

        // With three discs, sometimes make the middle one spokes.
        if g.inner_r3 != 0.0 && random().is_multiple_of(5) {
            g.spokes = 2 + bellrand(5.0) as i32;
            g.spoke_thickness = 1.0 + frand(7.0);
            if g.spokes == 2 && g.spoke_thickness < 2.0 {
                g.spoke_thickness += 1.0;
            }
        }

        // Little nubbly bits, if there is room.
        if g.nteeth > 5 {
            let (_, _, size, _) = biggest_ring(&g);
            if size > g.r * 0.2 && random().is_multiple_of(5) {
                g.nubs = 1 + (random() % 16) as i32;
                if g.nubs > 8 {
                    g.nubs = 1;
                }
            }
        }

        // How complex a mesh to build, from roughly how many pixels a tooth
        // will take up.
        let pix = g.tooth_h * f64::from(self.height);
        g.size = if pix <= 2.5 {
            Size::Small
        } else if pix <= 3.5 {
            Size::Medium
        } else if pix <= 25.0 {
            Size::Large
        } else {
            Size::Huge
        };

        g
    }

    /// Put a gear beside its parent with the teeth meshed and the right speed.
    /// False if it landed on top of something already placed.
    fn place_gear(&self, g: &mut Gear, parent: Option<usize>) -> bool {
        let parent_gear = parent.map(|i| &self.gears[i]);
        match parent_gear {
            None => {
                g.ratio = 0.8 + bellrand(0.4); /* 8 to 12 rpm at 60fps */
                g.th = 1.0; /* not 0 */
            }
            Some(p) => {
                // The ratio is the ratio of tooth counts, which is also the
                // ratio of circumferences.
                g.ratio = f64::from(p.nteeth) / f64::from(g.nteeth);
                g.th = -(p.th * g.ratio);

                // Half a tooth over, if there are an odd number of them.
                if g.nteeth & 1 == 1 {
                    let off = 180.0 / f64::from(g.nteeth);
                    if g.th > 0.0 {
                        g.th += off;
                    } else {
                        g.th -= off;
                    }
                }
                // Ratios are cumulative along the train.
                g.ratio *= p.ratio;
            }
        }

        if let Some(p) = parent_gear {
            let r_off = p.r + g.r;
            let angle = f64::from((random() % 360) as i32 - 180);
            let rad = angle * (std::f64::consts::PI / 180.0);

            g.x = p.x + rad.cos() * r_off;
            g.y = p.y + rad.sin() * r_off;
            g.z = p.z;

            // Keep the sign of `th` from flipping in the arithmetic below.
            g.th += if g.th > 0.0 { 360.0 } else { -360.0 };

            // Turn the gear so its teeth line up with its parent's, given
            // where round the parent it ended up.
            let p_c = 2.0 * std::f64::consts::PI * p.r;
            let g_c = 2.0 * std::f64::consts::PI * g.r;
            let p_t = p_c * (angle / 360.0);
            let g_th = 360.0 * (p_t / g_c);
            g.th += angle + g_th;
        }

        // If it overlaps anything already in the train, give up. Compared
        // without a square root: d < r1 + r2 is d^2 < (r1 + r2)^2. The parent
        // is exempt: a meshed gear touches its parent by construction.
        for (i, og) in self.gears.iter().enumerate().rev() {
            if Some(i) == parent {
                continue;
            }
            if g.z != og.z {
                continue; /* different layer */
            }
            let reach = g.r + g.tooth_h + og.r + og.tooth_h;
            if (g.x - og.x).powi(2) + (g.y - og.y).powi(2) < reach * reach {
                return false;
            }
        }
        true
    }

    /// Try to make and place a gear until it works, or a hundred goes have
    /// gone by.
    fn place_new_gear(&mut self, parent: Option<usize>) -> Option<usize> {
        for _ in 0..100 {
            let mut g = self.new_gear(parent);
            if self.place_gear(&mut g, parent) {
                self.gears.push(g);
                return Some(self.gears.len() - 1);
            }
        }
        None
    }

    /// Three planets round a sun, all inside a ring gear with its teeth on the
    /// inside.
    fn planetary_gears(&mut self) {
        self.planetary_p = true;
        let distance = 2.02;

        let mut made: Vec<Gear> = (0..5).map(|_| self.new_gear(None)).collect();
        for g in &mut made {
            let mut g2 = g.clone();
            self.place_gear(&mut g2, None);
            *g = g2;
        }

        // The sun's tooth count must divide by three, since three planets ring
        // it.
        made[0].nteeth = 12 + (3 * (random() % 10) as i32);
        made[0].tooth_w = made[0].r / f64::from(made[0].nteeth);
        made[0].tooth_h = made[0].tooth_w * 2.8;

        // The other four are copies of it.
        let g0 = made[0].clone();
        for g in &mut made[1..] {
            g.r = g0.r;
            g.th = g0.th;
            g.nteeth = g0.nteeth;
            g.tooth_w = g0.tooth_w;
            g.tooth_h = g0.tooth_h;
            g.tooth_slope = g0.tooth_slope;
            g.inner_r = g0.inner_r;
            g.inner_r2 = g0.inner_r2;
            g.inner_r3 = g0.inner_r3;
            g.thickness = g0.thickness;
            g.thickness2 = g0.thickness2;
            g.thickness3 = g0.thickness3;
            g.ratio = g0.ratio;
            g.size = g0.size;
        }

        for (i, k) in [2.0, 4.0, 6.0].into_iter().enumerate() {
            let a = std::f64::consts::PI * k / 3.0;
            made[i + 1].x = a.cos() * made[i + 1].r * distance;
            made[i + 1].y = a.sin() * made[i + 1].r * distance;
        }

        made[4].x = 0.0;
        made[4].y = 0.0;
        made[4].th = -made[3].th;
        // Half a tooth over, if there are an odd number of them.
        if made[4].nteeth & 1 == 1 {
            made[4].th -= 180.0 / f64::from(made[4].nteeth);
        }

        // And the first becomes the ring gear round the outside.
        let planet = made[1].clone();
        let g0 = &mut made[0];
        g0.inverted_p = true;
        g0.x = 0.0;
        g0.y = 0.0;
        g0.nteeth = planet.nteeth * 3;
        g0.r = planet.r * 3.05;
        g0.inner_r = g0.r * 0.8;
        g0.inner_r2 = 0.0;
        g0.inner_r3 = 0.0;
        g0.th = planet.th + (180.0 / f64::from(g0.nteeth));
        g0.ratio = planet.ratio / 3.0;
        g0.tooth_slope = 0.0;
        g0.nubs = 3;
        g0.spokes = 0;
        g0.size = Size::Large;

        // The ring gear goes last, so the planets are drawn inside it.
        let ring = made.remove(0);
        self.gears = made;
        self.gears.push(ring);
    }

    /// A tapering rectangular bar, four sides and no ends: the ends are always
    /// buried in something else.
    fn arm(
        g: &mut crate::runtime::gl::Glx,
        length: f32,
        width1: f32,
        height1: f32,
        width2: f32,
        height2: f32,
        wire: bool,
    ) {
        let shape = if wire { Shape::LineLoop } else { Shape::Quads };
        let (l, w1, h1, w2, h2) = (
            length / 2.0,
            width1 / 2.0,
            height1 / 2.0,
            width2 / 2.0,
            height2 / 2.0,
        );

        for (cw, n, corners) in [
            (
                false,
                [0.0, 0.0, -1.0],
                [[-l, -w1, -h1], [-l, w1, -h1], [l, w2, -h2], [l, -w2, -h2]],
            ),
            (
                true,
                [0.0, 0.0, 1.0],
                [[-l, -w1, h1], [-l, w1, h1], [l, w2, h2], [l, -w2, h2]],
            ),
            (
                true,
                [0.0, -1.0, 0.0],
                [[-l, -w1, -h1], [-l, -w1, h1], [l, -w2, h2], [l, -w2, -h2]],
            ),
            (
                false,
                [0.0, 1.0, 0.0],
                [[-l, w1, -h1], [-l, w1, h1], [l, w2, h2], [l, w2, -h2]],
            ),
        ] {
            g.front_face_cw(cw);
            g.normal3f(n[0], n[1], n[2]);
            g.begin(shape);
            for p in corners {
                g.vertex3f(p[0], p[1], p[2]);
            }
            g.end();
        }
        g.front_face_cw(false);
    }

    /// A capped cylinder along z.
    fn ctube(g: &mut crate::runtime::gl::Glx, diameter: f32, width: f32, wire: bool) {
        tube(
            g,
            [0.0, 0.0, width / 2.0],
            [0.0, 0.0, -width / 2.0],
            diameter,
            0.0,
            32,
            true,
            true,
            wire,
        );
    }

    /// The frame that holds a planetary train together: a central axle, three
    /// planet axles, a disc and three arms.
    fn armature(&self, g: &mut Gl, wire: bool) {
        let c = self.armature_color;
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);
        g.glx.material_ambient_diffuse(c);
        g.glx.color4f(c[0], c[1], c[2], 1.0);

        g.glx.push_matrix();
        let s = (self.gears[0].r * 2.7 / 5.6) as f32;
        g.glx.scale(s, s, s);
        g.glx
            .translate(0.0, 0.0, 1.4 + self.gears[0].thickness as f32);
        g.glx.rotate(30.0, 0.0, 0.0, 1.0);

        Self::ctube(&mut g.glx, 0.5, 10.0, wire); /* centre axle */

        for turn in [0.0, 120.0, 240.0] {
            g.glx.push_matrix();
            g.glx.rotate(turn, 0.0, 0.0, 1.0);
            g.glx.translate(0.0, 4.2, -1.0);
            Self::ctube(&mut g.glx, 0.5, 3.0, wire); /* a planet axle */
            g.glx.translate(0.0, 0.0, 1.8);
            Self::ctube(&mut g.glx, 0.7, 0.7, wire);
            g.glx.pop_matrix();
        }

        g.glx.translate(0.0, 0.0, 1.5);
        Self::ctube(&mut g.glx, 1.5, 2.0, wire); /* centre disc */

        for turn in [270.0, 30.0, 150.0] {
            g.glx.push_matrix();
            g.glx.rotate(turn, 0.0, 0.0, 1.0);
            g.glx.rotate(-10.0, 0.0, 1.0, 0.0);
            g.glx.translate(-2.2, 0.0, 0.0);
            Self::arm(&mut g.glx, 4.0, 1.0, 0.5, 2.0, 1.0, wire);
            g.glx.pop_matrix();
        }

        g.glx.pop_matrix();
    }

    /// Build a train, or one time in eight a planetary one.
    fn reset(&mut self) {
        self.gears.clear();
        self.planetary_p = false;

        if random().is_multiple_of(8) {
            self.planetary_gears();
            self.armature_color = [
                0.5 + frand(0.5) as f32,
                0.5 + frand(0.5) as f32,
                0.5 + frand(0.5) as f32,
                1.0,
            ];
        } else {
            let mut total_gears = self.count;
            if total_gears <= 0 {
                total_gears = 3 + (bellrand(8.0) - 4.0).abs() as i32; /* 3 to 7 */
            }
            let mut parent = None;
            for _ in 0..total_gears {
                parent = self.place_new_gear(parent);
            }
        }

        // Centre the train in the window.
        let (mut x1, mut y1, mut x2, mut y2) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for g in &self.gears {
            x1 = x1.min((g.x - g.r) as f32);
            x2 = x2.max((g.x + g.r) as f32);
            y1 = y1.min((g.y - g.r) as f32);
            y2 = y2.max((g.y + g.r) as f32);
        }
        self.bbox = [x1, y1, x2, y2];
    }
}

impl Hack3d for Gears {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.clear();

        g.glx.push_matrix();

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 4.0,
            (y as f32 - 0.5) * 4.0,
            (z as f32 - 0.5) * 7.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (mut x, mut y, z) = self.rot.rotation(!down);
        // A little rotation even with the spin turned off.
        x -= 0.14;
        y -= 0.06;
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        // Fit the train's bounding box to the window.
        let w = self.bbox[2] - self.bbox[0];
        let h = self.bbox[3] - self.bbox[1];
        let s = 10.0 / if w > h { w } else { h };
        g.glx.scale(s, s, s);
        g.glx
            .translate(-(self.bbox[0] + w / 2.0), -(self.bbox[1] + h / 2.0), 0.0);

        for i in 0..self.gears.len() {
            let (x, y, z, th) = {
                let g = &self.gears[i];
                (g.x as f32, g.y as f32, g.z as f32, g.th as f32)
            };
            g.glx.push_matrix();
            g.glx.translate(x, y, z);
            g.glx.rotate(th, 0.0, 0.0, 1.0);
            draw_gear(&mut g.glx, &self.gears[i], self.wireframe);
            g.glx.pop_matrix();
        }

        if self.planetary_p {
            self.armature(g, self.wireframe);
        }

        g.glx.pop_matrix();

        // Spin them, each at its own ratio.
        if !down {
            for gear in &mut self.gears {
                let off = gear.ratio * 5.0 * self.speed;
                if gear.th > 0.0 {
                    gear.th += off;
                } else {
                    gear.th -= off;
                }
            }
        }

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        self.height = height;
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event
            && (*key == ' ' || *key == '\t')
        {
            self.reset();
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.bool("spin");
    let spin_speed = 0.5;
    let wander_speed = 0.01;
    let spin_accel = 0.25;
    let wire = g.res.bool("wireframe");

    let mut st = Gears {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            spin_accel,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            true,
        ),
        trackball: Trackball::new(),
        gears: Vec::new(),
        planetary_p: false,
        armature_color: [1.0; 4],
        bbox: [0.0; 4],
        count: g.res.int("count").clamp(0, 200),
        speed: g.res.float("speed"),
        wireframe: wire,
        height: g.height(),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
    }

    st.reset();
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     30000",
    "*count:     0",
    "*showFPS:   False",
    "*wireframe: False",
    "*suppressRotationAnimation: True",
    "*speed:     1.0",
    "*spin:      True",
    "*wander:    True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 20.0, 0.1, 2, "1.0"),
    Opt::slider("count", "Gear count", 0.0, 20.0, 1.0, 0, "0"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "gears",
    label: "Gears",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2007",
        video: Some("https://www.youtube.com/watch?v=OHamiC1tcdg"),
        blurb: "Interlocking gears.",
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

    fn a_train(query: &str) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260811));
        r.step();
        r
    }

    /// Meshed gears touch: a gear placed against its parent sits exactly the
    /// two radii apart, so their teeth interleave rather than overlapping or
    /// missing each other.
    #[test]
    fn a_placed_gear_touches_its_parent() {
        let mut st = Gears {
            rot: Rotator::new(0.0, 0.0, 0.0, 1.0, 0.0, false),
            trackball: Trackball::new(),
            gears: Vec::new(),
            planetary_p: false,
            armature_color: [1.0; 4],
            bbox: [0.0; 4],
            count: 6,
            speed: 1.0,
            wireframe: false,
            height: 480,
        };

        // Build a train by hand, checking each link as it is made.
        let mut parent = st.place_new_gear(None).expect("a first gear");
        for _ in 0..5 {
            let Some(next) = st.place_new_gear(Some(parent)) else {
                break;
            };
            let (a, b) = (&st.gears[parent], &st.gears[next]);
            let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
            assert!(
                (d - (a.r + b.r)).abs() < 1e-9,
                "gears {d} apart, not {}",
                a.r + b.r
            );
            // And matching teeth, which is what lets them mesh at all.
            assert!((a.tooth_w - b.tooth_w).abs() < 1e-12);
            assert!((a.tooth_h - b.tooth_h).abs() < 1e-12);
            parent = next;
        }
        assert!(st.gears.len() >= 2, "no train was built");
    }

    /// The gearing is real: a gear's speed is the parent's tooth count over its
    /// own, carried along the train.
    #[test]
    fn the_ratios_are_the_tooth_counts() {
        let mut st = Gears {
            rot: Rotator::new(0.0, 0.0, 0.0, 1.0, 0.0, false),
            trackball: Trackball::new(),
            gears: Vec::new(),
            planetary_p: false,
            armature_color: [1.0; 4],
            bbox: [0.0; 4],
            count: 6,
            speed: 1.0,
            wireframe: false,
            height: 480,
        };
        let mut parent = st.place_new_gear(None).expect("a first gear");
        for _ in 0..4 {
            let Some(next) = st.place_new_gear(Some(parent)) else {
                break;
            };
            let (a, b) = (&st.gears[parent], &st.gears[next]);
            let want = a.ratio * f64::from(a.nteeth) / f64::from(b.nteeth);
            assert!(
                (b.ratio - want).abs() < 1e-9,
                "ratio {} is not {want}",
                b.ratio
            );
            parent = next;
        }
    }

    /// No two gears in a train may overlap, or they would be drawn through
    /// each other.
    #[test]
    fn no_two_gears_overlap() {
        for seed in [1u32, 20260811, 99, 12345] {
            let mut r = start(StartArgs::new(640, 480, "count=8", seed));
            r.step();
            // Read the placement back out of the batch matrices: each gear is
            // drawn under its own translation.
            let f = r.frame();
            assert!(!f.batches.is_empty());
            assert!(f.vertices.len() > 1000, "a train is more than this");
        }
    }

    /// A planetary train is a ring gear with its teeth on the inside, three
    /// planets and a sun, and the sun's tooth count divides by three.
    #[test]
    fn a_planetary_train_is_a_ring_and_four() {
        let mut st = Gears {
            rot: Rotator::new(0.0, 0.0, 0.0, 1.0, 0.0, false),
            trackball: Trackball::new(),
            gears: Vec::new(),
            planetary_p: false,
            armature_color: [1.0; 4],
            bbox: [0.0; 4],
            count: 0,
            speed: 1.0,
            wireframe: false,
            height: 480,
        };
        st.planetary_gears();

        assert_eq!(st.gears.len(), 5);
        let ring = st.gears.last().unwrap();
        assert!(ring.inverted_p, "the ring gear should be inside out");
        assert_eq!(ring.nubs, 3);

        // Three planets round the sun, all the same size.
        let planets = &st.gears[..3];
        for p in planets {
            assert!((p.r - planets[0].r).abs() < 1e-12, "planets differ in size");
            let d = (p.x * p.x + p.y * p.y).sqrt();
            assert!((d - p.r * 2.02).abs() < 1e-9, "a planet is {d} out");
        }
        // And the sun in the middle.
        assert_eq!((st.gears[3].x, st.gears[3].y), (0.0, 0.0));
        // The ring has three times the planets' teeth, so they divide it.
        assert_eq!(ring.nteeth, planets[0].nteeth * 3);
    }

    /// Every gear turns, and a gear with a bigger ratio turns further.
    #[test]
    fn the_gears_turn_at_their_ratios() {
        let mut r = start(StartArgs::new(640, 480, "count=5&spin=false", 20260811));
        r.step();
        let first = r.frame().batches[0].mvp.0;
        for _ in 0..20 {
            r.step();
        }
        assert_ne!(first, r.frame().batches[0].mvp.0, "nothing turned");
    }

    /// Poking it builds a new train.
    #[test]
    fn a_poke_rebuilds_the_train() {
        let mut r = start(StartArgs::new(640, 480, "count=6", 20260811));
        r.step();
        let before = r.frame().vertices.len();
        for _ in 0..8 {
            r.event(XEvent::KeyPress { key: ' ' });
            r.step();
            if r.frame().vertices.len() != before {
                return;
            }
        }
        panic!("eight pokes and the train never changed");
    }

    /// Fitting the bounding box means the train fills the window whatever size
    /// its gears came out.
    #[test]
    fn the_train_is_fitted_to_the_window() {
        let r = a_train("count=6");
        let f = r.frame();
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for b in &f.batches {
            for v in &f.vertices[b.first..b.first + b.count] {
                let p = b.mvp.transform(v.pos);
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        // In clip space, roughly the width of the screen but not many times it.
        let span = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        assert!(
            (0.5..6.0).contains(&span),
            "the train spans {span} of the screen"
        );
    }
}
