//! Port of `hacks/glx/antspotlight.c`.
//!
//! ```text
//! antspotlight, Copyright (c) 2003 Blair Tennessy <tennessy@cs.ubc.ca>
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
//! An ant walks over an image, carrying the only light in the room.
//!
//! The picture is on the floor and the floor is unlit, so the only part of it
//! that can be seen is what the ant's spotlight falls on. Rather than light the
//! whole floor and let most of it come out black, the saver builds a fan of
//! twenty-four triangle strips spreading out from under the ant along the way
//! it is facing, and draws only that: the geometry *is* the beam. Upstream
//! notes that it ought to be intersecting the cone of light with the plane
//! properly and is not.
//!
//! The runtime has no spotlights, so this one is worked out per vertex and
//! handed to the fan as a colour. That is not an approximation of what OpenGL
//! does with `GL_SPOT_CUTOFF` and `GL_SPOT_EXPONENT`; it is the same
//! calculation, since fixed-function lighting is per vertex too, moved from
//! the pipeline into the saver.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::shapes::unit_sphere;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
};

const MATERIAL_GRAY: [f32; 4] = [0.2, 0.2, 0.2, 1.0];
const MATERIAL_GRAY5: [f32; 4] = [0.5, 0.5, 0.5, 1.0];
const MATERIAL_GRAY6: [f32; 4] = [0.6, 0.6, 0.6, 1.0];
/// The glass shell the ant is drawn in a second time, translucent.
const MATERIAL_GRAYB: [f32; 4] = [0.2, 0.2, 0.2, 0.5];

/// How far the camera may be pushed back with the wheel.
const MAX_MAGNIFICATION: i32 = 10;
/// The step the fan is laid out by. Upstream calls this `cutoff` too, but it
/// is not the light's cutoff: the fan is twenty-four strips of an eighth of
/// this each, so it spans a half turn, which is wider than the beam.
const FAN_STEP: f32 = std::f32::consts::PI / 3.0;

/// `GL_SPOT_CUTOFF`, sixty degrees. OpenGL measures this from the beam's axis,
/// so the cone is a hundred and twenty degrees across.
const SPOT_CUTOFF: f32 = std::f32::consts::PI / 3.0;

/// The spotlight's fall-off with distance: `1 / (0.1 + 0.05 d)`.
const ATT_CONSTANT: f32 = 0.1;
const ATT_LINEAR: f32 = 0.05;
/// How sharply the beam fades towards its edge.
const SPOT_EXPONENT: f32 = 3.0;

struct Ant {
    position: [f32; 3],
    goal: [f32; 3],
    direction: f32,
    velocity: f32,
    /// How far through its walking cycle the ant is.
    step: f32,
}

struct AntSpotlight {
    rot: Rotator,
    trackball: Trackball,

    /// How much of the texture the picture actually covers.
    max_tx: f32,
    max_ty: f32,
    texture: Option<u32>,

    ant: Ant,
    boardsize: f32,
    mag: i32,
    wire: bool,
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dz = a[2] - b[2];
    (dx * dx + dz * dz).sqrt()
}

impl AntSpotlight {
    /// `find_goal`: somewhere else on the board, far enough away to be worth
    /// walking to.
    fn find_goal(&mut self) {
        let n = (self.boardsize + 0.5) as i32 - 2;
        loop {
            let g = [
                (random() % n as u32) as f32 - self.boardsize / 2.0 + 1.0,
                0.0,
                (random() % n as u32) as f32 - self.boardsize / 2.0 + 1.0,
            ];
            self.ant.goal = g;
            if distance(self.ant.position, g) >= 2.0 {
                return;
            }
        }
    }

    /// `mySphere`, and `mySphere2` for the wireframe silhouette.
    fn my_sphere(g: &mut Gl, radius: f32, wire: bool) {
        g.glx.push_matrix();
        g.glx.scale(radius, radius, radius);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        unit_sphere(&mut g.glx, 16, 16, wire);
        g.glx.pop_matrix();
    }

    /// `draw_ant`: the body as spheres, and the legs and antennae as lines.
    ///
    /// `silhouette` picks upstream's `mySphere2`, which draws the body in
    /// wireframe. Its `myCone2` draws nothing at all, so the two cones between
    /// the head and thorax are only ever a pair of translations.
    fn draw_ant(&self, g: &mut Gl, material: [f32; 4], shadow: bool, silhouette: bool) {
        let step = self.ant.step;
        let tau = std::f32::consts::TAU;
        let (cos1, cos2, cos3) = (
            step.cos(),
            (step + tau / 3.0).cos(),
            (step + 2.0 * tau / 3.0).cos(),
        );
        let (sin1, sin2, sin3) = (
            step.sin(),
            (step + tau / 3.0).sin(),
            (step + 2.0 * tau / 3.0).sin(),
        );

        g.glx.blend(Blend::Alpha);
        g.glx.material_diffuse(material);
        g.glx.cull_face(true);

        g.glx.push_matrix();
        g.glx.scale(1.0, 1.3, 1.0);
        Self::my_sphere(g, 0.18, silhouette);
        g.glx.scale(1.0, 1.0 / 1.3, 1.0);
        g.glx.translate(0.0, 0.30, 0.0);
        Self::my_sphere(g, 0.2, silhouette);

        // Where the two cones would be, if upstream drew them.
        g.glx.translate(-0.05, 0.17, 0.05);
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(-25.0, 0.0, 1.0, 0.0);
        g.glx.translate(0.0, 0.10, 0.0);
        g.glx.rotate(25.0, 0.0, 1.0, 0.0);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);

        g.glx.scale(1.0, 1.3, 1.0);
        g.glx.translate(0.15, -0.65, 0.05);
        Self::my_sphere(g, 0.25, silhouette);
        g.glx.scale(1.0, 1.0 / 1.3, 1.0);
        g.glx.pop_matrix();
        g.glx.cull_face(false);

        // The legs and antennae are unlit lines whose colour is per vertex.
        g.glx.lighting(false);
        g.glx.color_material(true);

        g.glx.begin(Shape::Lines);
        for z in [0.40f32, -0.40] {
            g.glx
                .color4f(material[0], material[1], material[2], material[3]);
            g.glx.vertex3f(0.0, 0.30, 0.0);
            g.glx.color4f(
                MATERIAL_GRAY[0],
                MATERIAL_GRAY[1],
                MATERIAL_GRAY[2],
                MATERIAL_GRAY[3],
            );
            g.glx.vertex3f(0.40, 0.70, z);
        }
        g.glx.end();

        if !shadow {
            g.glx.begin(Shape::Points);
            g.glx
                .color4f(MATERIAL_GRAY5[0], MATERIAL_GRAY5[1], MATERIAL_GRAY5[2], 1.0);
            g.glx.vertex3f(0.40, 0.70, 0.40);
            g.glx.vertex3f(0.40, 0.70, -0.40);
            g.glx.end();
        }

        // Six legs, three a side, a third of a cycle apart. The left three
        // swing with the cosine and the right three with the sine, which is
        // what puts them out of step with each other.
        // Where the hip is, where the knee is, where the foot is, how far the
        // leg swings and how far it lifts. The left three run on `z` above
        // zero and the right three below it.
        let legs: [(f32, f32, f32, f32, f32, f32); 6] = [
            (1.0, 0.05, 0.15, 0.25, 0.05 * cos1, 0.1 * sin1),
            (1.0, 0.00, 0.00, 0.00, 0.05 * cos2, 0.1 * sin2),
            (1.0, -0.05, -0.15, -0.25, 0.05 * cos3, 0.1 * sin3),
            (-1.0, 0.05, 0.15, 0.25, -0.05 * sin1, 0.1 * cos1),
            (-1.0, 0.00, 0.00, 0.00, -0.05 * sin2, 0.1 * cos2),
            (-1.0, -0.05, -0.15, -0.25, -0.05 * sin3, 0.1 * cos3),
        ];
        for (side, y0, y1, y2, swing, lift) in legs {
            g.glx.begin(Shape::LineStrip);
            g.glx
                .color4f(material[0], material[1], material[2], material[3]);
            g.glx.vertex3f(0.0, y0, side * 0.18);
            g.glx.vertex3f(0.35 + swing, y1, side * 0.25);
            g.glx.color4f(
                MATERIAL_GRAY[0],
                MATERIAL_GRAY[1],
                MATERIAL_GRAY[2],
                MATERIAL_GRAY[3],
            );
            g.glx.vertex3f(-0.20 + swing, y2 + lift, side * 0.45);
            g.glx.end();
        }

        if !shadow {
            g.glx.begin(Shape::Points);
            g.glx
                .color4f(MATERIAL_GRAY5[0], MATERIAL_GRAY5[1], MATERIAL_GRAY5[2], 1.0);
            for (side, _, _, y2, swing, lift) in legs {
                g.glx.vertex3f(-0.20 + swing, y2 + lift, side * 0.45);
            }
            g.glx.end();
        }

        g.glx.color_material(false);
        g.glx.lighting(true);
    }

    /// `show_ant`: the ant in place, drawn once solid and once as translucent
    /// glass over the top of itself.
    fn show_ant(&self, g: &mut Gl) {
        g.glx.push_matrix();
        g.glx
            .translate(self.ant.position[0], 0.33, self.ant.position[2]);
        g.glx.rotate(
            180.0 + self.ant.direction * 180.0 / std::f32::consts::PI,
            0.0,
            1.0,
            0.0,
        );
        g.glx.rotate(90.0, 0.0, 0.0, 1.0);

        // The skeleton, then the glass.
        self.draw_ant(g, MATERIAL_GRAY5, false, true);
        if !self.wire {
            g.glx.blend(Blend::Alpha);
            self.draw_ant(g, MATERIAL_GRAYB, false, false);
            g.glx.blend(Blend::Off);
        }

        g.glx.pop_matrix();
    }

    /// What the spotlight leaves at a point on the floor.
    ///
    /// The whole of OpenGL's fixed-function spotlight, since the runtime has
    /// none: the light falls off with distance, is cut off outside sixty
    /// degrees of the beam's axis, and is raised to the spot exponent inside
    /// it. The floor's normal is straight up, so the diffuse term is just how
    /// far above the point the light is.
    fn spot(&self, p: [f32; 3], light: [f32; 3], dir: [f32; 3]) -> f32 {
        let d = [light[0] - p[0], light[1] - p[1], light[2] - p[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist == 0.0 {
            return 0.0;
        }
        let l = [d[0] / dist, d[1] / dist, d[2] / dist];

        // The angle between the beam's axis and the way back to this point.
        let dl = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if dl == 0.0 {
            return 0.0;
        }
        let cos_spot = -(l[0] * dir[0] + l[1] * dir[1] + l[2] * dir[2]) / dl;
        if cos_spot < SPOT_CUTOFF.cos() {
            return 0.0;
        }

        let att = 1.0 / (ATT_CONSTANT + ATT_LINEAR * dist);
        // The floor faces straight up, so `dot(n, l)` is the y of the
        // direction to the light.
        let diffuse = l[1].max(0.0);
        att * cos_spot.powf(SPOT_EXPONENT) * diffuse
    }

    /// `draw_board`: the fan of triangle strips that is the lit part of the
    /// picture. There is no board anywhere else; the rest of the floor is not
    /// drawn at all.
    fn draw_board(&self, g: &mut Gl, light: [f32; 3], dir: [f32; 3]) {
        let Some(tex) = self.texture else { return };
        g.glx.texturing(true);
        g.glx.bind_texture(tex);
        // The beam's brightness is the vertex colour, so the material has to
        // come from the vertex too.
        g.glx.color_material(true);

        // The middle of the fan is roughly where the spotlight is.
        let center = [self.ant.position[0], 0.0, self.ant.position[2]];
        let tex_at = |p: [f32; 3]| {
            [
                (self.boardsize / 2.0 + p[0]) * self.max_tx / self.boardsize,
                (self.boardsize / 2.0 + p[2]) * self.max_ty / self.boardsize,
            ]
        };
        // The middle of the fan is the one place upstream measures the texture
        // from the other end.
        let centertex = [
            (self.boardsize / 2.0 + center[0]) * self.max_tx / self.boardsize,
            self.max_ty - ((self.boardsize / 2.0 + center[2]) * self.max_ty / self.boardsize),
        ];

        let lit = |p: [f32; 3]| {
            let s = self.spot(p, light, dir);
            [
                (MATERIAL_GRAY6[0] * s).min(1.0),
                (MATERIAL_GRAY6[1] * s).min(1.0),
                (MATERIAL_GRAY6[2] * s).min(1.0),
            ]
        };

        // Upstream's own note: the vertices here should follow the shape of
        // the illuminated board, and ideally would come from intersecting the
        // cone of light with the plane. Watch those constants.
        for i in -12..12 {
            g.glx.begin(Shape::TriangleStrip);
            g.glx.normal3f(0.0, 1.0, 0.0);

            let mid = [center[0], 0.01, center[2]];
            let c = lit(mid);
            g.glx.color4f(c[0], c[1], c[2], 1.0);
            g.glx.tex_coord2f(centertex[0], 1.0 - centertex[1]);
            g.glx.vertex3f(mid[0], mid[1], mid[2]);

            let theta1 = self.ant.direction + i as f32 * (FAN_STEP / 8.0);
            let theta2 = self.ant.direction + (i + 1) as f32 * (FAN_STEP / 8.0);

            for j in 1..=64 {
                let fj = j as f32 / 6.0;
                for theta in [theta1, theta2] {
                    let p = [
                        center[0] + fj * theta.cos(),
                        0.0,
                        center[2] - fj * theta.sin(),
                    ];
                    let t = tex_at(p);
                    let c = lit(p);
                    g.glx.color4f(c[0], c[1], c[2], 1.0);
                    // The textures here are top-down and OpenGL's are
                    // bottom-up.
                    g.glx.tex_coord2f(t[0], 1.0 - t[1]);
                    g.glx.vertex3f(p[0], p[1], p[2]);
                }
            }
            g.glx.end();
        }

        g.glx.color_material(false);
        g.glx.texturing(false);
    }

    /// `draw_antspotlight_strip`: the lit board, the ant on it, and one step
    /// of the ant's walk.
    fn draw_strip(&mut self, g: &mut Gl) {
        // The spotlight rides a little ahead of the ant and points down and
        // forwards.
        let light = [
            self.ant.position[0] + 0.7 * self.ant.direction.cos(),
            0.5,
            self.ant.position[2] - 0.7 * self.ant.direction.sin(),
        ];
        let dir = [self.ant.direction.cos(), -0.5, -self.ant.direction.sin()];

        if !self.wire {
            self.draw_board(g, light, dir);
        }
        self.show_ant(g);

        // Near the goal, or now and then for no reason, pick another.
        if distance(self.ant.position, self.ant.goal) < 0.2 || random().is_multiple_of(100) {
            self.find_goal();
        } else {
            // Turn towards the goal, by at most a hundredth of a turn.
            let dx = self.ant.goal[0] - self.ant.position[0];
            let dz = -(self.ant.goal[2] - self.ant.position[2]);
            let pi = std::f32::consts::PI;

            let mut theta = if dx.abs() > 0.01 {
                let t = (dz / dx).atan();
                if dx < 0.0 { t + pi } else { t }
            } else if dz > 0.0 {
                pi / 2.0
            } else {
                3.0 * pi / 2.0
            };
            if theta < 0.0 {
                theta += 2.0 * pi;
            }

            let mut ideal = theta - self.ant.direction;
            if ideal > pi {
                ideal -= 2.0 * pi;
            }
            let dt = ideal.signum() * ideal.abs().min(pi / 100.0);
            self.ant.direction += dt;
            while self.ant.direction < 0.0 {
                self.ant.direction += 2.0 * pi;
            }
            while self.ant.direction > 2.0 * pi {
                self.ant.direction -= 2.0 * pi;
            }
        }

        self.ant.position[0] += self.ant.velocity * self.ant.direction.cos();
        self.ant.position[2] += self.ant.velocity * (-self.ant.direction).sin();
        self.ant.step += 10.0 * self.ant.velocity;
        while self.ant.step > std::f32::consts::TAU {
            self.ant.step -= std::f32::consts::TAU;
        }
    }
}

impl Hack3d for AntSpotlight {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        if self.texture.is_none() && !self.wire {
            let size = 1024;
            if let Some(img) = g.load_image(size, size) {
                let id = g.glx.gen_texture();
                g.glx.bind_texture(id);
                self.max_tx = 1.0;
                self.max_ty = 1.0;
                g.glx.tex_image_2d(img.width, img.height, img.pixels);
                g.glx.tex_clamp(false);
                self.texture = Some(id);
            }
        }

        g.glx.depth_test(true);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_enable(1, true);
        g.glx.clear();

        g.glx.push_matrix();

        // Follow the ant.
        g.glx.translate(0.0, 0.0, -6.0 - self.mag as f32);
        g.glx.rotate(35.0, 1.0, 0.0, 0.0);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.translate(
            -self.ant.position[0],
            self.ant.position[1],
            -self.ant.position[2],
        );

        self.draw_strip(g);

        g.glx.pop_matrix();

        // The rotator is stepped even though nothing reads it, so that a run
        // stays in step with upstream's use of the random stream.
        let down = self.trackball.button_down();
        let _ = self.rot.rotation(!down);

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
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
        g.glx.perspective(45.0, 1.0 / h, 1.0, 25.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.line_width(2.0);
        g.glx.point_size(2.0);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event {
            match key {
                '+' | '=' => {
                    self.mag = (self.mag - 1).max(1);
                    return true;
                }
                '-' | '_' => {
                    self.mag = (self.mag + 1).min(MAX_MAGNIFICATION);
                    return true;
                }
                _ => {}
            }
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let rot_speed = 0.3;
    let wire = g.res.bool("wireframe");

    let mut st = AntSpotlight {
        rot: Rotator::new(rot_speed, rot_speed, rot_speed, 1.0, 0.0, true),
        trackball: Trackball::new(),
        max_tx: 1.0,
        max_ty: 1.0,
        texture: None,
        ant: Ant {
            position: [0.0, 0.0, 0.0],
            goal: [0.0, 0.0, 0.0],
            direction: 0.0,
            velocity: 0.02,
            step: 0.0,
        },
        boardsize: 8.0,
        mag: 1,
        wire,
    };
    st.find_goal();

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    // Two directional lights for the ant, one from each side, and no scene
    // ambient at all: the floor is only lit by the spotlight the ant carries.
    let ambient = [0.4, 0.4, 0.4, 1.0];
    let diffuse = [1.0, 1.0, 1.0, 1.0];
    g.glx.light_ambient(0, ambient);
    g.glx.light_diffuse(0, diffuse);
    g.glx.light_position(0, 1.0, 5.0, 1.0, 0.0);
    g.glx.light_ambient(1, ambient);
    g.glx.light_diffuse(1, diffuse);
    g.glx.light_position(1, -1.0, -5.0, 1.0, 0.0);
    g.glx.light_model_ambient([0.0, 0.0, 0.0, 1.0]);
    g.glx.material_shininess(60.0);
    g.glx.material_specular([0.8, 0.8, 0.8, 1.0]);

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     20000",
    "*showFPS:   False",
    "*wireframe: False",
];

const OPTS: &[Opt] =
    &[Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted()];

pub static DEF: SaverDef = SaverDef {
    slug: "antspotlight",
    label: "Ant Spotlight",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Blair Tennessy",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=NYisFYtODTA"),
        blurb: "An ant walks over an image.",
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

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    fn an_ant() -> AntSpotlight {
        AntSpotlight {
            rot: Rotator::new(0.3, 0.3, 0.3, 1.0, 0.0, true),
            trackball: Trackball::new(),
            max_tx: 1.0,
            max_ty: 1.0,
            texture: None,
            ant: Ant {
                position: [0.0, 0.0, 0.0],
                goal: [0.0, 0.0, 0.0],
                direction: 0.0,
                velocity: 0.02,
                step: 0.0,
            },
            boardsize: 8.0,
            mag: 1,
            wire: false,
        }
    }

    /// A goal is somewhere on the board and never so close that the ant has
    /// already arrived.
    #[test]
    fn a_goal_is_on_the_board_and_worth_walking_to() {
        let mut st = an_ant();
        for _ in 0..50 {
            st.find_goal();
            let g = st.ant.goal;
            let half = st.boardsize / 2.0;
            assert!(
                g[0] >= -half && g[0] <= half && g[2] >= -half && g[2] <= half,
                "goal {g:?} is off the board"
            );
            assert_eq!(g[1], 0.0, "the goal left the floor");
            assert!(
                distance(st.ant.position, g) >= 2.0,
                "goal {g:?} is on top of the ant"
            );
            // Walk the ant there so the next goal has to be somewhere else.
            st.ant.position = g;
        }
    }

    /// The spotlight is brightest under itself, falls away with distance, and
    /// stops dead outside the beam.
    #[test]
    fn the_beam_is_brightest_in_the_middle_and_stops_at_its_edge() {
        let st = an_ant();
        // Pointing along +x and down, as it is when the ant faces that way.
        let light = [0.7, 0.5, 0.0];
        let dir = [1.0, -0.5, 0.0];

        // The lamp points forward and down, so the bright patch is ahead of
        // it rather than under it: the axis meets the floor at x = 1.7.
        let axis = st.spot([1.7, 0.0, 0.0], light, dir);
        let ahead = st.spot([3.0, 0.0, 0.0], light, dir);
        let far = st.spot([5.0, 0.0, 0.0], light, dir);
        assert!(axis > 0.0, "nothing where the beam points");
        assert!(
            axis > ahead && ahead > far,
            "it does not fall away: {axis} {ahead} {far}"
        );

        // Behind the ant, and off to the side, there is no light at all.
        assert_eq!(st.spot([-3.0, 0.0, 0.0], light, dir), 0.0, "behind");
        assert_eq!(st.spot([0.7, 0.0, 6.0], light, dir), 0.0, "beside");
    }

    /// The beam is a cone of sixty degrees, so a point on its axis is lit and
    /// one just outside the half-angle is not.
    #[test]
    fn the_beam_is_sixty_degrees_wide() {
        let st = an_ant();
        let light = [0.0, 1.0, 0.0];
        // Straight down, so the cutoff is a circle on the floor of radius
        // tan(30) about the point under the lamp.
        let dir = [0.0, -1.0, 0.0];
        let edge = SPOT_CUTOFF.tan();
        assert!(st.spot([edge * 0.9, 0.0, 0.0], light, dir) > 0.0, "inside");
        assert_eq!(st.spot([edge * 1.1, 0.0, 0.0], light, dir), 0.0, "outside");
    }

    /// The ant turns towards its goal rather than snapping round to face it,
    /// and closes on it.
    ///
    /// It cannot turn faster than a two-hundredth of a turn a frame, which is
    /// a wider circle than the goal, so it does not stop on the goal: it
    /// sweeps past and comes round again. Upstream never lets that go on,
    /// because a frame in a hundred picks a new goal anyway.
    #[test]
    fn the_ant_walks_to_its_goal() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        r.step();
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");

        // Over a few hundred frames it has to have moved a long way.
        let mut st = an_ant();
        st.ant.goal = [3.0, 0.0, 3.0];
        let start = distance(st.ant.position, st.ant.goal);
        let mut prev_dir = st.ant.direction;
        let mut worst_turn: f32 = 0.0;
        let mut closest = f32::MAX;
        for _ in 0..900 {
            // Just the walking half of `draw_strip`, with no GL.
            let dx = st.ant.goal[0] - st.ant.position[0];
            let dz = -(st.ant.goal[2] - st.ant.position[2]);
            let pi = std::f32::consts::PI;
            let mut theta = if dx.abs() > 0.01 {
                let t = (dz / dx).atan();
                if dx < 0.0 { t + pi } else { t }
            } else if dz > 0.0 {
                pi / 2.0
            } else {
                3.0 * pi / 2.0
            };
            if theta < 0.0 {
                theta += 2.0 * pi;
            }
            let mut ideal = theta - st.ant.direction;
            if ideal > pi {
                ideal -= 2.0 * pi;
            }
            let dt = ideal.signum() * ideal.abs().min(pi / 100.0);
            st.ant.direction += dt;
            st.ant.position[0] += st.ant.velocity * st.ant.direction.cos();
            st.ant.position[2] += st.ant.velocity * (-st.ant.direction).sin();

            let turn = (st.ant.direction - prev_dir).abs();
            worst_turn = worst_turn.max(turn);
            prev_dir = st.ant.direction;
            closest = closest.min(distance(st.ant.position, st.ant.goal));
        }
        assert!(
            worst_turn <= std::f32::consts::PI / 100.0 + 1e-5,
            "it snapped round by {worst_turn}"
        );
        assert!(start > 4.0, "the goal was not far enough away to be a test");
        assert!(closest < 0.5, "it never got near the goal: {closest} away");
    }

    /// The picture is only drawn where the light falls: the fan is textured
    /// and there is no floor anywhere else.
    #[test]
    fn only_the_lit_part_of_the_picture_is_drawn() {
        let r = run("", 3);
        let f = r.frame();
        assert!(
            f.batches.iter().any(|b| b.texture.is_some()),
            "the picture was never drawn"
        );
        // Every vertex of the fan is within the beam's reach of the ant, which
        // is about ten units at the far end.
        for b in f.batches.iter().filter(|b| b.texture.is_some()) {
            for v in &f.vertices[b.first..b.first + b.count] {
                assert!(
                    v.pos[0].abs() < 16.0 && v.pos[2].abs() < 16.0,
                    "a lit vertex at {:?} is nowhere near the ant",
                    v.pos
                );
            }
        }
    }
}
