//! Port of `hacks/glx/hopffibration.c`.
//!
//! ```text
//! hopffibration --- Displays the Hopf fibration of the 4D hypersphere S³.
//!
//! Copyright (c) 2025-2026 Carsten Steger <carsten@mirsanmir.org>.
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//! ```
//!
//! The Hopf map sends the 3-sphere onto the ordinary sphere, and every point
//! of the ordinary sphere is the image of a whole great circle. The grey ball
//! in the corner is the ordinary sphere, the coloured dots on it are the
//! points being asked about, and the tangles of coloured tube are the great
//! circles those points came from, projected down from four dimensions into
//! three. A dot and its fiber are the same colour.
//!
//! Any two fibers are linked exactly once. A circle of points on the ball
//! gives a torus of fibers, a wavy closed curve gives a Hopf torus, an arc
//! gives a Hopf band, and the one point at the north pole gives a fiber that
//! passes through infinity and shows up as a straight rod. The animations
//! walk between eight configurations of points to show all of it off.
//!
//! The maths, the choreography and the sphere are all in [`crate::runtime::hopf`];
//! this is the part that draws.
//!
//! Three things upstream has that are not here. It renders a second pass from
//! the light to get shadows, which lives entirely in its GLSL path behind a
//! depth texture, so the shadows knob would toggle nothing and is gone. Its
//! anti-aliasing knob switches a multisampled framebuffer object on and off,
//! which in a browser is the canvas context's business rather than a saver's,
//! so that knob is gone too. And it calls `gltrackball_rotate` once before
//! loading the identity over the top of it, which does nothing.
//!
//! The detail knob does stay, and its default here is coarse rather than
//! medium. This is one of the heavy savers: the fibers are tubes swept along
//! curves subdivided until they are smooth, and the geometry is rebuilt every
//! frame because it moves every frame, which is what upstream does too
//! (`GL_STREAM_DRAW`). At coarse the median animation comes to 256k vertices
//! a frame and three quarters of them to 343k, which is what `beats` draws;
//! the four heaviest reach 767k. At medium those numbers double.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::hopf::{
    Animations, BasePoint, GEN_TORUS, HopfCircle, Icosphere, Mat3, axis_angle_to_quat, ease,
    gen_spiral_base, gen_torus_base, look_at_rotmat, mult_rotmat, norm, quat_to_rotmat, rotateall,
    rotatex, rotatey, rotatez,
};
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
};
use std::f32::consts::PI;

/// The animation tables, converted from upstream's half a megabyte of C by
/// `web/homepage/gen-hopfanimations.nu`.
const ANIMATIONS: &str = include_str!("../../data/hopfanimations.txt");

const DISP_PERSPECTIVE: i32 = 0;
const DISP_ORTHOGRAPHIC: i32 = 1;

const TUBE_RADIUS: f32 = 0.01;
const BASE_POINT_RADIUS: f32 = 0.01;
const BASE_SPHERE_RADIUS: f32 = 0.2;

/// At most this many objects move together in one phase.
const MAX_ANIM_GEOM: usize = 10;

/// The camera position.
const EYE_POS: [f32; 3] = [0.0, 0.0, 3.0];

/// Where the base sphere sits relative to the ball the fibers are in.
const BASE_OFFSET: [f32; 3] = [0.9, -0.9, 0.25];

/// The state of one of the objects being animated: the parameters of the
/// curve its base points lie on, as they stand this frame.
#[derive(Clone, Copy, Default)]
struct Geom {
    generator: i32,
    p: f32,
    q: f32,
    r: f32,
    offset: f32,
    sector: f32,
    n: i32,
    num: i32,
    rotate: bool,
    quat_base: [f32; 4],
}

struct Hopf {
    trackball: Trackball,
    anims: Animations,

    base_space: bool,
    projection: i32,
    num_tube: usize,
    max_circle_dist: f32,

    /// The 3D rotation of the whole projection.
    alpha: f32,
    beta: f32,
    delta: f32,
    /// The rotation of the base points on the base sphere.
    zeta: f32,
    eta: f32,
    theta: f32,

    rot_axis_base: [f32; 3],
    quat_base: [f32; 4],
    rot_axis_space: [f32; 3],
    quat_space: [f32; 4],
    angle_space_start: f32,
    angle_space_end: f32,

    /// Which of the eight configurations the base points are in.
    anim_state: usize,
    anim_remain_in_state: i32,
    /// The animation being run, as an index into `anims.phases`.
    anim_phases: usize,
    anim_phase: usize,
    anim_phase_num: usize,
    /// The phase being run, as an index into `anims.multi`.
    anim: usize,
    anim_step: i32,
    anim_easing_fct_rot_rnd: i32,
    anim_easing_fct_rot_space: i32,
    anim_rotate_rnd: bool,
    anim_rotate_space: bool,
    anim_geom: [Geom; MAX_ANIM_GEOM],

    base_points: Vec<BasePoint>,
    sphere_base: Icosphere,
    sphere_base_point: Icosphere,
}

/// `gen_random_rot_axis`: a direction spread evenly over the sphere.
fn random_rot_axis() -> [f32; 3] {
    let t = frand(2.0 * PI as f64) as f32;
    let p = (frand(2.0) as f32 - 1.0).acos();
    [p.sin() * t.cos(), p.sin() * t.sin(), p.cos()]
}

impl Hopf {
    /// `set_animation_quats`.
    fn set_animation_quats(&mut self, t: f32) {
        if self.anim_rotate_rnd {
            let te = 2.0 * PI * ease(t, self.anim_easing_fct_rot_rnd);
            self.quat_base = axis_angle_to_quat(self.rot_axis_base, te);
        }
        if self.anim_rotate_space {
            let te = ease(t, self.anim_easing_fct_rot_space);
            let te = (1.0 - te) * self.angle_space_start + te * self.angle_space_end;
            self.quat_space = axis_angle_to_quat(self.rot_axis_space, te);
        }
    }

    /// `set_animation_geometry`: where object `i` has got to at time `t`.
    fn set_animation_geometry(&mut self, t: f32, i: usize) {
        if i >= MAX_ANIM_GEOM {
            return;
        }
        let s = self.anims.multi[self.anim].so[i];
        let lerp = |a: f32, b: f32, e: i32| {
            let te = ease(t, e);
            (1.0 - te) * a + te * b
        };
        let g = &mut self.anim_geom[i];
        g.num = s.num;
        g.p = lerp(s.p_start, s.p_end, s.easing_p);
        g.q = lerp(s.q_start, s.q_end, s.easing_q);
        g.r = lerp(s.r_start, s.r_end, s.easing_r);
        g.offset = lerp(s.offset_start, s.offset_end, s.easing_offset);
        g.sector = lerp(s.sector_start, s.sector_end, s.easing_sector);
        if g.rotate {
            let angle = lerp(s.angle_start, s.angle_end, s.easing_rotate);
            g.quat_base = axis_angle_to_quat(s.rot_axis_base, angle);
        }
    }

    /// `init_next_anim_phase`.
    fn init_next_anim_phase(&mut self, phase: usize) {
        self.anim = self.anims.phases[self.anim_phases][phase];
        self.anim_step = 0;

        let mo = &self.anims.multi[self.anim];
        let n = mo.so.len().min(MAX_ANIM_GEOM);
        for i in 0..n {
            let s = mo.so[i];
            self.anim_geom[i] = Geom {
                generator: s.generator,
                n: s.n,
                rotate: norm(s.rot_axis_base) > 0.0,
                quat_base: [1.0, 0.0, 0.0, 0.0],
                ..Geom::default()
            };
        }

        let mo = &self.anims.multi[self.anim];
        self.anim_rotate_rnd = if mo.rotate_prob == 0.0 {
            false
        } else if mo.rotate_prob == 1.0 {
            true
        } else {
            (frand(1.0) as f32) < mo.rotate_prob
        };
        self.anim_easing_fct_rot_rnd = mo.easing_rot_rnd;
        self.anim_easing_fct_rot_space = mo.easing_rot_space;
        self.rot_axis_space = mo.rot_axis_space;
        self.anim_rotate_space = norm(mo.rot_axis_space) > 0.0;
        self.angle_space_start = mo.angle_start;
        self.angle_space_end = mo.angle_end;

        self.rot_axis_base = random_rot_axis();

        self.set_animation_quats(0.0);
        for i in 0..n {
            self.set_animation_geometry(0.0, i);
        }
    }

    /// `set_next_anim`: pick the next configuration to head for, and an
    /// animation that gets there.
    fn set_next_anim(&mut self) {
        let states = crate::runtime::hopf::NUM_ANIM_STATES;
        let next = if self.anim_remain_in_state <= 0 {
            frand(states as f64).floor() as usize
        } else {
            self.anim_state
        };

        if next == self.anim_state {
            self.anim_remain_in_state -= 1;
        } else {
            // Stay in the new configuration for a while, in proportion to how
            // many animations it has that stay put.
            self.anim_remain_in_state =
                (self.anims.anims[self.anims.table[next][next]].len() as i32 + 2) / 3;
        }

        let set = self.anims.table[self.anim_state][next];
        self.anim_state = next;
        let choices = self.anims.anims[set].len();

        let idx = if choices > 1 {
            // Never the same animation twice running.
            let mut idx = frand(choices as f64).floor() as usize;
            let mut tries = 0;
            while self.anims.anims[set][idx] == self.anim_phases && tries < 100 {
                idx = frand(choices as f64).floor() as usize;
                tries += 1;
            }
            idx
        } else {
            0
        };

        self.anim_phases = self.anims.anims[set][idx];
        self.anim_phase_num = self.anims.phases[self.anim_phases].len();
        self.anim_phase = 0;
        self.init_next_anim_phase(0);
    }

    /// Advance the animation and work out where the base points are now.
    fn step_animation(&mut self, running: bool) {
        if running {
            self.anim_step += 1;
            let steps = self.anims.multi[self.anim].num_steps.max(1);
            let t = self.anim_step as f32 / steps as f32;

            self.set_animation_quats(t);
            for i in 0..self.anims.multi[self.anim].so.len().min(MAX_ANIM_GEOM) {
                self.set_animation_geometry(t, i);
            }

            if self.anim_step >= steps {
                self.anim_step = 0;
                if self.anim_phase < self.anim_phase_num - 1 {
                    self.anim_phase += 1;
                    let p = self.anim_phase;
                    self.init_next_anim_phase(p);
                } else {
                    self.set_next_anim();
                }
            }
        }

        self.base_points.clear();
        for i in 0..self.anims.multi[self.anim].so.len().min(MAX_ANIM_GEOM) {
            let g = self.anim_geom[i];
            if g.generator == GEN_TORUS {
                gen_torus_base(
                    &mut self.base_points,
                    g.p,
                    g.q,
                    g.r,
                    g.n,
                    g.offset,
                    g.sector,
                    g.num,
                    g.rotate,
                    g.quat_base,
                );
            } else {
                gen_spiral_base(
                    &mut self.base_points,
                    g.p,
                    g.q,
                    g.r,
                    g.offset,
                    g.sector,
                    g.num,
                    g.rotate,
                    g.quat_base,
                );
            }
        }
    }

    /// The rotation that takes a base point where the animation wants it.
    fn base_rotation(&self) -> Mat3 {
        let mat1 = rotateall(self.zeta, self.eta, self.theta);
        let mat2 = quat_to_rotmat(self.quat_base);
        mult_rotmat(&mat2, &mat1)
    }

    /// The matrix the base sphere's triangles are sorted against: what the
    /// camera sees of it after the fixed rotation.
    fn sort_matrix(&self) -> Mat3 {
        let mut mat = look_at_rotmat(EYE_POS, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        rotatex(&mut mat, self.alpha);
        rotatey(&mut mat, self.beta);
        rotatez(&mut mat, self.delta);
        mat
    }

    /// `draw_hopf_circle_ff`: one fiber, as a closed tube swept along it.
    ///
    /// Upstream draws a triangle strip per segment, which the recorder would
    /// take as a draw call per segment; the same triangles wound the same way
    /// go into one block here, so a whole animation is one call.
    fn draw_fiber(&self, g: &mut Gl, hb: BasePoint) {
        let hc = HopfCircle::new(hb);
        let pts = hc.points(self.max_circle_dist);
        let nt = self.num_tube;
        let step = 2.0 * PI / nt as f32;

        // A point on the ring around `pts[l]` at angle `j`, and its normal,
        // which for a tube is the direction it lies in from the axis.
        let ring = |l: usize, j: usize| {
            let phi = j as f32 * step;
            let (sp, cp) = phi.sin_cos();
            let p = &pts[l];
            let mut t = [0.0f32; 3];
            let mut q = [0.0f32; 3];
            for m in 0..3 {
                t[m] = cp * p.n[m] + sp * p.b[m];
                q[m] = p.p[m] + TUBE_RADIUS * t[m];
            }
            (q, t)
        };

        for i in 0..pts.len() - 1 {
            for j in (1..=nt).rev() {
                let a0 = ring(i, j);
                let b0 = ring(i + 1, j);
                let a1 = ring(i, j - 1);
                let b1 = ring(i + 1, j - 1);
                for v in [a0, b0, a1, a1, b0, b1] {
                    g.glx.normal3f(v.1[0], v.1[1], v.1[2]);
                    g.glx.vertex3f(v.0[0], v.0[1], v.0[2]);
                }
            }
        }

        if !hc.is_line() {
            return;
        }

        // The fiber through the north pole is a rod rather than a ring, so it
        // needs a cap at each end. There are exactly two points in this case.
        for (i, sgn) in [(0usize, 1.0f32), (1, -1.0)] {
            let p = &pts[i];
            let cap = |j: usize| {
                let phi = sgn * j as f32 * step;
                let (sp, cp) = phi.sin_cos();
                let mut q = p.p;
                for (m, out) in q.iter_mut().enumerate() {
                    *out += TUBE_RADIUS * (cp * p.n[m] + sp * p.b[m]);
                }
                q
            };
            for j in (1..=nt).rev() {
                let (r0, r1) = (cap(j), cap(j - 1));
                for v in [p.p, r0, r1] {
                    g.glx.normal3f(0.0, 0.0, sgn);
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
            }
        }
    }

    /// `draw_icosphere_ff`, with the triangles already sorted.
    fn draw_icosphere(g: &mut Gl, sphere: &Icosphere) {
        g.glx.begin(Shape::Triangles);
        for t in &sphere.stri {
            for &l in t {
                let n = sphere.norm[l];
                let v = sphere.vert[l];
                g.glx.normal3f(n[0], n[1], n[2]);
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
        }
        g.glx.end();
    }

    /// Load the modelview that puts the base sphere in its corner.
    fn base_modelview(&self, g: &mut Gl) {
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.look_at(EYE_POS, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx
            .translate(BASE_OFFSET[0], BASE_OFFSET[1], BASE_OFFSET[2]);
        g.glx.rotate(self.alpha, 1.0, 0.0, 0.0);
        g.glx.rotate(self.beta, 0.0, 1.0, 0.0);
        g.glx.rotate(self.delta, 0.0, 0.0, 1.0);
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let details = g.res.string("details").to_string();
    let (num_tube, base_sphere_s, base_point_s, max_circle_dist) = match details.as_str() {
        "fine" => (16, 20, 3, 0.0003),
        "medium" => (12, 14, 2, 0.0005),
        _ => (8, 12, 2, 0.0010),
    };
    let projection = if g
        .res
        .string("projection")
        .eq_ignore_ascii_case("orthographic")
    {
        DISP_ORTHOGRAPHIC
    } else {
        DISP_PERSPECTIVE
    };

    let mut this = Hopf {
        trackball: Trackball::new(),
        anims: Animations::parse(ANIMATIONS),
        base_space: g.res.bool("basespace"),
        projection,
        num_tube,
        max_circle_dist,
        alpha: 290.0,
        beta: 0.0,
        delta: 270.0,
        zeta: 0.0,
        eta: 0.0,
        theta: 0.0,
        rot_axis_base: [1.0, 0.0, 0.0],
        quat_base: [1.0, 0.0, 0.0, 0.0],
        rot_axis_space: [1.0, 0.0, 0.0],
        quat_space: [1.0, 0.0, 0.0, 0.0],
        angle_space_start: 0.0,
        angle_space_end: 2.0 * PI,
        anim_state: 0,
        anim_remain_in_state: 1,
        anim_phases: usize::MAX,
        anim_phase: 0,
        anim_phase_num: 0,
        anim: 0,
        anim_step: 0,
        anim_easing_fct_rot_rnd: 0,
        anim_easing_fct_rot_space: 0,
        anim_rotate_rnd: false,
        anim_rotate_space: false,
        anim_geom: [Geom::default(); MAX_ANIM_GEOM],
        base_points: Vec::new(),
        sphere_base: Icosphere::new(base_sphere_s, BASE_SPHERE_RADIUS),
        sphere_base_point: Icosphere::new(base_point_s, BASE_POINT_RADIUS),
    };

    this.anim_state = frand(crate::runtime::hopf::NUM_ANIM_STATES as f64).floor() as usize;
    this.set_next_anim();
    Box::new(this)
}

impl Hack3d for Hopf {
    fn reshape(&mut self, _g: &mut Gl, _width: i32, _height: i32) {
        // Upstream only records the size here and sets both matrices when it
        // draws, because the projection depends on which of the two the knob
        // asks for.
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let down = self.trackball.button_down();
        self.step_animation(!down);

        let aspect = g.width() as f32 / g.height() as f32;
        g.glx.viewport(0, 0, g.width(), g.height());
        g.glx.clear();
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        if self.projection == DISP_ORTHOGRAPHIC {
            if aspect >= 1.0 {
                g.glx
                    .ortho(-1.2 * aspect, 1.2 * aspect, -1.2, 1.2, 0.1, 10.0);
            } else {
                g.glx
                    .ortho(-1.2, 1.2, -1.2 / aspect, 1.2 / aspect, 0.1, 10.0);
            }
        } else if aspect >= 1.0 {
            g.glx.perspective(45.0, aspect, 0.1, 10.0);
        } else {
            let fovy = 360.0 / PI * ((45.0 * PI / 360.0).tan() / aspect).atan();
            g.glx.perspective(fovy, aspect, 0.1, 10.0);
        }

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.depth_test(true);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        // Set while the modelview is still the identity, so the light stays
        // where the camera is rather than turning with the fibers.
        g.glx.light_position(0, 0.7, 0.7, 1.0, 0.0);
        g.glx.light_ambient(0, [0.3, 0.3, 0.3, 1.0]);
        g.glx.light_diffuse(0, [0.7, 0.7, 0.7, 1.0]);
        g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(50.0);
        g.glx.depth_mask(true);
        g.glx.blend(Blend::Off);
        g.glx.cull_face(true);
        g.glx.front_face_cw(false);
        // The fiber colours ride on the vertices rather than on the material,
        // which is the same lighting and keeps the whole tangle in one call.
        g.glx.color_material(true);

        g.glx.look_at(EYE_POS, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        let ms = quat_to_rotmat(self.quat_space);
        g.glx.mult_matrix(mat3_to_mat4(&ms));
        g.glx.rotate(self.alpha, 1.0, 0.0, 0.0);
        g.glx.rotate(self.beta, 0.0, 1.0, 0.0);
        g.glx.rotate(self.delta, 0.0, 0.0, 1.0);

        let mat = self.base_rotation();
        let points: Vec<BasePoint> = self.base_points.iter().map(|b| b.rotated(&mat)).collect();

        g.glx.begin(Shape::Triangles);
        for &ht in &points {
            let c = ht.color();
            g.glx.color3f(c[0], c[1], c[2]);
            self.draw_fiber(g, ht);
        }
        g.glx.end();

        if !self.base_space {
            return g.res.int("delay") as u32;
        }

        // The dots that say which points of the base sphere the fibers came
        // from, each one out on the sphere's surface.
        self.sphere_base_point.sort(&self.sort_matrix());
        for &ht in &points {
            self.base_modelview(g);
            g.glx.translate(
                ht.a * BASE_SPHERE_RADIUS,
                ht.b * BASE_SPHERE_RADIUS,
                ht.c * BASE_SPHERE_RADIUS,
            );
            let c = ht.color();
            g.glx.color3f(c[0], c[1], c[2]);
            Self::draw_icosphere(g, &self.sphere_base_point);
        }

        // And the base sphere itself, half transparent, which is why its
        // triangles were sorted back to front and why nothing is culled.
        g.glx.depth_mask(false);
        g.glx.blend(Blend::Alpha);
        g.glx.cull_face(false);
        self.base_modelview(g);
        self.sphere_base.sort(&self.sort_matrix());
        g.glx.color_material(false);
        g.glx.material_ambient_diffuse([0.7, 0.7, 0.7, 0.6]);
        Self::draw_icosphere(g, &self.sphere_base);
        g.glx.depth_mask(true);
        g.glx.blend(Blend::Off);

        g.res.int("delay") as u32
    }
}

/// A rotation as `glMultMatrixf` wants it: column major, and a row and a
/// column of nothing round the outside.
fn mat3_to_mat4(m: &Mat3) -> crate::runtime::gl::Mat4 {
    let mut r = [0.0f32; 16];
    for i in 0..3 {
        for j in 0..3 {
            r[j * 4 + i] = m[i][j];
        }
    }
    r[15] = 1.0;
    crate::runtime::gl::Mat4(r)
}

const DEFAULTS: &[&str] = &[
    "*delay:       20000",
    "*showFPS:     False",
    "*details:     coarse",
    "*projection:  perspective",
    "*basespace:   True",
];

const DETAILS: &[SelectItem] = &[
    SelectItem {
        value: "coarse",
        label: "Coarse",
    },
    SelectItem {
        value: "medium",
        label: "Medium",
    },
    SelectItem {
        value: "fine",
        label: "Fine",
    },
];

const PROJECTIONS: &[SelectItem] = &[
    SelectItem {
        value: "perspective",
        label: "Perspective",
    },
    SelectItem {
        value: "orthographic",
        label: "Orthographic",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::select("details", "Detail", DETAILS, "coarse"),
    Opt::select("projection", "Projection", PROJECTIONS, "perspective"),
    Opt::boolean("basespace", "Display the base space", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hopffibration",
    label: "Hopf Fibration",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Steger",
        year: "2025",
        video: Some("https://www.youtube.com/watch?v=75TQ0CjxRfY"),
        blurb: "The Hopf fibration of the 4d hypersphere S³. Each point of the \
                grey sphere is the image of a whole great circle of the \
                hypersphere, and the tangles of coloured tube are those \
                circles projected down into three dimensions. Any two of them \
                are linked exactly once.",
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

    #[test]
    fn the_whole_tangle_of_fibers_is_one_draw_call() {
        // The colours ride on the vertices, so however many fibers an
        // animation asks for they all go down together. With the base space
        // off there is nothing else in the frame at all.
        let mut r = start(StartArgs::new(640, 480, "basespace=false", 20260812));
        for _ in 0..40 {
            r.step();
            let f = r.frame();
            assert_eq!(f.batches.len(), 1, "{} calls", f.batches.len());
            assert!(f.batches[0].count > 100);
        }
    }

    #[test]
    fn the_base_space_is_a_sphere_of_dots_and_a_ball() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        r.step();
        let f = r.frame();
        // The fibers, one call per dot, and the ball last.
        assert!(f.batches.len() > 2);
        let ball = f.batches.last().expect("something is drawn");
        assert!(!ball.cull_face, "the ball is seen from inside as well");
        assert_eq!(ball.blend, Blend::Alpha, "the ball is half transparent");
        assert!(!ball.depth_mask, "the ball must not hide what is behind it");
        // Everything before it is opaque.
        for b in &f.batches[..f.batches.len() - 1] {
            assert_eq!(b.blend, Blend::Off);
        }
    }

    #[test]
    fn a_fiber_is_drawn_as_a_tube_around_its_curve() {
        // Every vertex is a tube radius away from the curve, so the whole
        // tangle stays inside the ball the projection squeezes space into.
        let mut r = start(StartArgs::new(640, 480, "basespace=false", 20260812));
        r.step();
        let f = r.frame();
        for b in &f.batches {
            for v in &f.vertices[b.first..b.first + b.count] {
                let d = (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt();
                assert!(d < 1.0 + TUBE_RADIUS + 1e-4, "a fiber reached {d}");
                let n = (v.normal[0] * v.normal[0]
                    + v.normal[1] * v.normal[1]
                    + v.normal[2] * v.normal[2])
                    .sqrt();
                assert!((n - 1.0).abs() < 1e-3, "a tube normal is {n} long");
            }
        }
    }

    #[test]
    fn it_walks_through_the_configurations_without_getting_stuck() {
        // Each animation runs to its end, hands over to the next phase, and
        // the last phase picks a new animation; a state that never left would
        // freeze the picture.
        let mut r = start(StartArgs::new(640, 480, "basespace=false", 20260812));
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..600 {
            r.step();
            seen.insert(r.frame().vertices.len());
            assert!(!r.frame().batches.is_empty());
        }
        assert!(seen.len() > 100, "only {} shapes in 600 frames", seen.len());
    }

    #[test]
    fn the_coarse_setting_is_the_default_and_is_the_lightest() {
        let count = |q: &str| {
            let mut r = start(StartArgs::new(640, 480, q, 20260812));
            r.step();
            r.frame().vertices.len()
        };
        let coarse = count("basespace=false&details=coarse");
        let medium = count("basespace=false&details=medium");
        let fine = count("basespace=false&details=fine");
        assert!(coarse < medium && medium < fine, "{coarse} {medium} {fine}");
        assert_eq!(coarse, count("basespace=false"), "coarse is the default");
    }
}
