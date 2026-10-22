//! Port of `hacks/glx/glschool.c`, `glschool_alg.c` and `glschool_gl.c`.
//!
//! ```text
//! glschool.c, Copyright (c) 2005-2006 David C. Lambert <dcl@panix.com>
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
//! A school of fish, using the classic "Boids" algorithm by Craig Reynolds.
//!
//! Every fish steers by three urges and a fourth that keeps the school
//! together: get away from whoever is too close, swim the way your neighbours
//! are swimming, move towards the middle of them, and head for the goal. They
//! are tried in that order and each is skipped once the acceleration so far is
//! big enough, so a fish about to collide is not also trying to be sociable.
//! The goal is a point that jumps somewhere else in the tank every fifty
//! frames, which is what stops the school settling.
//!
//! A neighbour is anyone within `minradius`, measured on a distance raised to
//! `distexp` rather than the plain one, so the avoidance falls away much faster
//! than linearly and only a fish that is really close pushes hard.
//!
//! The colour is not decoration: it is the angle between where a fish is going
//! and where its neighbours are going, off a 360-entry hue ramp. A school
//! swimming together is one colour, and one that has just been split by the
//! goal moving is a spray of them.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_color_ramp};
use crate::runtime::gl::{Blend, Fog, Shape};
use crate::runtime::shapes::unit_sphere;
use crate::runtime::tube::cone;
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, frand};

/// How many faces round a fish. Upstream's, and it also decides whether the
/// body is drawn at all: three or fewer and the cone is capped instead.
const FACES: i32 = 16;

/// The hue ramp the fish are coloured from, one entry a degree.
const N_COLORS: usize = 360;

#[derive(Clone, Copy, Default)]
struct BBox {
    mins: [f64; 3],
    maxs: [f64; 3],
}

impl BBox {
    fn mid(&self, i: usize) -> f64 {
        (self.maxs[i] + self.mins[i]) / 2.0
    }

    fn range(&self, i: usize) -> f64 {
        self.maxs[i] - self.mins[i]
    }
}

#[derive(Clone, Copy, Default)]
struct Fish {
    pos: [f64; 3],
    vel: [f64; 3],
    accel: [f64; 3],
    old_vel: [f64; 3],
    /// How hard this fish takes its own acceleration, a little either side of
    /// one, so that a school does not move as one body.
    magic: [f64; 3],
    /// Where the neighbours were going last time anyone looked, which is what
    /// the colour is worked out from. Upstream never initialises this and
    /// reads it before it is first written; here it starts at rest.
    avg_vel: [f64; 3],
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn difference(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

struct School {
    max_vel: f64,
    min_vel: f64,
    dist_exp: f64,
    momentum: f64,
    acc_limit: f64,
    min_radius: f64,
    min_radius_exp: f64,
    avoid_fact: f64,
    match_fact: f64,
    center_fact: f64,
    target_fact: f64,
    dist_comp: f64,
    goal: [f64; 3],
    box_mids: [f64; 3],
    box_ranges: [f64; 3],
    the_box: BBox,
    fish: Vec<Fish>,
}

impl School {
    fn set_bbox(&mut self, mins: [f64; 3], maxs: [f64; 3]) {
        self.the_box = BBox { mins, maxs };
        for i in 0..3 {
            self.box_mids[i] = self.the_box.mid(i);
            self.box_ranges[i] = self.the_box.range(i);
        }
    }

    fn init_fishes(&mut self) {
        let (mins, ranges) = (self.the_box.mins, self.box_ranges);
        for f in &mut self.fish {
            for i in 0..3 {
                f.pos[i] = mins[i] + frand(1.0) * ranges[i];
                f.accel[i] = 0.0;
                f.vel[i] = frand(1.0);
                f.magic[i] = 0.70 + 0.60 * frand(1.0);
                f.old_vel[i] = 0.0;
            }
        }
    }

    /// Somewhere else in the tank, wider than it is tall so the school sweeps
    /// across rather than up and down.
    fn new_goal(&mut self) {
        for (i, k) in [0.85, 0.40, 0.85].into_iter().enumerate() {
            self.goal[i] = k * (frand(1.0) - 0.5) * self.box_ranges[i] + self.box_mids[i];
        }
    }

    /// What the neighbours of one fish add up to: where to get away from, where
    /// their middle is, and how fast they are going. Returns how many there
    /// were, since none means the middle two urges are skipped.
    fn group_vectors(&self, r: usize) -> ([f64; 3], [f64; 3], [f64; 3], usize) {
        let (mut avoidance, mut centroid, mut avg_vel) = ([0.0; 3], [0.0; 3], [0.0; 3]);
        let mut count = 0;
        let reference = self.fish[r];

        for (t, test) in self.fish.iter().enumerate() {
            if t == r {
                continue;
            }
            let diff = difference(reference.pos, test.pos);

            let mut dist = norm(diff) - self.dist_comp;
            if dist < 0.0 {
                dist = 0.1;
            }
            let adj_dist = dist.powf(self.dist_exp);
            if adj_dist > self.min_radius_exp {
                continue;
            }

            count += 1;
            for i in 0..3 {
                avg_vel[i] += test.vel[i];
                centroid[i] += test.pos[i];
                avoidance[i] += diff[i] / adj_dist;
            }
        }
        if count > 0 {
            for i in 0..3 {
                avg_vel[i] /= count as f64;
                centroid[i] /= count as f64;
            }
        }
        (avoidance, centroid, avg_vel, count)
    }

    /// The four urges, in order, each one skipped once the acceleration so far
    /// has reached the limit.
    fn compute_accelerations(&mut self) {
        for r in 0..self.fish.len() {
            let (avoidance, centroid, avg_vel, count) = self.group_vectors(r);
            let (pos, vel) = (self.fish[r].pos, self.fish[r].vel);

            // Get away from whoever is too close.
            let mut acc = [
                avoidance[0] * self.avoid_fact,
                avoidance[1] * self.avoid_fact,
                avoidance[2] * self.avoid_fact,
            ];

            if count > 0 && norm(acc) < self.acc_limit {
                // Swim the way they are swimming.
                self.fish[r].avg_vel = avg_vel;
                for j in 0..3 {
                    acc[j] += (avg_vel[j] - vel[j]) * self.match_fact;
                }

                // And towards the middle of them.
                if norm(acc) < self.acc_limit {
                    for j in 0..3 {
                        acc[j] += (centroid[j] - pos[j]) * self.center_fact;
                    }
                }
            }

            // And, if there is still room, head for the goal.
            if norm(acc) < self.acc_limit {
                let diff = difference(self.goal, pos);
                let mut dist = norm(diff) - self.dist_comp;
                if dist < 0.0 {
                    dist = 0.1;
                }
                if dist > self.min_radius {
                    let adj_dist = dist.powf(self.dist_exp);
                    for j in 0..3 {
                        acc[j] += diff[j] * self.target_fact / adj_dist;
                    }
                }
            }

            self.fish[r].accel = acc;
        }
    }

    fn apply_movements(&mut self) {
        let (bbox, min_vel, max_vel, momentum) =
            (self.the_box, self.min_vel, self.max_vel, self.momentum);

        for f in &mut self.fish {
            let mut v_mag = 0.0;
            for i in 0..3 {
                // A fish that has left the tank on this axis does not take its
                // acceleration on it, so nothing drives it further out.
                let oob = f.pos[i] > bbox.maxs[i] || f.pos[i] < bbox.mins[i];
                if !oob {
                    f.vel[i] += f.accel[i] * f.magic[i];
                }
                v_mag += f.vel[i] * f.vel[i];
            }
            v_mag = v_mag.sqrt();

            // Upstream divides by this without looking. It cannot be zero for a
            // fish that started with a random velocity, but a knob set to
            // something strange should not put a NaN on the screen.
            if v_mag > 0.0 {
                let s = if v_mag > max_vel {
                    Some(max_vel / v_mag)
                } else if v_mag < min_vel {
                    Some(min_vel / v_mag)
                } else {
                    None
                };
                if let Some(s) = s {
                    for i in 0..3 {
                        f.vel[i] *= s;
                    }
                }
            }

            for i in 0..3 {
                f.vel[i] = momentum * f.old_vel[i] + (1.0 - momentum) * f.vel[i];
                f.pos[i] += f.vel[i];
                f.old_vel[i] = f.vel[i];

                // Out one side and in the other.
                if f.pos[i] < bbox.mins[i] {
                    f.pos[i] = bbox.maxs[i];
                } else if f.pos[i] > bbox.maxs[i] {
                    f.pos[i] = bbox.mins[i];
                }
            }
        }
    }
}

/// The axis to turn `v` about to bring `+z` onto it, and by how much. Returns
/// the axis and the angle in degrees.
///
/// The angle comes out of an arcsine, so it is only ever 0 to 90; the caller
/// takes it round the other way when the vector points backwards.
fn normal_and_theta_to_plus_z(v: [f64; 3]) -> ([f64; 3], f64) {
    let v_norm = norm(v);
    if v_norm == 0.0 {
        return ([0.0, 1.0, 0.0], 0.0);
    }
    // The cross product of +z with v, written out.
    let x_v = [-v[1], v[0], 0.0];
    let sin_theta = norm(x_v) / v_norm;
    (x_v, sin_theta.asin().to_degrees())
}

struct GlSchool {
    school: School,
    colors: Vec<XColor>,
    wireframe: bool,
    draw_goal: bool,
    draw_bbox: bool,
    goal_chg_freq: i32,
    goal_counter: i32,

    bbox_list: u32,
    goal_list: u32,
    fish_list: u32,
}

impl GlSchool {
    /// The five inside walls of the tank, each a slightly different blue, so
    /// that a corner reads as a corner.
    fn draw_bounding_box(&self, g: &mut Gl) {
        let b = self.school.the_box;
        let (x0, y0, z0) = (b.mins[0] as f32, b.mins[1] as f32, b.mins[2] as f32);
        let (x1, y1, z1) = (b.maxs[0] as f32, b.maxs[1] as f32, b.maxs[2] as f32);
        let shape = if self.wireframe {
            Shape::LineLoop
        } else {
            Shape::Quads
        };

        g.glx.front_face_cw(false);
        for (blue, quad) in [
            // back
            (
                0.15,
                [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]],
            ),
            // left
            (
                0.2,
                [[x0, y0, z1], [x0, y0, z0], [x0, y1, z0], [x0, y1, z1]],
            ),
            // right
            (
                0.2,
                [[x1, y0, z0], [x1, y0, z1], [x1, y1, z1], [x1, y1, z0]],
            ),
            // top
            (
                0.1,
                [[x1, y1, z1], [x0, y1, z1], [x0, y1, z0], [x1, y1, z0]],
            ),
            // bottom
            (
                0.3,
                [[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]],
            ),
        ] {
            g.glx.begin(shape);
            g.glx.color4f(0.0, 0.0, blue, 1.0);
            for v in quad {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();
        }
    }

    fn create_bbox_list(&mut self, g: &mut Gl) {
        if self.bbox_list == 0 {
            self.bbox_list = g.glx.gen_lists(1);
        }
        g.glx.new_list(self.bbox_list);
        self.draw_bounding_box(g);
        g.glx.end_list();
    }

    /// A fish is a cone with a ball on the blunt end, pointing along `+z`.
    fn create_draw_lists(&mut self, g: &mut Gl) {
        let wire = self.wireframe;

        self.goal_list = g.glx.gen_lists(1);
        g.glx.new_list(self.goal_list);
        g.glx.scale(5.0, 5.0, 5.0);
        unit_sphere(&mut g.glx, 10, 10, wire);
        g.glx.end_list();

        self.fish_list = g.glx.gen_lists(1);
        g.glx.new_list(self.fish_list);
        cone(
            &mut g.glx,
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 10.0],
            2.0,
            0.0,
            FACES,
            true,
            FACES <= 3,
            wire,
        );
        g.glx.translate(0.0, 0.0, -0.3);
        g.glx.scale(2.0, 2.0, 2.0);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        if FACES > 3 {
            unit_sphere(&mut g.glx, FACES, FACES, wire);
        }
        g.glx.end_list();
    }
}

impl Hack3d for GlSchool {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        if self.goal_counter % self.goal_chg_freq.max(1) == 0 {
            self.school.new_goal();
        }
        self.goal_counter += 1;

        self.school.apply_movements();

        g.glx.clear();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        if self.draw_bbox {
            g.glx.lighting(false);
            g.glx.call_list(self.bbox_list);
            g.glx.lighting(!self.wireframe);
        }

        if self.draw_goal {
            g.glx.push_matrix();
            let goal = self.school.goal;
            g.glx
                .translate(goal[0] as f32, goal[1] as f32, goal[2] as f32);
            g.glx.material_ambient_diffuse([1.0, 0.0, 0.0, 1.0]);
            g.glx.color4f(1.0, 0.0, 0.0, 1.0);
            g.glx.call_list(self.goal_list);
            g.glx.pop_matrix();
        }

        for i in 0..self.school.fish.len() {
            let f = self.school.fish[i];
            let (_, col_theta) = normal_and_theta_to_plus_z(f.avg_vel);
            let (x_vect, rot_theta) = normal_and_theta_to_plus_z(f.vel);

            // Past ninety degrees the arcsine folds back, so a fish heading
            // away is taken round the other side.
            let col_theta = if f.avg_vel[2] < 0.0 {
                180.0 - col_theta
            } else {
                col_theta
            };
            let rot_theta = if f.vel[2] < 0.0 {
                180.0 - rot_theta
            } else {
                rot_theta
            };

            let c = self.colors[(col_theta + 240.0) as usize % N_COLORS];
            let rgb = [
                f32::from(c.red) / 65535.0,
                f32::from(c.green) / 65535.0,
                f32::from(c.blue) / 65535.0,
                1.0,
            ];
            // Upstream sets this with glColor under GL_COLOR_MATERIAL, which is
            // the same thing said the shorter way.
            g.glx.material_ambient_diffuse(rgb);
            g.glx.color4f(rgb[0], rgb[1], rgb[2], 1.0);

            g.glx.push_matrix();
            g.glx
                .translate(f.pos[0] as f32, f.pos[1] as f32, f.pos[2] as f32);
            g.glx.rotate(
                (180.0 + rot_theta) as f32,
                x_vect[0] as f32,
                x_vect[1] as f32,
                x_vect[2] as f32,
            );
            g.glx.call_list(self.fish_list);
            g.glx.pop_matrix();
        }

        self.school.compute_accelerations();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let aspect = f64::from(width) / f64::from(height.max(1));
        self.school.set_bbox(
            [-aspect * 160.0, -130.0, -450.0],
            [aspect * 160.0, 130.0, -50.0],
        );
        self.create_bbox_list(g);

        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(60.0, aspect as f32, 0.1, 451.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let n_fish = g.res.int("nfish").clamp(1, 5000) as usize;
    let min_radius = g.res.float("minradius");
    let dist_exp = g.res.float("distexp");

    let mut st = GlSchool {
        school: School {
            max_vel: g.res.float("maxvel"),
            min_vel: g.res.float("minvel"),
            dist_exp,
            momentum: g.res.float("momentum"),
            acc_limit: g.res.float("acclimit"),
            min_radius,
            min_radius_exp: min_radius.powf(dist_exp),
            avoid_fact: g.res.float("avoidfact"),
            match_fact: g.res.float("matchfact"),
            center_fact: g.res.float("centerfact"),
            target_fact: g.res.float("targetfact"),
            dist_comp: g.res.float("distcomp"),
            goal: [0.0; 3],
            box_mids: [0.0; 3],
            box_ranges: [0.0; 3],
            the_box: BBox::default(),
            fish: vec![Fish::default(); n_fish],
        },
        colors: make_color_ramp(0, 1.0, 1.0, 359, 1.0, 1.0, N_COLORS, false),
        wireframe: g.res.bool("wireframe"),
        draw_goal: g.res.bool("drawgoal"),
        draw_bbox: g.res.bool("drawbbox"),
        goal_chg_freq: g.res.int("goalchgf"),
        goal_counter: 0,
        bbox_list: 0,
        goal_list: 0,
        fish_list: 0,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    g.glx.depth_test(true);
    g.glx.cull_face(true);
    if !st.wireframe {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_position(0, 0.0, 50.0, -50.0, 1.0);
        g.glx.light_ambient(0, [0.1, 0.1, 0.1, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_ambient_diffuse([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);
    }
    g.glx.blend(Blend::Off);
    if g.res.bool("fog") {
        g.glx.fog(Some(Fog::Exp2 {
            density: 0.0025,
            color: [0.0, 0.0, 0.15, 1.0],
        }));
    }

    st.school.init_fishes();
    st.create_draw_lists(g);
    st.school.compute_accelerations();

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*nfish:        100",
    "*fog:          False",
    "*drawbbox:     True",
    "*drawgoal:     False",
    "*goalchgf:     50",
    "*maxvel:       7.0",
    "*minvel:       1.0",
    "*acclimit:     8.0",
    "*distexp:      2.2",
    "*avoidfact:    1.5",
    "*matchfact:    0.15",
    "*centerfact:   0.1",
    "*targetfact:   80",
    "*minradius:    30.0",
    "*momentum:     0.9",
    "*distcomp:     10.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("nfish", "Fish count", 5.0, 500.0, 1.0, 0, "100"),
    Opt::slider("avoidfact", "Avoidance", 0.0, 10.0, 0.1, 2, "1.5"),
    Opt::slider("matchfact", "Velocity matching", 0.0, 3.0, 0.05, 2, "0.15"),
    Opt::slider("centerfact", "Centering", 0.0, 1.0, 0.01, 2, "0.1"),
    Opt::slider("targetfact", "Goal following", 0.0, 400.0, 5.0, 0, "80"),
    Opt::boolean("drawbbox", "Draw bounding box", "true"),
    Opt::boolean("drawgoal", "Draw goal", "false"),
    Opt::boolean("fog", "Fog", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glschool",
    label: "GL School",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "David C. Lambert and Jamie Zawinski",
        year: "2006",
        video: Some("https://www.youtube.com/watch?v=SuMIatcSPdU"),
        blurb: "A school of fish, using the classic \"Boids\" algorithm by \
                Craig Reynolds.",
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
    use crate::runtime::gl::Primitive;

    fn a_school(n: usize) -> School {
        let mut s = School {
            max_vel: 7.0,
            min_vel: 1.0,
            dist_exp: 2.2,
            momentum: 0.9,
            acc_limit: 8.0,
            min_radius: 30.0,
            min_radius_exp: 30.0f64.powf(2.2),
            avoid_fact: 1.5,
            match_fact: 0.15,
            center_fact: 0.1,
            target_fact: 80.0,
            dist_comp: 10.0,
            goal: [0.0; 3],
            box_mids: [0.0; 3],
            box_ranges: [0.0; 3],
            the_box: BBox::default(),
            fish: vec![Fish::default(); n],
        };
        s.set_bbox([-200.0, -130.0, -450.0], [200.0, 130.0, -50.0]);
        s.init_fishes();
        s.new_goal();
        s
    }

    /// The tank has no lid: a fish that swims out of one wall comes back in
    /// through the opposite one, so the school never escapes.
    #[test]
    fn no_fish_ever_leaves_the_tank() {
        let mut s = a_school(30);
        for _ in 0..600 {
            s.compute_accelerations();
            s.apply_movements();
            for f in &s.fish {
                for i in 0..3 {
                    assert!(
                        f.pos[i] >= s.the_box.mins[i] && f.pos[i] <= s.the_box.maxs[i],
                        "a fish is at {:?}, outside {:?}..{:?}",
                        f.pos,
                        s.the_box.mins,
                        s.the_box.maxs
                    );
                }
            }
        }
    }

    /// Speed is clamped before the momentum blend, and the blend is a mix of
    /// two vectors that were themselves within the limit, so nothing ever ends
    /// up going faster than `maxvel`.
    #[test]
    fn nothing_swims_faster_than_the_limit() {
        let mut s = a_school(25);
        for _ in 0..400 {
            s.compute_accelerations();
            s.apply_movements();
            for f in &s.fish {
                assert!(
                    norm(f.vel) <= s.max_vel + 1e-9,
                    "a fish is doing {}, over {}",
                    norm(f.vel),
                    s.max_vel
                );
            }
        }
    }

    /// Fish out of range of each other are not neighbours, and the count is
    /// what decides whether the middle two urges are used at all.
    #[test]
    fn only_the_near_ones_count_as_neighbours() {
        let mut s = a_school(3);
        // One at the origin, one just beside it and one right across the tank.
        s.fish[0].pos = [0.0, 0.0, -250.0];
        s.fish[1].pos = [5.0, 0.0, -250.0];
        s.fish[2].pos = [190.0, 120.0, -60.0];

        let (_, centroid, _, count) = s.group_vectors(0);
        assert_eq!(count, 1, "the far one should not be a neighbour");
        assert!((centroid[0] - 5.0).abs() < 1e-9);

        // And with nobody near, nothing at all.
        s.fish[1].pos = [-190.0, -120.0, -440.0];
        assert_eq!(s.group_vectors(0).3, 0);
    }

    /// Avoidance comes first and, close in, is strong enough on its own to use
    /// up the whole acceleration budget, which is what stops a fish about to
    /// collide from also trying to be sociable.
    #[test]
    fn avoidance_crowds_out_the_other_urges() {
        let mut s = a_school(2);
        s.fish[0].pos = [0.0, 0.0, -250.0];
        s.fish[0].vel = [0.0, 0.0, 1.0];
        s.fish[1].pos = [10.5, 0.0, -250.0];
        s.fish[1].vel = [0.0, 0.0, 1.0];
        s.goal = [0.0, 0.0, -250.0];
        s.compute_accelerations();

        let acc = s.fish[0].accel;
        assert!(norm(acc) >= s.acc_limit, "not crowded out: {acc:?}");
        // Pushed away from the other fish, which is along -x.
        assert!(acc[0] < 0.0, "pushed the wrong way: {acc:?}");
        // And nothing was borrowed from matching, since that would have written
        // the neighbour's velocity down.
        assert_eq!(s.fish[0].avg_vel, [0.0; 3]);
    }

    /// The goal jumps somewhere else every fifty frames and stays put in
    /// between, which is visible as the red ball not moving.
    #[test]
    fn the_goal_moves_on_the_beat() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "nfish=5&drawgoal=true&drawbbox=false",
            20260811,
        ));
        let mut seen = Vec::new();
        for _ in 0..120 {
            r.step();
            // The goal ball is drawn before any fish.
            seen.push(r.frame().batches[0].mvp.0);
        }
        // A new goal on the frames where the counter comes back round to zero,
        // which is the first and every fiftieth after it.
        assert_eq!(seen[0], seen[49], "the goal moved mid-run");
        assert_ne!(seen[49], seen[50], "the goal did not move on the fiftieth");
        assert_eq!(seen[50], seen[99]);
        assert_ne!(seen[99], seen[100]);
    }

    /// The tank walls are drawn unlit, so they stay the flat blue they were
    /// given, and the fish are lit.
    #[test]
    fn the_walls_are_unlit_and_the_fish_are_not() {
        let mut r = start(StartArgs::new(640, 480, "nfish=4", 20260811));
        r.step();
        let f = r.frame();

        // The five walls are one batch: nothing changes between them but the
        // colour, and that is per vertex.
        let walls: Vec<_> = f.batches.iter().take_while(|b| !b.lighting).collect();
        assert_eq!(walls.len(), 1, "the walls should have folded into one");
        assert_eq!(walls[0].primitive, Primitive::Triangles);
        assert_eq!(walls[0].count, 5 * 6, "five quads, two triangles each");

        let blues: std::collections::BTreeSet<_> = f.vertices[..walls[0].count]
            .iter()
            .map(|v| v.color[2].to_bits())
            .collect();
        // Four shades over five walls: the two sides are the same, so it is the
        // floor and the ceiling that tell you which way up you are.
        assert_eq!(blues.len(), 4, "the walls are not shaded apart");
        assert!(
            f.vertices[..walls[0].count]
                .iter()
                .all(|v| v.color[0] == 0.0 && v.color[1] == 0.0),
            "a wall came out some colour other than blue"
        );

        assert!(
            f.batches.iter().skip(1).all(|b| b.lighting),
            "a fish was drawn unlit"
        );
    }

    /// A fish is a cone with a ball on it, so the count of batches follows the
    /// count of fish.
    #[test]
    fn there_are_as_many_fish_as_asked_for() {
        for n in [5usize, 17, 60] {
            let mut r = start(StartArgs::new(640, 480, &format!("nfish={n}"), 20260811));
            r.step();
            let lit = r.frame().batches.iter().filter(|b| b.lighting).count();
            assert_eq!(lit, n * 2, "{n} fish should be {} batches", n * 2);
        }
    }
}
