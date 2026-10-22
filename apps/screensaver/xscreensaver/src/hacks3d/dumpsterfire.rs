//! Port of `hacks/glx/dumpsterfire.c`.
//!
//! ```text
//! dumpsterfire, Copyright © 2025 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Created by jwz: 22-Apr-2025
//!
//! Dumpster model by: xx_n0va_x https://skfb.ly/psRAy
//!   and slightly modified by jwz.
//!   Licensed under Creative Commons Attribution
//!   http://creativecommons.org/licenses/by/4.0/
//! ```
//!
//! A dumpster drops in, one lid opens, it catches fire, burns for a while, is
//! put out, closes up and rolls away, and then another one drops in.
//!
//! The fire is ten thousand sprites whose colours run down a table of the
//! colour a black body glows at each temperature, from thirteen hundred
//! degrees at the base to five hundred and fifty at the tips, and each
//! particle's remaining life is also its temperature. Above the rim they pick
//! up the wind; above that again they are pulled back towards the middle,
//! which is what turns the column into a cone.
//!
//! Upstream billboards each sprite by taking the modelview matrix, forcing its
//! rotation to the identity and loading that back, which is a matrix change
//! and so a batch each: ten thousand of them. The result of that is a quad
//! standing square to the camera at the particle's transformed position, so
//! this transforms the positions itself and emits the whole fire as one batch
//! against an identity modelview. Same pixels, one draw.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gl::{Blend, Mat4, Shape, TexEnv};
use crate::runtime::gllist::GlList;
use crate::runtime::rotator::Rotator;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

/// The parts of the model, in the order they are drawn.
const COMPONENTS: [(&str, &str, bool); 7] = [
    // (model, colour resource, whether it is half a dumpster and is mirrored)
    (
        crate::models::DUMPSTER_MODEL_FRAME_HALF,
        "dumpsterFrameColor",
        true,
    ),
    (
        crate::models::DUMPSTER_MODEL_PANELS_HALF,
        "dumpsterPanelColor",
        true,
    ),
    (
        crate::models::DUMPSTER_MODEL_INSIDE_HALF,
        "insideColor",
        true,
    ),
    (
        crate::models::DUMPSTER_MODEL_HINGES_HALF,
        "hingesColor",
        true,
    ),
    (crate::models::DUMPSTER_MODEL_AXLE, "axleColor", false),
    (crate::models::DUMPSTER_MODEL_LID, "lidColor", false),
    (
        crate::models::DUMPSTER_MODEL_LID_PANELS,
        "lidPanelColor",
        false,
    ),
];

const FRAME_HALF: usize = 0;
const PANELS_HALF: usize = 1;
const INSIDE_HALF: usize = 2;
const HINGES_HALF: usize = 3;
const AXLE: usize = 4;
const LID: usize = 5;
const LID_PANELS: usize = 6;

/// The colour a black body glows at each temperature, from 550°C to over
/// 1300°C. The brightest is repeated so that the hottest flames are not all
/// hidden down inside the dumpster.
const FIRE_COLORS: [u32; 24] = [
    0x352201, 0x542803, 0x681100, 0x861600, 0xA00000, 0xC11B1B, 0xD44115, 0xE9582C, 0xE97E1C,
    0xFFAA0F, 0xFBC034, 0xFFCF61, 0xFFE6AD, 0xFFE6AD, 0xFFE6AD, 0xFFE6AD, 0xFFE6AD, 0xFFE6AD,
    0xFFE6AD, 0xFFE6AD, 0xFFE6AD, 0xFFE6AD, 0xFFE6AD, 0xFFE6AD,
];

/// A colour resource as GL wants it. The defaults are all hex triples, and
/// a colour that will not parse is drawn white, which is at least visible.
fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

fn rgb(x: u32) -> [f32; 4] {
    [
        ((x >> 16) & 0xFF) as f32 / 255.0,
        ((x >> 8) & 0xFF) as f32 / 255.0,
        (x & 0xFF) as f32 / 255.0,
        1.0,
    ]
}

fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

const TEX_SIZE: usize = 128;

/// The whole thing is a cycle of seven acts.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum State {
    Drop,
    Ignite,
    Open,
    Burn,
    Quench,
    Close,
    Roll,
}

const STATES: [State; 7] = [
    State::Drop,
    State::Ignite,
    State::Open,
    State::Burn,
    State::Quench,
    State::Close,
    State::Roll,
];

#[derive(Clone, Copy, Default)]
struct Particle {
    fade: f32,
    color: [f32; 4],
    pos: [f32; 3],
    speed: [f32; 3],
    accel: [f32; 3],
}

struct Dumpster {
    trackball: Trackball,
    rot: Rotator,
    pos: [f32; 3],
    wind: [f32; 3],
    tick: f32,
    state: State,
    /// How far each lid is open, in turns of the hinge.
    lid_angle: [f32; 2],
    lists: Vec<u32>,
    colors: Vec<[f32; 4]>,
    texture: u32,
    density: f32,
    particles: Vec<Particle>,
    aspect: f32,
    scale: f32,
    speed: f32,
    wire: bool,
}

/// `build_texture`: a soft round blob, brightest in the middle.
fn build_texture(g: &mut Gl) -> u32 {
    let s2 = (TEX_SIZE / 2) as f64;
    let mut data = Vec::with_capacity(TEX_SIZE * TEX_SIZE * 4);
    for y in 0..TEX_SIZE {
        for x in 0..TEX_SIZE {
            let dx = s2 - x as f64;
            let dy = s2 - y as f64;
            let dist = (dx * dx + dy * dy).sqrt() / s2;
            let v = (255.0 * (if dist > 1.0 { 0.0 } else { 1.0 - dist }).sin()) as u8;
            data.extend_from_slice(&[0xFF, 0xFF, 0xFF, v]);
        }
    }
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    g.glx.tex_image_2d(TEX_SIZE as i32, TEX_SIZE as i32, data);
    id
}

/// The order to draw the sprites in, furthest first.
///
/// Upstream hands each position to `gluProject` in the order y, z, x, which
/// looks like a slip: the sort is by the depth of a point that is not where
/// the sprite ends up. It is kept, because it is what the saver looks like.
fn sort_order(particles: &[Particle], m: &Mat4) -> Vec<usize> {
    let mut order: Vec<(f32, usize)> = particles
        .iter()
        .enumerate()
        .map(|(i, p)| (transform(m, [p.pos[1], p.pos[2], p.pos[0]])[2], i))
        .collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    order.into_iter().map(|(_, i)| i).collect()
}

/// Multiply a point through a column-major 4x4, as GL stores them.
fn transform(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    let m = &m.0;
    std::array::from_fn(|i| m[i] * p[0] + m[4 + i] * p[1] + m[8 + i] * p[2] + m[12 + i])
}

impl Dumpster {
    /// `draw_component`.
    fn draw_component(&self, g: &mut Gl, i: usize, half: bool) {
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);
        g.glx.material_ambient_diffuse(self.colors[i]);
        g.glx.front_face_cw(false);
        g.glx.call_list(self.lists[i]);
        if half {
            // Half a dumpster, mirrored: the winding flips with it.
            g.glx.push_matrix();
            g.glx.scale(-1.0, 1.0, 1.0);
            g.glx.front_face_cw(true);
            g.glx.call_list(self.lists[i]);
            g.glx.pop_matrix();
        }
    }

    /// `draw_box`: the dumpster, dropping in, opening, closing or rolling out
    /// according to where in the cycle we are.
    fn draw_box(&mut self, g: &mut Gl) {
        g.glx.push_matrix();
        g.glx.blend(Blend::Off);
        g.glx.texturing(false);
        g.glx.depth_test(true);
        g.glx.depth_mask(true);
        g.glx.lighting(!self.wire);
        g.glx.cull_face(true);
        g.glx.color_material(self.wire);
        g.glx.scale(12.0, 12.0, 12.0);

        match self.state {
            State::Drop => {
                self.pos[0] = 0.0;
                self.pos[1] = 0.0;
                // It falls in and bounces.
                self.pos[2] = (1.0 - ease(Ease::OutBounce, self.tick as f64) as f32) * 3.0;
            }
            State::Open => {
                let i = usize::from(self.lid_angle[0] == 0.0);
                self.lid_angle[i] = self.tick + 0.0001;
            }
            State::Close => {
                let i = usize::from(self.lid_angle[0] == 0.0);
                self.lid_angle[i] = (1.0 - self.tick) + 0.0001;
            }
            State::Roll => self.pos[0] += self.tick * 2.0,
            _ => self.pos[2] = 0.0,
        }

        if self.wire {
            g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        }
        g.glx.translate(self.pos[0], self.pos[2], self.pos[1]);
        for i in [FRAME_HALF, PANELS_HALF, INSIDE_HALF] {
            self.draw_component(g, i, true);
        }
        self.draw_component(g, AXLE, false);
        self.draw_component(g, HINGES_HALF, true);

        for i in 0..2 {
            let deg = 115.0;
            let off = [0.0f32, 0.63, -0.25];
            // Closing eases the other way round, so a lid slams rather than
            // settling.
            let a2 = if self.state == State::Close {
                1.0 - ease(Ease::OutBounce, (1.0 - self.lid_angle[i]) as f64) as f32
            } else {
                ease(Ease::OutBounce, self.lid_angle[i] as f64) as f32
            };
            g.glx.push_matrix();
            g.glx.translate(off[0], off[1], off[2]);
            g.glx.rotate(-deg * a2, 1.0, 0.0, 0.0);
            g.glx.translate(-off[0], -off[1], -off[2]);
            if i == 1 {
                g.glx.translate(-0.46, 0.0, 0.0);
            }
            self.draw_component(g, LID, false);
            self.draw_component(g, LID_PANELS, false);
            g.glx.pop_matrix();
        }
        g.glx.pop_matrix();
    }

    /// `draw_fire`.
    fn draw_fire(&mut self, g: &mut Gl) {
        if matches!(self.state, State::Drop | State::Close | State::Roll) {
            return;
        }

        // Not too big or the flames peek through a closed lid, not too small
        // or the fire looks spotty.
        let mut size = 0.25 / self.density;
        size = size.clamp(0.80, 1.20);
        if self.wire {
            size *= 0.6;
        }

        g.glx.push_matrix();
        if !self.wire {
            g.glx.depth_test(true);
            g.glx.depth_mask(false);
            g.glx.texturing(true);
            g.glx.tex_env(TexEnv::Modulate);
            g.glx.bind_texture(self.texture);
            g.glx.lighting(false);
            g.glx.blend(Blend::Alpha);
        }
        g.glx.color_material(true);

        // Sit over whichever lid is open.
        g.glx.translate(
            if self.lid_angle[0] == 0.0 { -1.0 } else { 1.0 } * 2.3,
            4.0,
            0.0,
        );
        g.glx.rotate(90.0, -1.0, 0.0, 0.0); // Z is up
        g.glx.scale(0.5, 0.5, 0.5);

        // Transparency needs the sprites drawn back to front. Upstream sorts
        // by the projected depth of each particle; it hands the coordinates to
        // gluProject in the order y, z, x, which looks like a slip, but the
        // ordering it produces is what the saver has always looked like.
        let m = g.glx.modelview();
        let order = sort_order(&self.particles, &m);

        // One quad per particle, square to the camera, at the particle's
        // transformed position. That is what upstream's billboard comes to,
        // so the positions are transformed here and the lot goes out as one
        // batch against an identity modelview.
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.begin(if self.wire {
            Shape::Lines
        } else {
            Shape::Quads
        });
        for i in &order {
            let p = &self.particles[*i];
            let t = transform(&m, p.pos);
            g.glx
                .color4f(p.color[0], p.color[1], p.color[2], p.color[3]);
            for (u, v) in [(1.0, 1.0), (0.0, 1.0), (0.0, 0.0), (1.0, 0.0)] {
                g.glx.tex_coord2f(u, v);
                g.glx
                    .vertex3f(t[0] + (u - 0.5) * size, t[1] + (v - 0.5) * size, t[2]);
            }
        }
        g.glx.end();
        g.glx.pop_matrix();
        g.glx.pop_matrix();

        if !self.wire {
            g.glx.depth_mask(true);
        }

        self.tick_particles();
    }

    /// Move every particle on one step, and bring the dead ones back at the
    /// bottom of the fire.
    fn tick_particles(&mut self) {
        let quenching = matches!(self.state, State::Quench | State::Ignite);
        let burning = self.state < State::Quench;
        let wind = self.wind;
        for p in &mut self.particles {
            for l in 0..3 {
                p.pos[l] += p.speed[l];
            }
            // Above head height they are pulled back towards the middle,
            // which is what turns the column into a cone.
            if p.pos[2] > 5.0 {
                p.accel[0] = 0.0016 * if p.pos[0] > 0.0 { -1.0 } else { 1.0 };
                p.accel[1] = 0.0016 * if p.pos[1] > 0.0 { -1.0 } else { 1.0 };
            }
            for l in 0..3 {
                p.speed[l] += p.accel[l];
            }
            // Clear of the dumpster, the wind gets at them. Upstream notes
            // that this should really add to the speed, but that adding to
            // the acceleration looks better.
            if p.pos[2] > 4.5 {
                for (a, w) in p.accel.iter_mut().zip(wind.iter()) {
                    *a += w;
                }
            }

            // Alpha is both how long the particle has left and how hot it is.
            p.color[3] -= p.fade;
            if quenching {
                p.color[3] -= p.fade * 3.0;
            }
            if p.color[3] <= 0.0 {
                if burning {
                    *p = Particle {
                        pos: [0.0; 3],
                        speed: [
                            0.12 * (frand(1.0) as f32 - 0.5),
                            0.12 * (frand(1.0) as f32 - 0.5),
                            0.06 * (frand(1.0) as f32 - 0.5),
                        ],
                        accel: [0.0, 0.0, 0.0032],
                        fade: frand(0.2) as f32 + 0.006,
                        color: rgb(FIRE_COLORS[FIRE_COLORS.len() - 1]),
                    };
                }
            } else {
                let i =
                    ((p.color[3] * FIRE_COLORS.len() as f32) as usize).min(FIRE_COLORS.len() - 1);
                let c = rgb(FIRE_COLORS[i]);
                p.color[..3].copy_from_slice(&c[..3]);
            }
        }
    }

    /// `tick_dumpster`: move the story on.
    fn advance(&mut self) {
        if self.trackball.button_down() {
            return;
        }
        // The animation is written against a fixed frame rate rather than
        // real time.
        let fps = 27.0;
        let ts = match self.state {
            State::Drop => 3.0,
            State::Ignite => 1.0,
            State::Open => 1.0,
            State::Burn => 99.0,
            State::Quench => 3.0,
            State::Close => 1.0,
            State::Roll => 3.0,
        };
        self.tick += self.speed * (1.0 / (ts * fps));
        if self.tick < 1.0 {
            return;
        }
        self.tick = 0.0;
        self.state = STATES[(self.state as usize + 1) % STATES.len()];
        match self.state {
            State::Ignite => {
                // Pick which lid we are opening, and which way the wind blows.
                self.lid_angle[(random() % 2) as usize] += 0.001;
                self.wind = [0.15 * (bellrand(1.0) - 0.5), -0.15 * bellrand(0.5), 0.0];
            }
            State::Roll => self.lid_angle = [0.0, 0.0],
            State::Close => self
                .particles
                .iter_mut()
                .for_each(|p| *p = Particle::default()),
            State::Drop => self.trackball.reset(0.0, 0.0),
            _ => {}
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let speed = g.res.float("speed") as f32;
    let density = g.res.float("density") as f32;
    let nparticles = ((10000.0 * density) as usize).max(10);

    let mut lists = Vec::new();
    let mut colors = Vec::new();
    for (src, key, _) in COMPONENTS {
        colors.push(resource_color(g, key));
        let model = GlList::parse(src);
        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        g.glx.push_matrix();
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        model.render(&mut g.glx, wire);
        g.glx.pop_matrix();
        g.glx.end_list();
        lists.push(list);
    }

    let texture = build_texture(g);
    let mut this = Dumpster {
        trackball: Trackball::new(),
        rot: Rotator::new(
            if g.res.bool("spin") {
                speed as f64 * 0.04
            } else {
                0.0
            },
            if g.res.bool("spin") {
                speed as f64 * 0.04
            } else {
                0.0
            },
            if g.res.bool("spin") {
                speed as f64 * 0.04
            } else {
                0.0
            },
            0.5,
            if g.res.bool("wander") {
                speed as f64 * 0.004
            } else {
                0.0
            },
            false,
        ),
        pos: [0.0; 3],
        wind: [0.0; 3],
        tick: 0.0,
        state: State::Drop,
        lid_angle: [0.0, 0.0],
        lists,
        colors,
        texture,
        density,
        particles: vec![Particle::default(); nparticles],
        aspect: 1.0,
        scale: 1.0,
        speed,
        wire,
    };

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Dumpster {
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
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        // A click skips to the end of whatever act we are in: put the fire
        // out, or send the dumpster on its way.
        match self.state {
            State::Drop | State::Ignite => {
                self.state = State::Close;
                self.tick = 1.0;
                true
            }
            State::Open | State::Burn => {
                self.state = State::Open;
                self.tick = 1.0;
                if self.lid_angle[0] != 0.0 {
                    self.lid_angle[0] -= 0.6;
                } else {
                    self.lid_angle[1] -= 0.6;
                }
                true
            }
            _ => false,
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
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx.clear();
        if !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.4, 0.2, 0.4, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        }

        g.glx.push_matrix();
        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 1.0,
            (z as f32 - 0.5) * 15.0,
        );
        g.glx.mult_matrix(self.trackball.matrix());
        let (_, y, _) = self.rot.rotation(!down);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);

        g.glx.translate(0.0, -6.0, 0.0);
        g.glx.scale(0.7, 0.7, 0.7);
        g.glx.rotate(5.0, 1.0, 0.0, 0.0);
        g.glx.rotate(-10.0, 0.0, 1.0, 0.0);
        self.draw_box(g);
        self.draw_fire(g);
        g.glx.pop_matrix();

        self.advance();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:              20000",
    "*showFPS:            False",
    "*wireframe:          False",
    "*dumpsterFrameColor: #777799",
    "*dumpsterPanelColor: #8888AA",
    "*insideColor:        #112211",
    "*hingesColor:        #666666",
    "*axleColor:          #444444",
    "*lidColor:           #8888FF",
    "*lidPanelColor:      #7777EE",
    "*spin:               True",
    "*wander:             True",
    "*speed:              1.0",
    "*density:            1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 5.0, 0.1, 1, "1.0"),
    Opt::slider("density", "Flame density", 0.1, 2.0, 0.1, 1, "1.0"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "dumpsterfire",
    label: "Dumpster Fire",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2025",
        video: Some("https://www.youtube.com/watch?v=odBVYPqvhNY"),
        blurb: "A dumpster drops in, catches fire, burns for a while, is put \
                out and rolls away. Then another one.",
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

    /// Ten thousand sprites arrive as one batch, not ten thousand.
    #[test]
    fn the_whole_fire_is_one_draw() {
        let mut r = start(StartArgs::new(640, 480, "speed=5", 20260811));
        // Get past the drop and the ignition, into the burn.
        let mut burning = false;
        for _ in 0..400 {
            r.step();
            let f = r.frame();
            let quads: Vec<_> = f
                .batches
                .iter()
                .filter(|b| {
                    b.primitive == crate::runtime::gl::Primitive::Triangles && b.count > 10000
                })
                .collect();
            if let Some(b) = quads.first() {
                burning = true;
                assert_eq!(quads.len(), 1, "the fire came out as {} draws", quads.len());
                // A quad is two triangles: six vertices a particle.
                assert_eq!(b.count, 10000 * 6, "{} vertices is not the fire", b.count);
                break;
            }
        }
        assert!(burning, "it never caught fire");
    }

    /// The sprites are drawn in the order upstream draws them: sorted by the
    /// depth of the position with its coordinates rolled round, which is a
    /// slip upstream but is what the fire looks like.
    #[test]
    fn the_sprites_are_sorted_the_way_upstream_sorts_them() {
        let particles: Vec<Particle> = (0..200)
            .map(|i| {
                let f = i as f32;
                Particle {
                    pos: [(f * 0.7).sin() * 3.0, (f * 1.3).cos() * 3.0, f * 0.05],
                    ..Particle::default()
                }
            })
            .collect();
        // A matrix that puts the scene in front of the camera, as the fire's
        // own transform does.
        let mut m = Mat4([0.0; 16]);
        for i in 0..4 {
            m.0[i * 5] = 1.0;
        }
        m.0[14] = -20.0;

        let order = sort_order(&particles, &m);
        assert_eq!(order.len(), particles.len(), "a particle went missing");
        let key = |i: usize| {
            let p = particles[i].pos;
            transform(&m, [p[1], p[2], p[0]])[2]
        };
        assert!(
            order.windows(2).all(|w| key(w[0]) <= key(w[1])),
            "the sprites are out of order"
        );
        // Rolled round: the key is the y coordinate, not the z, so sorting by
        // it does not sort by the depth the sprites are drawn at.
        let by_depth: Vec<usize> = {
            let mut v: Vec<usize> = (0..particles.len()).collect();
            v.sort_by(|a, b| {
                transform(&m, particles[*a].pos)[2].total_cmp(&transform(&m, particles[*b].pos)[2])
            });
            v
        };
        assert_ne!(order, by_depth, "the coordinates are no longer rolled");
    }

    /// The cycle runs all the way round and comes back to the beginning.
    #[test]
    fn the_story_goes_round() {
        let mut r = start(StartArgs::new(640, 480, "speed=5", 20260811));
        let mut seen = Vec::new();
        for _ in 0..4000 {
            r.step();
            // The dumpster is at the origin except while it drops or rolls,
            // so the batch count tells the acts apart well enough: the fire
            // adds a batch of its own.
            let f = r.frame();
            let fire = f
                .batches
                .iter()
                .any(|b| b.primitive == crate::runtime::gl::Primitive::Triangles && b.count > 5000);
            if seen.last() != Some(&fire) {
                seen.push(fire);
            }
        }
        assert!(
            seen.len() >= 3,
            "the fire only changed state {} times",
            seen.len()
        );
    }

    /// Every part of the model loads and has its own colour.
    #[test]
    fn the_dumpster_is_seven_parts() {
        for (src, _, _) in COMPONENTS {
            let model = GlList::parse(src);
            assert!(model.points > 0, "a part of the model is empty");
        }
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        // Four of the seven parts are half a dumpster and are drawn again
        // mirrored, the axle is drawn once, and the two lid parts are drawn
        // once per lid.
        let solids = r
            .frame()
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
            .count();
        assert_eq!(
            solids,
            4 * 2 + 1 + 2 * 2,
            "{solids} draws is not the dumpster"
        );
    }

    /// The blob the sprites are drawn with is brightest in the middle and
    /// nothing at all outside its circle.
    #[test]
    fn the_sprite_fades_out_at_its_rim() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let tex = r.texture(1).expect("the sprite texture");
        let at = |x: usize, y: usize| tex.data[(y * TEX_SIZE + x) * 4 + 3];
        assert!(at(64, 64) > 200, "the middle is only {}", at(64, 64));
        assert_eq!(at(0, 0), 0, "the corner is not empty");
        assert!(at(64, 4) < 40, "the rim is {}", at(64, 4));
    }
}
