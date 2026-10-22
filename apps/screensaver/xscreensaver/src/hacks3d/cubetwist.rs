//! Port of `hacks/glx/cubetwist.c`.
//!
//! ```text
//! cubetwist, Copyright © 2016-2025 Jamie Zawinski <jwz@jwz.org>
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
//! A series of nested cubes rotate and slide recursively.
//!
//! The cubes are not drawn independently. Each one is drawn inside the matrix
//! its parent left behind, and the parent's rotation and offset are copied down
//! to it before it draws, so every level adds the *same* small turn to the one
//! above. One oscillator driving the outermost cube by ninety degrees therefore
//! twists the whole stack into a helix, and the deeper a cube is the further it
//! has been carried. That is the entire trick, and it is four lines at the tail
//! of the recursion.
//!
//! A cube is not solid either: it is twenty-four struts, each an L of two quads
//! along one edge, so what you see through the frame is the next cube in.
//!
//! The motion comes from a little list of oscillators, each easing one variable
//! from where it is to somewhere else on a sine and then, if it has repeats
//! left, swapping its ends and going back. When the list empties, one frame in
//! sixty starts another. The rate works out the same whatever the speed knob
//! says, because the tick is divided by the speed and each oscillator's rate is
//! multiplied by it; that is upstream's arithmetic, and the knob's real effect
//! is on how far a nudge carries.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Ease, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, ease,
    frand, random,
};

/// Which of the outermost cube's six numbers an oscillator drives.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Var {
    RotX,
    RotY,
    RotZ,
    PosX,
    PosY,
    PosZ,
}

struct Cube {
    size: f32,
    thickness: f32,
    color: [f32; 4],
}

struct Oscillator {
    ratio: f64,
    from: f64,
    to: f64,
    speed: f64,
    var: Var,
    remaining: i32,
}

struct CubeTwist {
    rot: Rotator,
    trackball: Trackball,
    cubes: Vec<Cube>,
    oscillators: Vec<Oscillator>,
    /// The outermost cube's offset and turn, which every level inherits.
    pos: [f64; 3],
    cube_rot: [f64; 3],

    speed: f32,
    thickness: f32,
    displacement: f32,
    wireframe: bool,
}

/// `RANDSIGN`.
fn randsign() -> f64 {
    if random() & 1 != 0 { 1.0 } else { -1.0 }
}

impl CubeTwist {
    /// One edge of a cube: an L of two quads, drawn as fans.
    fn draw_strut(&self, g: &mut Gl, c: &Cube) {
        let wire = self.wireframe;
        let (size, t) = (c.size, c.thickness);

        g.glx.push_matrix();
        g.glx.front_face_cw(true);
        g.glx.normal3f(0.0, 0.0, -1.0);
        g.glx.translate(-size / 2.0, -size / 2.0, -size / 2.0);

        let fan = |g: &mut Gl, vs: [[f32; 3]; 4]| {
            g.glx.begin(if wire {
                Shape::LineLoop
            } else {
                Shape::TriangleFan
            });
            for v in vs {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();
        };

        fan(
            g,
            [
                [0.0, 0.0, 0.0],
                [size, 0.0, 0.0],
                [size - t, t, 0.0],
                [t, t, 0.0],
            ],
        );

        g.glx.normal3f(0.0, 1.0, 0.0);
        fan(
            g,
            [[t, t, 0.0], [size - t, t, 0.0], [size - t, t, t], [t, t, t]],
        );
        g.glx.pop_matrix();
    }

    /// The whole stack, each cube drawn inside the matrix the one outside it
    /// left behind.
    fn draw_cubes(&self, g: &mut Gl) {
        for c in &self.cubes {
            g.glx
                .color4f(c.color[0], c.color[1], c.color[2], c.color[3]);
            g.glx.material_ambient_diffuse(c.color);

            g.glx.push_matrix();
            for j in 0..6 {
                for _ in 0..4 {
                    self.draw_strut(g, c);
                    g.glx.rotate(90.0, 0.0, 0.0, 1.0);
                }
                if j == 3 {
                    g.glx.rotate(90.0, 0.0, 0.0, 1.0);
                }
                if j < 4 {
                    g.glx.rotate(90.0, 0.0, 1.0, 0.0);
                } else {
                    g.glx.rotate(180.0, 1.0, 0.0, 0.0);
                }
            }
            g.glx.pop_matrix();

            // Upstream leaves these on the matrix stack rather than pushing,
            // because it is a tail call: the next cube in is drawn in the
            // frame this one just turned, so the twist compounds down the
            // stack.
            g.glx.rotate(self.cube_rot[0] as f32, 1.0, 0.0, 0.0);
            g.glx.rotate(self.cube_rot[1] as f32, 0.0, 1.0, 0.0);
            g.glx.rotate(self.cube_rot[2] as f32, 0.0, 0.0, 1.0);
            g.glx
                .translate(self.pos[0] as f32, self.pos[1] as f32, self.pos[2] as f32);
        }
    }

    fn make_cubes(&mut self) {
        let step = 2.0 * (self.thickness + self.displacement);
        let mut size = 1.0f32;
        let mut cc = [
            0.3 + frand(0.7) as f32,
            0.3 + frand(0.7) as f32,
            0.3 + frand(0.7) as f32,
            1.0,
        ];

        self.cubes.clear();
        loop {
            self.cubes.push(Cube {
                size,
                thickness: self.thickness,
                color: cc,
            });
            size -= step;
            if size <= step {
                break;
            }
        }

        let cstep = 0.8 / self.cubes.len() as f32;
        for c in &mut self.cubes {
            c.color = cc;
            for v in cc.iter_mut().take(3) {
                *v -= cstep;
            }
        }
    }

    fn var(&mut self, v: Var) -> &mut f64 {
        match v {
            Var::RotX => &mut self.cube_rot[0],
            Var::RotY => &mut self.cube_rot[1],
            Var::RotZ => &mut self.cube_rot[2],
            Var::PosX => &mut self.pos[0],
            Var::PosY => &mut self.pos[1],
            Var::PosZ => &mut self.pos[2],
        }
    }

    fn tick_oscillators(&mut self) {
        let tick = 0.1 / f64::from(self.speed);
        let mut keep = Vec::with_capacity(self.oscillators.len());
        let taken = std::mem::take(&mut self.oscillators);

        for mut a in taken {
            a.ratio += tick * a.speed;
            if a.ratio > 1.0 {
                a.ratio = 1.0;
            }

            let v = a.from + (a.to - a.from) * ease(Ease::InOutSine, a.ratio);
            *self.var(a.var) = v;

            if a.ratio < 1.0 {
                /* mid cycle */
                keep.push(a);
            } else {
                a.remaining -= 1;
                if a.remaining > 0 {
                    /* keep going the other way */
                    std::mem::swap(&mut a.from, &mut a.to);
                    a.ratio = 0.0;
                    keep.push(a);
                }
                /* ended, and expired: dropped */
            }
        }
        self.oscillators = keep;
    }

    fn add_oscillator(&mut self, var: Var, speed: f64, to: f64, repeat: i32) {
        /* If an oscillator is already running on this variable, don't add
        another. Upstream's loop stops one short of the end of the list, so
        the newest one is never checked; kept, because which oscillators get
        through decides what the picture does. */
        let n = self.oscillators.len();
        if n > 1 && self.oscillators[..n - 1].iter().any(|a| a.var == var) {
            return;
        }

        let from = *self.var(var);
        self.oscillators.insert(
            0,
            Oscillator {
                ratio: 0.0,
                from,
                to,
                speed,
                var,
                remaining: repeat.max(1),
            },
        );
    }

    fn add_random_oscillator(&mut self) {
        let speed = f64::from(self.speed);
        let s1 = speed * 0.07;
        let s2 = speed * 0.3;
        let disp = f64::from(self.thickness + self.displacement);
        let c1 = 1 + if random().is_multiple_of(4) {
            (random() % 3) as i32
        } else {
            0
        };
        let c2 = 2;
        match random() % 6 {
            0 => self.add_oscillator(Var::RotX, s1, 90.0 * randsign(), c1),
            1 => self.add_oscillator(Var::RotY, s1, 90.0 * randsign(), c1),
            2 => self.add_oscillator(Var::RotZ, s1, 90.0 * randsign(), c1),
            3 => self.add_oscillator(Var::PosX, s2, disp * randsign(), c2),
            4 => self.add_oscillator(Var::PosY, s2, disp * randsign(), c2),
            _ => self.add_oscillator(Var::PosZ, s2, disp * randsign(), c2),
        }
    }

    /// Pick a fresh thickness and displacement, which is what decides how many
    /// cubes there are and how far apart they sit.
    fn reroll(&mut self) {
        if random() & 1 != 0 {
            self.thickness = 0.03 + frand(0.02) as f32;
            self.displacement = if random() & 1 != 0 {
                0.0
            } else {
                self.thickness / 3.0
            };
        } else {
            self.thickness = 0.001 + frand(0.02) as f32;
            self.displacement = 0.0;
        }
    }
}

impl Hack3d for CubeTwist {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.clear();

        g.glx.push_matrix();
        g.glx.scale(1.1, 1.1, 1.1);

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 4.0,
            (y as f32 - 0.5) * 4.0,
            (z as f32 - 0.5) * 2.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(6.0, 6.0, 6.0);
        self.draw_cubes(g);
        g.glx.pop_matrix();

        if !down {
            self.tick_oscillators();
        }

        if self.oscillators.is_empty() && !down && random().is_multiple_of(60) {
            self.pos = [0.0; 3];
            self.cube_rot = [0.0; 3];
            self.add_random_oscillator();
        }

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width;
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
            self.oscillators.clear();
            self.reroll();
            self.make_cubes();
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let flat = g.res.bool("flat");
    let spin = g.res.bool("spin");
    let spin_speed = 0.05;
    let wander_speed = 0.005;
    let spin_accel = 1.0;

    let mut st = CubeTwist {
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
        cubes: Vec::new(),
        oscillators: Vec::new(),
        pos: [0.0; 3],
        cube_rot: [0.0; 3],
        speed: g.res.float("speed").max(0.001) as f32,
        thickness: g.res.float("thickness").min(0.5) as f32,
        displacement: g.res.float("displacement").min(0.5) as f32,
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire && !flat {
        // Two lights from below and a yellow specular, which is what puts the
        // warm edge on a strut. With flat shading, which is the default, none
        // of this happens and the struts are their own colour.
        g.glx.lighting(true);
        for (i, pos) in [[0.5, -1.0, -0.5, 0.0], [-0.75, -1.0, 0.0, 0.0]]
            .into_iter()
            .enumerate()
        {
            g.glx.light_enable(i, true);
            g.glx.light_position(i, pos[0], pos[1], pos[2], pos[3]);
            g.glx.light_ambient(i, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(i, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(i, [1.0, 1.0, 1.0, 1.0]);
        }
        g.glx.material_ambient_diffuse([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_specular([1.0, 1.0, 0.0, 1.0]);
        g.glx.material_shininess(30.0);
    }

    // A thickness of nothing is the knob's way of saying "pick one", and the
    // two branches are quite different pictures: thick struts and few cubes,
    // or hair-thin ones and many.
    if st.thickness <= 0.0001 {
        st.reroll();
    }
    st.make_cubes();

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*flat:         True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        1.0",
    "*thickness:    0.0",
    "*displacement: 0.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Animation speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("thickness", "Thickness", 0.0, 0.5, 0.01, 3, "0.0"),
    Opt::slider("displacement", "Displacement", 0.0, 0.5, 0.01, 3, "0.0"),
    Opt::boolean("flat", "Flat shading", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "cubetwist",
    label: "Cube Twist",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2016",
        video: Some("https://www.youtube.com/watch?v=RjrtUtMEa_4"),
        blurb: "A series of nested cubes rotate and slide recursively.",
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

    /// The cubes nest: each is smaller than the one outside it, and they stop
    /// before they run out of room for a strut.
    #[test]
    fn the_cubes_nest_all_the_way_in() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "thickness=0.04&displacement=0.0",
            20260811,
        ));
        r.step();
        let f = r.frame();
        // Twenty-four struts a cube, two fans a strut, four corners a fan. A
        // fan stays a fan in the vertex buffer rather than being cut up.
        let per_cube = 24 * 2 * 4;
        assert_eq!(f.vertices.len() % per_cube, 0, "a partial cube was drawn");
        let cubes = f.vertices.len() / per_cube;
        // A step of 0.08 from a size of 1 leaves about twelve.
        assert!((10..=13).contains(&cubes), "{cubes} cubes");
    }

    /// The nesting is what makes it a twist: each cube is drawn inside the
    /// matrix its parent left behind, so a turn on the outermost compounds all
    /// the way in.
    #[test]
    fn the_twist_compounds_down_the_stack() {
        let mut r = start(StartArgs::new(640, 480, "thickness=0.04", 20260811));
        // Run until an oscillator has turned the outer cube a good way.
        let mut best = 0.0f32;
        let mut seen = 0.0f32;
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            let per_cube = 24 * 2 * 4;
            if f.batches.len() < 2 || f.vertices.len() < per_cube * 3 {
                continue;
            }
            // Compare the first cube's matrix with the innermost one's: the
            // angle between them grows with depth once a twist is under way.
            let first = f.batches[0].modelview;
            let last = f.batches[f.batches.len() - 1].modelview;
            let d: f32 = (0..12).map(|k| (first.0[k] - last.0[k]).abs()).sum();
            best = best.max(d);
            seen += 1.0;
        }
        assert!(seen > 100.0);
        assert!(best > 0.5, "the stack never twisted: {best}");
    }

    /// All the motion is in the matrices: a strut is built once and never
    /// rebuilt, which is what lets the stack twist without any geometry
    /// changing.
    #[test]
    fn the_struts_themselves_never_move() {
        let mut r = start(StartArgs::new(640, 480, "thickness=0.04", 20260811));
        // Measured in model space, where every cube of the stack is the same
        // set of struts and only its matrix differs.
        let spread = |r: &Runner3d| {
            let f = r.frame();
            f.vertices.iter().map(|v| v.pos[0]).fold(f32::MIN, f32::max)
                - f.vertices.iter().map(|v| v.pos[0]).fold(f32::MAX, f32::min)
        };
        r.step();
        let s0 = spread(&r);
        for _ in 0..300 {
            r.step();
        }
        // The struts themselves never change shape, only their matrices do.
        assert!((spread(&r) - s0).abs() < 1e-4);
    }

    /// Space rebuilds the stack with a new thickness, which changes how many
    /// cubes there are.
    #[test]
    fn a_poke_rebuilds_the_stack() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let mut counts = std::collections::BTreeSet::new();
        counts.insert(r.frame().vertices.len());
        for _ in 0..20 {
            r.event(XEvent::KeyPress { key: ' ' });
            r.step();
            counts.insert(r.frame().vertices.len());
        }
        assert!(counts.len() > 1, "every reroll gave the same stack");
    }
}
