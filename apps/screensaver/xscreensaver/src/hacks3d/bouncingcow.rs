//! Port of `hacks/glx/bouncingcow.c`.
//!
//! ```text
//! bouncingcow, Copyright (c) 2003-2019 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Boing, boing, boing.  Cow, cow, cow.
//! ```
//!
//! Cows bouncing on an invisible floor. Each one is launched at the same
//! speed under its own gravity, which upstream says is deliberate and is
//! empirically funnier than varying the launch instead. One cow in twelve
//! tumbles as it goes.
//!
//! In mathematical mode a cow occasionally inflates into the sphere of the
//! same volume, which is the joke about the physicist who begins "assume a
//! spherical cow". That is done by moving every vertex out along its own
//! radius and turning its normal towards radial as it goes, so the shading
//! stays right all the way through.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gllist::GlList;
use crate::runtime::rotator::Rotator;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

const FACE: usize = 0;
const HIDE: usize = 1;
const HOOFS: usize = 2;
const HORNS: usize = 3;
const TAIL: usize = 4;
const UDDER: usize = 5;

const MODELS: [&str; 6] = [
    crate::models::COW_FACE,
    crate::models::COW_HIDE,
    crate::models::COW_HOOFS,
    crate::models::COW_HORNS,
    crate::models::COW_TAIL,
    crate::models::COW_UDDER,
];

/// How far below the middle a cow may fall before it is launched again.
const BOTTOM: f32 = 28.0;

fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

fn randsign() -> f32 {
    if random() & 1 == 1 { 1.0 } else { -1.0 }
}

/// One cow, in the air.
struct Floater {
    x: f32,
    y: f32,
    z: f32,
    /// Where it was launched from, which is where it is launched from again.
    launch_x: f32,
    launch_z: f32,
    dx: f32,
    dy: f32,
    dz: f32,
    ddx: f32,
    ddy: f32,
    ddz: f32,
    /// Whether this one tumbles rather than just turning.
    spinner: bool,
    rot: Rotator,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Bounce,
    Inflate,
    Deflate,
}

struct Cow {
    trackball: Trackball,
    /// The six parts, each already turned into geometry at the current
    /// sphericalness.
    parts: Vec<GlList>,
    lists: Vec<u32>,
    floaters: Vec<Floater>,
    mode: Mode,
    /// Nought for a cow-shaped cow, one for a spherical one.
    ratio: f32,
    aspect: f32,
    scale: f32,
    speed: f32,
    mathematical: bool,
    wire: bool,
}

/// The colour and finish of each part.
fn finish(i: usize) -> ([f32; 4], [f32; 4], f32) {
    let black = [0.0, 0.0, 0.0, 1.0];
    let spec = [1.0, 1.0, 1.0, 1.0];
    match i {
        HIDE | TAIL => ([0.63, 0.43, 0.36, 1.0], black, 128.0),
        UDDER => ([1.0, 0.53, 0.53, 1.0], black, 128.0),
        HOOFS | HORNS => ([0.20, 0.20, 0.20, 1.0], spec, 20.0),
        FACE => ([0.10, 0.10, 0.10, 1.0], spec, 20.0),
        _ => ([1.0, 1.0, 1.0, 1.0], spec, 20.0),
    }
}

impl Cow {
    /// `reset_floater`: launch one from where it started.
    fn reset_floater(&self, f: &mut Floater, count: usize) {
        f.y = -BOTTOM;
        f.x = f.launch_x;
        f.z = f.launch_z;
        // Upstream varies the force of gravity rather than the launch
        // velocity, and says so: empirical studies indicate that it is way,
        // way funnier that way.
        f.dy = 5.0;
        f.dx = 0.0;
        f.dz = 0.0;
        f.ddy = self.speed * (-0.6 + bellrand(0.45));
        f.ddx = 0.0;
        f.ddz = 0.0;
        f.spinner = random().is_multiple_of((12 * count as u32).max(1));
        if random().is_multiple_of((30 * count as u32).max(1)) {
            f.dx = bellrand(1.8) * randsign();
            f.dz = bellrand(1.8) * randsign();
        }
    }

    fn tick_floater(&self, f: &mut Floater, count: usize) {
        f.dx += f.ddx;
        f.dy += f.ddy;
        f.dz += f.ddz;
        f.x += f.dx * self.speed;
        f.y += f.dy * self.speed;
        f.z += f.dz * self.speed;
        if f.y < -BOTTOM
            || f.x < -BOTTOM * 8.0
            || f.x > BOTTOM * 8.0
            || f.z < -BOTTOM * 8.0
            || f.z > BOTTOM * 8.0
        {
            self.reset_floater(f, count);
        }
    }

    /// `render_cow`: rebuild the six display lists at the given
    /// sphericalness.
    ///
    /// At nought this is the cow as modelled. At one every vertex has moved
    /// out to the sphere that encloses it and every normal has turned radial,
    /// so the thing is lit as a sphere rather than as a cow wearing one.
    fn render(&mut self, g: &mut Gl) {
        for (i, model) in self.parts.iter().enumerate() {
            if self.lists.len() <= i {
                self.lists.push(g.glx.gen_lists(1));
            }
            let list = self.lists[i];
            g.glx.new_list(list);
            if self.ratio <= 0.0 {
                model.render(&mut g.glx, self.wire);
            } else {
                // The radius of the sphere the cow is inflated into.
                let scale = 10.46;
                let scale2 = 0.5 + 0.5 * (1.0 - self.ratio);
                let stride = model.format.stride();
                g.glx.begin(model.primitive);
                for v in model.data.chunks_exact(stride) {
                    let (n, p) = (&v[0..3], &v[3..6]);
                    let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                    let normal: [f32; 3] = std::array::from_fn(|k| {
                        let (min, max) = (n[k], p[k] / d);
                        min + self.ratio * (max - min)
                    });
                    let pos: [f32; 3] = std::array::from_fn(|k| {
                        let (min, max) = (p[k], p[k] / d * scale);
                        (min + self.ratio * (max - min)) * scale2
                    });
                    g.glx.normal3f(normal[0], normal[1], normal[2]);
                    g.glx.vertex3f(pos[0], pos[1], pos[2]);
                }
                g.glx.end();
            }
            g.glx.end_list();
        }
    }

    fn draw_floater(
        &self,
        g: &mut Gl,
        f: &mut Floater,
        turning: bool,
        view: crate::runtime::gl::Mat4,
        count: usize,
    ) {
        let (x, y, z) = f.rot.position(turning);
        g.glx.push_matrix();
        g.glx.translate(f.x, f.y, f.z);
        g.glx.mult_matrix(view);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        if f.spinner {
            g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        }
        // The more cows there are, the smaller each one is.
        let n = 1.5
            * match count {
                100.. => 0.05,
                26..=99 => 0.18,
                10..=25 => 0.3,
                2..=9 => 0.7,
                _ => 1.0,
            };
        g.glx.scale(n, n, n);
        for i in 0..self.lists.len() {
            // A display list here replays geometry and not state, so each
            // part's colour goes on where it is called.
            let (color, spec, shiny) = finish(i);
            g.glx.material_ambient_diffuse(color);
            g.glx.material_specular(spec);
            g.glx.material_shininess(shiny);
            g.glx.call_list(self.lists[i]);
        }
        g.glx.pop_matrix();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let speed = g.res.float("speed") as f32;
    let count = g.res.int("count").max(1) as usize;
    let mut this = Cow {
        trackball: Trackball::new(),
        parts: MODELS.iter().map(|s| GlList::parse(s)).collect(),
        lists: Vec::new(),
        floaters: Vec::new(),
        mode: Mode::Bounce,
        ratio: 0.0,
        aspect: 1.0,
        scale: 1.0,
        speed,
        mathematical: g.res.bool("mathematical"),
        wire: g.res.bool("wireframe"),
    };
    this.render(g);

    for i in 0..count {
        let mut f = Floater {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            launch_x: 0.0,
            launch_z: 0.0,
            dx: 0.0,
            dy: 0.0,
            dz: 0.0,
            ddx: 0.0,
            ddy: 0.0,
            ddz: 0.0,
            spinner: false,
            rot: Rotator::new(10.0, 0.0, 0.0, 4.0, 0.05 * speed as f64, true),
        };
        // Two cows stand either side; more than that stand in a ring around
        // the first.
        if count == 2 {
            f.x = if i == 1 { 6.0 } else { -6.0 };
        } else if i != 0 {
            let th = (i - 1) as f64 * std::f64::consts::PI * 2.0 / (count - 1) as f64;
            let r = 10.0;
            f.x = (r * th.cos()) as f32;
            f.z = (r * th.sin()) as f32;
        }
        f.launch_x = f.x;
        f.launch_z = f.z;
        this.reset_floater(&mut f, count);
        // Stagger them, or they all bounce together.
        f.y = -BOTTOM + frand(BOTTOM as f64 * 2.0) as f32;
        this.floaters.push(f);
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Cow {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 9 / 16;
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
        g.glx.perspective(30.0, self.aspect, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.lighting(!self.wire);
        g.glx.color_material(self.wire);
        if !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.4, 0.2, 0.4, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        }

        g.glx.push_matrix();
        g.glx.scale(self.scale, self.scale, self.scale);
        g.glx.scale(0.5, 0.5, 0.5);

        if self.mathematical {
            // Assume a spherical cow.
            match self.mode {
                Mode::Bounce => {
                    if self.ratio == 0.0 && random().is_multiple_of(400) {
                        self.mode = Mode::Inflate;
                    } else if self.ratio > 0.0 && random().is_multiple_of(2000) {
                        self.mode = Mode::Deflate;
                    }
                }
                Mode::Inflate => {
                    self.ratio += 0.01;
                    if self.ratio >= 1.0 {
                        self.ratio = 1.0;
                        self.mode = Mode::Bounce;
                    }
                }
                Mode::Deflate => {
                    self.ratio -= 0.01;
                    if self.ratio <= 0.0 {
                        self.ratio = 0.0;
                        self.mode = Mode::Bounce;
                    }
                }
            }
            if self.ratio > 0.0 {
                self.render(g);
            }
        }

        let count = self.floaters.len();
        let mut floaters = std::mem::take(&mut self.floaters);
        let turning = !self.trackball.button_down();
        let view = self.trackball.matrix();
        for f in &mut floaters {
            self.draw_floater(g, f, turning, view, count);
            if turning {
                self.tick_floater(f, count);
            }
        }
        self.floaters = floaters;

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*count:        1",
    "*showFPS:      False",
    "*wireframe:    False",
    "*speed:        1.0",
    "*mathematical: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("count", "Cows", 1.0, 100.0, 1.0, 0, "1"),
    Opt::slider("speed", "Speed", 0.1, 5.0, 0.1, 1, "1.0"),
    Opt::boolean("mathematical", "Assume a spherical cow", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "bouncingcow",
    label: "Bouncing Cow",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=O_b5UWhv49w"),
        blurb: "Boing, boing, boing. Cow, cow, cow.",
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

    /// Six parts of a cow, all of them triangles and none of them empty.
    #[test]
    fn a_cow_is_six_parts() {
        for (i, src) in MODELS.iter().enumerate() {
            let m = GlList::parse(src);
            assert!(m.points > 0, "part {i} is empty");
            assert_eq!(
                m.primitive,
                crate::runtime::gl::Shape::Triangles,
                "part {i} is not triangles"
            );
        }
        // The hide is the bulk of it.
        assert!(GlList::parse(MODELS[HIDE]).points > GlList::parse(MODELS[FACE]).points * 10);
    }

    /// It comes down: gravity is applied every frame and a cow that has
    /// fallen far enough is launched again from where it started.
    #[test]
    fn what_goes_up_comes_down() {
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        let height = |r: &Runner3d| r.frame().batches[0].modelview.0[13];
        r.step();
        let mut ys = vec![height(&r)];
        for _ in 0..200 {
            r.step();
            ys.push(height(&r));
        }
        // Somewhere in there it rose and then fell.
        let top = ys.iter().copied().fold(f32::MIN, f32::max);
        let bottom = ys.iter().copied().fold(f32::MAX, f32::min);
        assert!(top - bottom > 1.0, "it never moved: {bottom}..{top}");
        assert!(
            ys.windows(2).any(|w| w[1] > w[0]) && ys.windows(2).any(|w| w[1] < w[0]),
            "it only went one way"
        );
    }

    /// A spherical cow really is spherical: every vertex ends up the same
    /// distance from the middle.
    #[test]
    fn assume_a_spherical_cow() {
        let hide = GlList::parse(MODELS[HIDE]);
        let stride = hide.format.stride();
        let radii: Vec<f32> = hide
            .data
            .chunks_exact(stride)
            .map(|v| {
                let p = &v[3..6];
                (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
            })
            .collect();
        let lo = radii.iter().copied().fold(f32::MAX, f32::min);
        let hi = radii.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 1.0, "the cow is already a sphere");

        // At a ratio of one every vertex has moved out to the same radius,
        // times the shrink that keeps it the same size on screen.
        let ratio = 1.0f32;
        let scale = 10.46;
        let scale2 = 0.5 + 0.5 * (1.0 - ratio);
        let out: Vec<f32> = hide
            .data
            .chunks_exact(stride)
            .map(|v| {
                let p = &v[3..6];
                let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                let q: [f32; 3] = std::array::from_fn(|k| {
                    let (min, max) = (p[k], p[k] / d * scale);
                    (min + ratio * (max - min)) * scale2
                });
                (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt()
            })
            .collect();
        let lo = out.iter().copied().fold(f32::MAX, f32::min);
        let hi = out.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo < 0.01, "the sphere is {lo}..{hi} across");
    }

    /// Six parts a cow, but the hoofs and the horns are the same dark grey
    /// and are drawn one after the other, so they arrive as one batch.
    #[test]
    fn every_cow_is_five_draws() {
        let batches = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
                .count()
        };
        assert_eq!(batches("count=1"), 5);
        assert_eq!(batches("count=4"), 20);
    }
}
