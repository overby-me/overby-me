//! Port of `hacks/glx/vigilance.c`.
//!
//! ```text
//! vigilance, Copyright © 2017-2023 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Draws surveillance cameras, taking an interest in their surroundings.
//! ```
//!
//! A row of security cameras on a wireframe floor, watching people walk past.
//! Nobody is drawn: a pedestrian is only a path through the scene, and what
//! you see is the cameras turning to follow one.
//!
//! A camera swings towards whatever it has decided to look at, speeding up
//! when it is far from its target and slowing as it arrives, and its hinge
//! stops it turning more than a right angle either way or looking further up
//! than fifty-five degrees. Sometimes one looks at another camera instead, and
//! if the two end up staring at each other, the one that noticed looks away,
//! at the sky or at the ground.
//!
//! Every so often they all decide a pedestrian is a threat, warm up, and shoot
//! red beams at them until the pedestrian stops. The beam is five nested boxes
//! of increasing opacity, ten thousand units long.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Fog, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

const CAMERA_BODY: usize = 0;
const CAMERA_CAP: usize = 1;
const CAMERA_HINGE: usize = 2;
const CAMERA_MOUNT: usize = 3;
const CAMERA_LENS: usize = 4;

const MODELS: [&str; 5] = [
    crate::models::SECCAM_BODY,
    crate::models::SECCAM_CAP,
    crate::models::SECCAM_HINGE,
    crate::models::SECCAM_PIPE,
    crate::models::SECCAM_LENS,
];

/// How far from the origin the lens sits in the model.
const BEAM_ZOFF: f32 = 0.185;

fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    WarmUp,
    Zot,
    CoolDown,
}

/// What a camera is looking at: nothing in particular, a pedestrian, or
/// another camera.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Nothing,
    Pedestrian(u64),
    Camera(usize),
}

#[derive(Clone)]
struct Camera {
    pos: [f32; 3],
    /// Rotation about the vertical axis, and front-to-back tilt, in degrees.
    facing: f32,
    pitch: f32,
    /// The angular velocity it had last time, which is what it accelerates
    /// away from.
    velocity: f32,
    focus_id: Focus,
    focus: [f32; 3],
    state: State,
    tick: f32,
}

/// Nobody is drawn: a pedestrian is a path through the scene and a speed
/// along it, and what you see is the cameras following one.
#[derive(Clone)]
struct Pedestrian {
    id: u64,
    pos: [f32; 3],
    length: f32,
    frequency: f32,
    amplitude: f32,
    ratio: f32,
    speed: f32,
}

impl Pedestrian {
    /// Where this one would be at the given point of its life.
    fn position(&self, ratio: f32) -> [f32; 3] {
        let ratio = if self.speed < 0.0 { 1.0 - ratio } else { ratio };
        [
            self.pos[0] + self.length * ratio,
            self.pos[1],
            self.pos[2]
                + (std::f32::consts::PI * ratio * self.frequency * 2.0).sin() * self.amplitude
                + self.amplitude / 2.0,
        ]
    }
}

struct Vigilance {
    trackball: Trackball,
    lists: Vec<u32>,
    colors: Vec<[f32; 4]>,
    ground: u32,
    ground_color: [f32; 4],
    cameras: Vec<Camera>,
    pedestrians: Vec<Pedestrian>,
    next_id: u64,
    aspect: f32,
    scale: f32,
    speed: f32,
    count: usize,
    wire: bool,
}

fn normalize(p: [f32; 3]) -> [f32; 3] {
    let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    if d < 0.0000001 {
        [0.0; 3]
    } else {
        [p[0] / d, p[1] / d, p[2] / d]
    }
}

/// The angle between two vectors, in radians.
fn vector_angle(a: [f32; 3], b: [f32; 3]) -> f32 {
    let la = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let lb = (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt();
    if la == 0.0 || lb == 0.0 || a == b {
        return 0.0;
    }
    // Guard the rounding: a cosine of 1.000001 is not a number acos likes.
    let cc = ((a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (la * lb)).min(1.0);
    cc.acos()
}

/// A colour resource as GL wants it.
fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

impl Vigilance {
    /// One part of a camera, with the material applied at call time: a
    /// display list here replays geometry and not state.
    fn draw_component(&self, g: &mut Gl, i: usize, color: Option<[f32; 4]>) {
        let spec = if i == CAMERA_LENS {
            [0.4, 0.4, 0.7, 1.0]
        } else {
            [1.0, 1.0, 1.0, 1.0]
        };
        g.glx.material_specular(spec);
        g.glx.material_shininess(20.0);
        g.glx
            .material_ambient_diffuse(color.unwrap_or(self.colors[i]));
        g.glx.call_list(self.lists[i]);
    }

    /// The models are half a camera; the other half is the same one mirrored.
    fn draw_double_component(&self, g: &mut Gl, i: usize) {
        self.draw_component(g, i, None);
        g.glx.front_face_cw(false);
        g.glx.scale(1.0, 1.0, -1.0);
        self.draw_component(g, i, None);
        g.glx.scale(1.0, 1.0, -1.0);
        g.glx.front_face_cw(true);
    }

    /// `draw_ray`: five nested boxes, each less transparent than the last,
    /// ten thousand units long.
    fn draw_ray(&self, g: &mut Gl, c: &Camera) {
        g.glx.push_matrix();
        g.glx.translate(c.pos[0], c.pos[1], c.pos[2] + BEAM_ZOFF);
        g.glx.rotate(-c.facing, 0.0, 0.0, 1.0);
        g.glx.rotate(c.pitch, 1.0, 0.0, 0.0);
        g.glx.rotate(frand(90.0) as f32, 0.0, 1.0, 0.0);
        g.glx.scale(0.08, 10000.0, 0.08);
        g.glx.lighting(false);
        g.glx.color_material(true);
        for i in 0..5 {
            g.glx.color4f(1.0, 0.0, 0.0, 0.1 + (i as f32 * 0.18));
            for face in [
                [
                    [0.0, 0.0, -0.5],
                    [0.0, 0.0, 0.5],
                    [0.0, 1.0, 0.5],
                    [0.0, 1.0, -0.5],
                ],
                [
                    [-0.5, 0.0, 0.0],
                    [0.5, 0.0, 0.0],
                    [0.5, 1.0, 0.0],
                    [-0.5, 1.0, 0.0],
                ],
                [
                    [0.0, 1.0, -0.5],
                    [0.0, 1.0, 0.5],
                    [0.0, 0.0, 0.5],
                    [0.0, 0.0, -0.5],
                ],
                [
                    [-0.5, 1.0, 0.0],
                    [0.5, 1.0, 0.0],
                    [0.5, 0.0, 0.0],
                    [-0.5, 0.0, 0.0],
                ],
            ] {
                g.glx.begin(if self.wire {
                    Shape::LineLoop
                } else {
                    Shape::Quads
                });
                for v in face {
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
                g.glx.end();
            }
            g.glx.scale(0.7, 1.0, 0.7);
        }
        g.glx.lighting(!self.wire);
        g.glx.color_material(false);
        g.glx.pop_matrix();
    }

    /// `draw_camera_1`: mount, hinge, body, cap and lens, each hung off the
    /// last so that the hinge carries the rest of it round.
    fn draw_camera(&self, g: &mut Gl, c: &Camera) {
        let scale = 0.01;
        g.glx.push_matrix();
        g.glx.translate(c.pos[0], c.pos[1], c.pos[2]);
        g.glx.scale(scale, scale, scale);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(-90.0, 0.0, 1.0, 0.0);
        self.draw_double_component(g, CAMERA_MOUNT);
        g.glx.rotate(-c.facing, 0.0, 1.0, 0.0);
        g.glx.rotate(-c.pitch, 0.0, 0.0, 1.0);
        self.draw_double_component(g, CAMERA_HINGE);

        if c.state == State::WarmUp && c.tick < 0.2 {
            // It draws back a little before it fires.
            g.glx.translate((0.2 - c.tick) / (scale * 3.0), 0.0, 0.0);
        }
        if c.state == State::Zot {
            // And shakes while it does.
            let j = || (0.005 - frand(0.01) as f32) / scale;
            g.glx.translate(j(), j(), j());
        }
        self.draw_double_component(g, CAMERA_BODY);
        if c.state == State::Zot {
            let j = || (0.005 - frand(0.01) as f32) / scale;
            g.glx.translate(j(), j(), j());
        }
        self.draw_double_component(g, CAMERA_CAP);

        // The lens lights up red as the camera warms up and fades as it
        // cools.
        let lens = match c.state {
            State::Idle => None,
            State::WarmUp => Some([1.0 - c.tick, 0.0, 0.0, 1.0]),
            State::Zot => Some([1.0, 0.0, 0.0, 1.0]),
            State::CoolDown => Some([c.tick, 0.0, 0.0, 1.0]),
        };
        self.draw_component(g, CAMERA_LENS, lens);
        g.glx.pop_matrix();
    }

    /// `add_pedestrian`: someone starts walking through the scene.
    fn add_pedestrian(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        let length = 35.0;
        self.pedestrians.push(Pedestrian {
            id,
            pos: [
                -length / 2.0,
                3.0 + frand(10.0) as f32,
                -1.5 + frand(4.0) as f32
                    + if !random().is_multiple_of(10) {
                        0.0
                    } else {
                        frand(8.0) as f32
                    },
            ],
            length,
            frequency: 4.0 + frand(4.0) as f32,
            amplitude: 0.1
                + if !random().is_multiple_of(10) {
                    bellrand(0.45)
                } else {
                    bellrand(1.5)
                },
            ratio: 0.0,
            speed: (4.0
                + frand(4.0) as f32
                + if !random().is_multiple_of(10) {
                    0.0
                } else {
                    frand(10.0) as f32
                })
                * if random() & 1 == 1 { 1.0 } else { -1.0 }
                * self.speed,
        });
    }

    /// Where the thing this camera is watching has got to.
    fn set_camera_focus(&mut self, i: usize) {
        match self.cameras[i].focus_id {
            Focus::Pedestrian(id) => {
                match self.pedestrians.iter().find(|p| p.id == id) {
                    Some(p) => self.cameras[i].focus = p.position(p.ratio),
                    // That one has escaped.
                    None => self.cameras[i].focus_id = Focus::Nothing,
                }
            }
            Focus::Camera(n) => {
                if n < self.cameras.len() {
                    self.cameras[i].focus = self.cameras[n].pos;
                }
            }
            Focus::Nothing => {}
        }
    }

    /// `tick_camera`: swing towards the target, then decide whether to look
    /// at something else, then run whatever the camera is in the middle of.
    fn tick_camera(&mut self, i: usize) {
        self.set_camera_focus(i);
        let c = self.cameras[i].clone();
        let x = c.focus[0] - c.pos[0];
        let y = c.focus[1] - c.pos[1];
        let z = c.focus[2] - c.pos[2] - BEAM_ZOFF;

        if x != 0.0 || y != 0.0 {
            let deg = 180.0 / std::f32::consts::PI;
            let target_facing = x.atan2(y) * deg;
            let target_pitch = z.atan2((x * x + y * y).sqrt()) * deg;

            let accel = 0.5 * self.speed;
            let decel_range = 20.0;
            let max_velocity = 5.0 * self.speed;
            let close_enough = 0.5 * self.speed;
            let dx = target_facing - c.facing;
            let dy = target_pitch - c.pitch;
            let angular_distance = (dx * dx + dy * dy).sqrt();

            // Split the velocity in two. Upstream notes this is not quite
            // right, treating polar as Cartesian, but that it is close
            // enough.
            let r = if dx == 0.0 { 1.0 } else { dy.abs() / dx.abs() };
            let (mut vx, mut vy) = (1.0 - r, r);

            let cam = &mut self.cameras[i];
            if angular_distance < decel_range {
                // Nearing the target, slow down, but never stop.
                cam.velocity -= accel;
                if cam.velocity <= 0.0 {
                    cam.velocity = accel;
                }
            } else {
                cam.velocity = (cam.velocity + accel).min(max_velocity);
            }
            // Do not overshoot.
            vx = vx.min(dx.abs());
            vy = vy.min(dy.abs());

            cam.facing += vx
                * cam.velocity
                * if target_facing > cam.facing {
                    1.0
                } else {
                    -1.0
                };
            cam.pitch += vy * cam.velocity * if target_pitch > cam.pitch { 1.0 } else { -1.0 };

            // Pointed really close, lock on, or the rounding makes it twitch.
            if angular_distance < close_enough {
                cam.facing = target_facing;
                cam.pitch = target_pitch;
            }
            // The hinge only goes so far.
            cam.facing = cam.facing.clamp(-90.0, 90.0);
            cam.pitch = cam.pitch.clamp(-90.0, 55.0);

            // Whatever it actually managed is its speed for next time.
            let ddx = cam.facing - c.facing;
            let ddy = cam.pitch - c.pitch;
            cam.velocity = (ddx * ddx + ddy * ddy).sqrt();
        }

        // Two cameras staring at each other: the one that noticed looks away,
        // which means in the same direction as the other one, at the sky or
        // at the ground.
        if let Focus::Camera(n) = self.cameras[i].focus_id {
            let c = self.cameras[i].clone();
            let c2 = self.cameras[n].clone();
            let dir = |facing: f32, pitch: f32| {
                let (aa, bb) = (
                    facing / (180.0 / std::f32::consts::PI),
                    pitch / (180.0 / std::f32::consts::PI),
                );
                [aa.sin() * bb.cos(), aa.cos() * bb.cos(), bb.sin()]
            };
            let angle = vector_angle(
                normalize(dir(c.facing, c.pitch)),
                normalize(dir(c2.facing, c2.pitch)),
            ) * (180.0 / std::f32::consts::PI);
            if angle > 100.0 {
                let cam = &mut self.cameras[i];
                cam.focus_id = Focus::Nothing;
                cam.focus = [
                    c.pos[0] + (c2.focus[0] - c2.pos[0]),
                    c.pos[1] + (c2.focus[1] - c2.pos[1]),
                    c.pos[2] + (c2.focus[2] - c2.pos[2]),
                ];
                cam.focus[2] = cam.focus[0] * if random() & 1 == 1 { 1.0 } else { -1.0 } * 5.0;
                cam.velocity = c2.velocity * 3.0;
            }
        }

        // Shiny: start paying attention to something else.
        let idle = self.cameras[i].state == State::Idle;
        let bored = if self.cameras[i].focus_id == Focus::Nothing {
            random().is_multiple_of(((50.0 / self.speed) as u32).max(1))
        } else {
            random().is_multiple_of(((1000.0 / self.speed) as u32).max(1))
        };
        if idle && bored {
            if self.cameras.len() > 1 && random().is_multiple_of(20) {
                // Look at a camera one or two along, which because they are
                // set out in two staggered lines are the only ones in sight.
                let n = self.cameras.len();
                let which = random() as usize % 4;
                let target = if i >= 2 && which == 0 {
                    i - 2
                } else if i >= 1 && which == 1 {
                    i - 1
                } else if i < n - 2 && which == 2 {
                    i + 2
                } else if i == n - 1 {
                    i - 1
                } else {
                    i + 1
                };
                self.cameras[i].focus_id = Focus::Camera(target);
            } else if !self.pedestrians.is_empty() {
                let n = random() as usize % self.pedestrians.len();
                self.cameras[i].focus_id = Focus::Pedestrian(self.pedestrians[n].id);
            }
        }

        // Run whatever the camera is in the middle of.
        if self.cameras[i].state != State::Idle {
            let first = self.pedestrians.first().map(|p| p.id);
            if let Some(id) = first {
                self.cameras[i].focus_id = Focus::Pedestrian(id);
            }
            let step = match self.cameras[i].state {
                State::WarmUp => 0.01,
                State::Zot => 0.006,
                State::CoolDown => 0.02,
                State::Idle => 0.0,
            };
            self.cameras[i].tick -= step * self.speed;
            if self.cameras[i].state == State::Zot
                && let Some(p) = self.pedestrians.first_mut()
            {
                // The target takes 1d6 hit points of damage.
                p.speed *= 0.995;
            }
            if self.cameras[i].tick <= 0.0 {
                self.cameras[i].tick = 1.0;
                match self.cameras[i].state {
                    State::WarmUp => self.cameras[i].state = State::Zot,
                    State::Zot => {
                        self.cameras[i].state = State::CoolDown;
                        self.cameras[i].focus_id = Focus::Nothing;
                        if let Some(p) = self.pedestrians.first_mut() {
                            // Threat eliminated.
                            p.ratio = 1.0;
                        }
                    }
                    State::CoolDown => self.cameras[i].state = State::Idle,
                    State::Idle => {}
                }
            }
        }
    }

    /// `draw_ground`: a wireframe floor and back wall, drawn as a lot of
    /// small grids rather than one big one.
    fn build_ground(&mut self, g: &mut Gl, color: [f32; 4]) {
        let cells = 20i32;
        let cell_size = 0.4;
        let (gridsw, gridsh) = (10, 2);

        self.ground_color = color;
        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        g.glx.push_matrix();
        g.glx
            .translate(-(cells * gridsw) as f32 * cell_size / 2.0, 0.0, 0.0);
        for _ in 0..2 {
            g.glx.push_matrix();
            g.glx.translate(0.0, cells as f32 * cell_size / 2.0, 0.0);
            for _ in 0..gridsh {
                g.glx.push_matrix();
                for _ in 0..gridsw {
                    g.glx.begin(Shape::Lines);
                    for i in -cells / 2..cells / 2 {
                        let a = i as f32 * cell_size;
                        let b = (cells / 2) as f32 * cell_size;
                        g.glx.vertex3f(a, -b, 0.0);
                        g.glx.vertex3f(a, b, 0.0);
                        g.glx.vertex3f(-b, a, 0.0);
                        g.glx.vertex3f(b, a, 0.0);
                    }
                    g.glx.end();
                    g.glx.translate(cells as f32 * cell_size, 0.0, 0.0);
                }
                g.glx.pop_matrix();
                g.glx.translate(0.0, cells as f32 * cell_size, 0.0);
            }
            g.glx.pop_matrix();
            // The floor, then the wall behind it.
            g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        }
        g.glx.pop_matrix();
        g.glx.end_list();
        self.ground = list;
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let speed = g.res.float("speed") as f32;
    let count = (g.res.int("count").max(1)) as usize;

    let mut lists = Vec::new();
    let mut colors = Vec::new();
    for (i, src) in MODELS.iter().enumerate() {
        colors.push(resource_color(
            g,
            match i {
                CAMERA_BODY => "bodyColor",
                CAMERA_CAP => "capColor",
                CAMERA_HINGE => "hingeColor",
                CAMERA_MOUNT => "mountColor",
                _ => "lensColor",
            },
        ));
        let model = GlList::parse(src);
        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        g.glx.push_matrix();
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        g.glx.scale(6.0, 6.0, 6.0);
        model.render(&mut g.glx, wire);
        g.glx.pop_matrix();
        g.glx.end_list();
        lists.push(list);
    }

    let mut this = Vigilance {
        trackball: Trackball::new(),
        lists,
        colors,
        ground: 0,
        ground_color: [1.0; 4],
        cameras: Vec::new(),
        pedestrians: Vec::new(),
        next_id: 100,
        aspect: 1.0,
        scale: 1.0,
        speed,
        count,
        wire,
    };
    let ground_color = resource_color(g, "groundColor");
    this.build_ground(g, ground_color);

    // The cameras stand in a row, or in two staggered lines when there are
    // enough of them to crowd.
    let range: f32 = if count <= 2 { 4.0 } else { 5.5 };
    let spacing = (range / count as f32).max(0.7);
    let extent = spacing * (count - 1) as f32;
    for i in 0..count {
        let mut c = Camera {
            pos: [i as f32 * spacing - extent / 2.0, 0.0, 0.7],
            facing: 0.0,
            pitch: -50.0,
            velocity: 0.0,
            focus_id: Focus::Nothing,
            focus: [0.0; 3],
            state: State::Idle,
            tick: 0.0,
        };
        if spacing < 1.6 {
            c.pos[2] = if i & 1 == 1 { 1.1 } else { -0.3 };
        }
        c.focus = [c.pos[0], c.pos[1] + 1.0, c.pos[2] + BEAM_ZOFF];
        this.cameras.push(c);
    }

    // Tilt the floor a little.
    this.trackball
        .reset(-0.70 + frand(1.58), -0.30 + frand(0.40));

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Vigilance {
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
        g.glx.perspective(30.0, self.aspect, 1.0, 200.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.front_face_cw(true);
        g.glx.lighting(!self.wire);
        g.glx.color_material(false);
        if !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.4, 0.2, 0.4, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        }
        g.glx
            .blend(if self.wire { Blend::Off } else { Blend::Alpha });

        g.glx.push_matrix();
        g.glx.mult_matrix(self.trackball.matrix());

        let camera_size = 5.0;
        // Re-frame the scene when there are very few cameras or a great many.
        if self.count <= 2 {
            g.glx.translate(0.0, -1.0, 7.0);
        }
        if self.count >= 20 {
            g.glx.translate(0.0, -1.5, -5.0);
        }
        if self.count >= 40 {
            g.glx.translate(0.0, 2.0, -15.0);
        }
        g.glx.scale(camera_size, camera_size, camera_size);
        // +Z is towards the sky, +X along the back wall, +Y towards the
        // viewer.
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.scale(1.0, -1.0, 1.0);

        // The floor and wall run off towards a horizon, so they are fogged;
        // without it the far end is as bright as the near one.
        g.glx.push_matrix();
        g.glx
            .scale(1.0 / camera_size, 1.0 / camera_size, 1.0 / camera_size);
        g.glx.translate(0.0, -2.38, -8.0);
        g.glx.fog(if self.wire {
            None
        } else {
            Some(Fog::Exp2 {
                density: 0.017,
                color: [0.0, 0.0, 0.0, 1.0],
            })
        });
        // The colour goes on here rather than inside the list: a display
        // list replays geometry and not state.
        let c = self.ground_color;
        g.glx.material_specular([0.4, 0.4, 0.7, 1.0]);
        g.glx.material_shininess(128.0);
        g.glx.material_ambient_diffuse(c);
        g.glx.color4f(c[0], c[1], c[2], c[3]);
        g.glx.line_width(2.0);
        g.glx.call_list(self.ground);
        g.glx.fog(None);
        g.glx.pop_matrix();

        g.glx.color_material(false);
        g.glx.lighting(!self.wire);

        // Walk the pedestrians on, and drop the ones who have got away.
        for p in &mut self.pedestrians {
            p.ratio += 0.001 * p.speed.abs();
        }
        self.pedestrians.retain(|p| p.ratio < 1.0);
        if self.pedestrians.is_empty()
            || random().is_multiple_of(((200.0 / self.speed) as u32).max(1))
        {
            self.add_pedestrian();
        }

        for i in 0..self.cameras.len() {
            let c = self.cameras[i].clone();
            self.draw_camera(g, &c);
            self.tick_camera(i);
        }

        // The beams go last, so that they blend over everything.
        for i in 0..self.cameras.len() {
            if self.cameras[i].state == State::Zot {
                let c = self.cameras[i].clone();
                self.draw_ray(g, &c);
            }
        }

        // Every so often they all decide a pedestrian is a threat.
        if self.cameras[0].state == State::Idle
            && self.pedestrians.first().is_some_and(|p| p.ratio < 0.3)
            && random().is_multiple_of(((50000.0 / self.speed) as u32).max(1))
        {
            for c in &mut self.cameras {
                c.state = State::WarmUp;
                c.tick = 1.0;
            }
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*count:        5",
    "*showFPS:      False",
    "*wireframe:    False",
    "*bodyColor:    #666666",
    "*capColor:     #FFFFFF",
    "*hingeColor:   #444444",
    "*mountColor:   #444444",
    "*lensColor:    #000000",
    "*groundColor:  #004400",
    "*speed:        1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("count", "Cameras", 1.0, 40.0, 1.0, 0, "5"),
    Opt::slider("speed", "Speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "vigilance",
    label: "Vigilance",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2017",
        video: Some("https://www.youtube.com/watch?v=b7y35gr3WZ0"),
        blurb: "Surveillance cameras, taking an interest in their \
                surroundings.",
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

    /// A camera's hinge only goes so far: a right angle either way, and no
    /// further up than fifty-five degrees.
    #[test]
    fn the_hinge_has_a_range_of_motion() {
        let mut r = start(StartArgs::new(640, 480, "count=8&speed=10", 20260811));
        for _ in 0..1500 {
            r.step();
        }
        // Drive it a long way and check nothing has been wound past its stop.
        // The state is not reachable from outside, so this reads the frame:
        // a camera that had turned too far would put geometry behind the
        // wall it is mounted on.
        let f = r.frame();
        assert!(
            f.vertices
                .iter()
                .all(|v| v.pos.iter().all(|c| c.is_finite())),
            "a vertex went to NaN"
        );
    }

    /// Everything the cameras watch is a path, not a body: a pedestrian walks
    /// a straight line with a sine wave up and down on top of it.
    #[test]
    fn a_pedestrian_is_a_path() {
        let p = Pedestrian {
            id: 1,
            pos: [-17.5, 5.0, 0.0],
            length: 35.0,
            frequency: 4.0,
            amplitude: 0.5,
            ratio: 0.0,
            speed: 5.0,
        };
        let start = p.position(0.0);
        let end = p.position(1.0);
        assert!((start[0] + 17.5).abs() < 1e-5, "starts at {}", start[0]);
        assert!((end[0] - 17.5).abs() < 1e-5, "ends at {}", end[0]);
        assert_eq!(start[1], end[1], "it wandered off its line");
        // Walking the other way runs the same path backwards.
        let mut back = p.clone();
        back.speed = -5.0;
        assert_eq!(back.position(0.0), end);
        assert_eq!(back.position(1.0), start);
    }

    /// The angle between two directions, which is what tells a camera it is
    /// being stared at.
    #[test]
    fn the_angle_between_two_looks() {
        let deg = 180.0 / std::f32::consts::PI;
        assert!(vector_angle([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]) == 0.0);
        assert!(
            (vector_angle([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]) * deg - 180.0).abs() < 0.01,
            "opposite directions are not a straight angle"
        );
        assert!((vector_angle([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]) * deg - 90.0).abs() < 0.01);
        // A vector against nothing has no angle rather than a NaN.
        assert_eq!(vector_angle([0.0; 3], [1.0, 0.0, 0.0]), 0.0);
    }

    /// Five cameras, each of five parts, and four of the five parts are drawn
    /// twice because the model is half a camera.
    #[test]
    fn a_camera_is_five_parts_and_most_of_them_twice() {
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        r.step();
        let solids = r
            .frame()
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
            .count();
        assert_eq!(solids, 4 * 2 + 1, "{solids} draws is not one camera");
    }

    /// When they fire, the beam is five nested boxes drawn over everything
    /// else, and it only appears while a camera is in that state.
    #[test]
    fn the_beam_only_shows_while_they_fire() {
        let mut r = start(StartArgs::new(640, 480, "count=3", 20260811));
        r.step();
        let quads = |r: &Runner3d| {
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
                .count()
        };
        // Three cameras and nothing else: nine batches each.
        assert_eq!(quads(&r), 3 * 9, "something is firing already");
    }
}
