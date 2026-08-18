//! Port of `hacks/glx/highvoltage.c`.
//!
//! ```text
//! highvoltage, Copyright © 2024 Jamie Zawinski <jwz@jwz.org>
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
//! A leisurely flight past a line of transmission towers, with the power lines
//! sagging between them.
//!
//! The ten towers are traced rather than modelled: each is a set of bare lines,
//! and the saver thickens every line into a square tube on the way into its
//! display list. The cables get thinner tubes and a string of insulator discs,
//! and only half of each tower is drawn, the other half being the same lines
//! mirrored.
//!
//! Nothing here is lit, depth-tested or culled. It is a flat drawing in one
//! colour on a pale ground, and the only thing that gives it depth is the fog,
//! which fades the far towers out before they reach the horizon.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Fog, Glx, Shape};
use crate::runtime::gllist::{Format, GlList};
use crate::runtime::tube::TubeMesh;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};
use std::collections::VecDeque;

/// One tower: up to four sets of lines. The `d` and `i` towers have no body of
/// their own and the `j` one, a wooden street pole, has no cross-arm but does
/// have the low-voltage wires going off to the houses.
struct TowerModel {
    body: Option<&'static str>,
    cross: Option<&'static str>,
    cables: Option<&'static str>,
    connections: Option<&'static str>,
}

macro_rules! tower {
    ($body:expr, $cross:expr, $cables:expr, $connections:expr) => {
        TowerModel {
            body: $body,
            cross: $cross,
            cables: $cables,
            connections: $connections,
        }
    };
}

fn tower_models() -> Vec<TowerModel> {
    use crate::models::*;
    vec![
        tower!(
            Some(HIGHVOLTAGE_MODEL_TOWER_A_BODY),
            Some(HIGHVOLTAGE_MODEL_TOWER_A_CROSS),
            Some(HIGHVOLTAGE_MODEL_TOWER_A_CABLES),
            None
        ),
        tower!(
            Some(HIGHVOLTAGE_MODEL_TOWER_B_BODY),
            Some(HIGHVOLTAGE_MODEL_TOWER_B_CROSS),
            Some(HIGHVOLTAGE_MODEL_TOWER_B_CABLES),
            None
        ),
        tower!(
            Some(HIGHVOLTAGE_MODEL_TOWER_C_BODY),
            Some(HIGHVOLTAGE_MODEL_TOWER_C_CROSS),
            Some(HIGHVOLTAGE_MODEL_TOWER_C_CABLES),
            None
        ),
        tower!(
            None,
            Some(HIGHVOLTAGE_MODEL_TOWER_D_CROSS),
            Some(HIGHVOLTAGE_MODEL_TOWER_D_CABLES),
            None
        ),
        tower!(
            Some(HIGHVOLTAGE_MODEL_TOWER_E_BODY),
            Some(HIGHVOLTAGE_MODEL_TOWER_E_CROSS),
            Some(HIGHVOLTAGE_MODEL_TOWER_E_CABLES),
            None
        ),
        tower!(
            Some(HIGHVOLTAGE_MODEL_TOWER_F_BODY),
            Some(HIGHVOLTAGE_MODEL_TOWER_F_CROSS),
            Some(HIGHVOLTAGE_MODEL_TOWER_F_CABLES),
            None
        ),
        tower!(
            Some(HIGHVOLTAGE_MODEL_TOWER_G_BODY),
            Some(HIGHVOLTAGE_MODEL_TOWER_G_CROSS),
            Some(HIGHVOLTAGE_MODEL_TOWER_G_CABLES),
            None
        ),
        tower!(
            Some(HIGHVOLTAGE_MODEL_TOWER_H_BODY),
            Some(HIGHVOLTAGE_MODEL_TOWER_H_CROSS),
            Some(HIGHVOLTAGE_MODEL_TOWER_H_CABLES),
            None
        ),
        tower!(
            None,
            Some(HIGHVOLTAGE_MODEL_TOWER_I_CROSS),
            Some(HIGHVOLTAGE_MODEL_TOWER_I_CABLES),
            None
        ),
        tower!(
            Some(HIGHVOLTAGE_MODEL_TOWER_J_BODY),
            None,
            Some(HIGHVOLTAGE_MODEL_TOWER_J_CABLES),
            Some(HIGHVOLTAGE_MODEL_TOWER_J_CONNECTIONS)
        ),
    ]
}

const MAX_WIRES: usize = 20;

/// A tower once it has been measured and compiled: its bounding box, where its
/// power lines attach, and the list holding the tubes.
struct Tower {
    list: u32,
    bbox: [f32; 6],
    wires: Vec<[f32; 3]>,
    has_connections: bool,
}

/// One tower's place in the line. The rotation is always nought: upstream tried
/// giving each tower a lean and left the line commented out, because it made
/// the flight twitchy and the wires missed their attachment points.
#[derive(Clone, Copy, Default)]
struct Obj {
    pos: [f32; 3],
    rot: [f32; 3],
}

#[derive(PartialEq)]
enum State {
    FadeIn,
    Draw,
    FadeOut,
}

struct HighVoltage {
    trackball: Trackball,
    towers: Vec<Tower>,
    meshes: Meshes,
    which: usize,
    objs: VecDeque<Obj>,
    dead: Option<Obj>,
    from: Obj,
    to: Obj,
    ratio: f32,
    state: State,
    /// Which side of the line of towers the camera flies down. It swaps every
    /// time the tower changes, and the vanishing point moves with it.
    left: bool,
    tick: f32,
    fg: [f32; 4],
    bg: [f32; 4],
    width: i32,
    height: i32,
    count: i32,
    speed: f32,
    spacing: f32,
    wire: bool,
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// The three tubes this saver is made of, built once. Going through the matrix
/// stack for each of the six hundred tubes in a tower would be six hundred draw
/// calls; baked into their vertices, a whole tower is one.
struct Meshes {
    steel: TubeMesh,
    disc: TubeMesh,
    line: TubeMesh,
}

impl Meshes {
    /// Solid only: in wireframe the lines are drawn as they came, so none of
    /// these is reached.
    fn new() -> Self {
        Meshes {
            steel: TubeMesh::tube(4, false, false, false),
            disc: TubeMesh::tube(6, true, true, false),
            line: TubeMesh::tube(6, true, false, false),
        }
    }
}

/// Draw a set of lines as square tubes. `cable_type` says which: 0 is the
/// structural steel, 1 the cables, which are thinner, skip the horizontal runs
/// and carry insulator discs, and 2 the low-voltage wires off the street pole.
fn render_lines(g: &mut Glx, m: &Meshes, list: &GlList, cable_type: i32, wire: bool) -> i32 {
    let mut polys = 0;
    if wire {
        list.render(g, true);
        return (list.points / 2) as i32;
    }
    if list.primitive == Shape::Triangles {
        list.render(g, false);
        return (list.points / 3) as i32;
    }
    let diam = 0.25
        * match cable_type {
            0 => 1.0,
            1 => 0.3,
            _ => 0.1,
        };
    let cap = diam / 2.0;
    let p = &list.data;
    assert_eq!(list.primitive, Shape::Lines);
    assert_eq!(list.format, Format::V3f);
    for i in (0..list.points * 3).step_by(6) {
        // The horizontal runs of a cable set are not cable, they are where the
        // power lines are hung.
        if cable_type == 1 && p[i + 2] == p[i + 5] {
            continue;
        }
        let (a, b) = ([p[i], p[i + 1], p[i + 2]], [p[i + 3], p[i + 4], p[i + 5]]);
        polys += m.steel.draw(g, a, b, diam, cap);

        if cable_type == 1 {
            let disc_spacing = 0.5;
            let disc_height = 0.5;
            let disc_width = 10.0;
            let (w, h, d) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
            let len = (w * w + h * h + d * d).sqrt();
            let ndiscs = (len / disc_spacing) as i32;
            for j in 0..ndiscs {
                let r1 = j as f32 / ndiscs as f32;
                let r2 = (j as f32 + disc_height) / ndiscs as f32;
                let at = |r: f32| [a[0] + w * r, a[1] + h * r, a[2] + d * r];
                polys += m.disc.draw(g, at(r1), at(r2), diam * disc_width, 0.0);
            }
        }
    }
    polys
}

/// Measure a tower and compile it: the bounding box it is normalised by, the
/// points its power lines hang from, and the tubes themselves.
fn render_tower(g: &mut Glx, m: &Meshes, model: &TowerModel, wire: bool) -> Tower {
    let parse = |s: Option<&'static str>| s.map(GlList::parse);
    let body = parse(model.body);
    let cross = parse(model.cross);
    let cables = parse(model.cables);
    let connections = parse(model.connections);

    let mut min = [[99999.0f32; 3]; 4];
    let mut max = [[-99999.0f32; 3]; 4];

    // The bounding box of each of the three sets, and of all of them together.
    // The connections are left out of it: the wires off a street pole run away
    // to houses that are not there, and would swamp the box.
    for k in 0..3 {
        let list = match k {
            0 => body.as_ref(),
            1 => cross.as_ref(),
            _ => cables.as_ref(),
        };
        let Some(list) = list else { continue };
        let p = &list.data;
        let (skip, stride) = match list.format {
            Format::V3f => (0, 3),
            _ => (3, 6),
        };
        for j in (skip..skip + list.points * 3).step_by(stride) {
            for c in 0..3 {
                min[k][c] = min[k][c].min(p[j + c]);
                max[k][c] = max[k][c].max(p[j + c]);
            }
        }

        if k > 0 {
            // The cross-arms and cables are mirrored about the body's centre.
            // A tower with no body of its own mirrors about nought, which is
            // what upstream's sentinels work out to.
            let r = min[0][0] + (max[0][0] - min[0][0]) / 2.0;
            for j in (0..list.points * 3).step_by(3) {
                let p2 = [r - p[j], p[j + 1], p[j + 2]];
                for c in 0..3 {
                    min[k][c] = min[k][c].min(p2[c]);
                    max[k][c] = max[k][c].max(p2[c]);
                }
            }
        }
        for c in 0..3 {
            min[3][c] = min[3][c].min(min[k][c]);
            max[3][c] = max[3][c].max(max[k][c]);
        }
    }

    let bbox = [
        min[3][0],
        min[3][1],
        min[3][2],
        max[3][0] - min[3][0],
        max[3][1] - min[3][1],
        max[3][2] - min[3][2],
    ];

    // The power lines hang from the horizontal runs in the cable set.
    let mut wires = Vec::new();
    if let Some(list) = cables.as_ref() {
        let p = &list.data;
        let cx = bbox[0] + bbox[3] / 2.0;
        let cy = bbox[1] + bbox[4] / 2.0;
        for j in (0..list.points * 3).step_by(6) {
            if p[j + 2] != p[j + 5] {
                continue;
            }
            wires.push([p[j], cy, p[j + 2]]);
            // Mirror all but the ones on the centre line.
            if p[j] != 0.0 {
                wires.push([cx - p[j], cy, p[j + 2]]);
            }
            assert!(wires.len() <= MAX_WIRES);
        }
        assert!(!wires.is_empty());
    }

    let list = g.gen_lists(1);
    g.new_list(list);
    let s = 1.0 / bbox[5];
    g.push_matrix();
    g.scale(s, s, s);
    g.translate(
        -(bbox[0] + bbox[3] / 2.0),
        -(bbox[1] + bbox[4] / 2.0),
        -(bbox[2] + bbox[5] / 2.0),
    );
    for (l, t) in [
        (body.as_ref(), 0),
        (cross.as_ref(), 0),
        (cables.as_ref(), 1),
        (connections.as_ref(), 2),
    ] {
        if let Some(l) = l {
            render_lines(g, m, l, t, wire);
        }
    }
    g.push_matrix();
    g.scale(-1.0, 1.0, 1.0);
    g.front_face_cw(true);
    for (l, t) in [(cross.as_ref(), 0), (cables.as_ref(), 1)] {
        if let Some(l) = l {
            render_lines(g, m, l, t, wire);
        }
    }
    g.pop_matrix();
    g.pop_matrix();
    g.end_list();
    g.front_face_cw(false);

    Tower {
        list,
        bbox,
        wires,
        has_connections: model.connections.is_some(),
    }
}

impl HighVoltage {
    /// The projection is a frustum rather than a plain perspective so that the
    /// vanishing point can sit at the edge of the window on the side the line
    /// of towers runs off to.
    fn set_projection(&self, g: &mut Gl) {
        let (mut height, width) = (self.height, self.width);
        let mut y = 0;
        let mut h = height as f32 / width as f32;
        if width > height * 5 {
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();

        let fovy = 30.0f32;
        let aspect = 1.0 / h;
        let near = 1.0;
        let far = 28.0 * self.spacing * self.count as f32 * 15.0;
        let fh = (fovy / 360.0 * std::f32::consts::PI).tan() * near;
        let (mut fw1, mut fw2) = (-fh * aspect, fh * aspect);
        if self.left {
            fw1 *= 2.0;
            fw2 = 0.0;
        } else {
            fw1 = 0.0;
            fw2 *= 2.0;
        }
        // The horizon sits below the middle of the window.
        g.glx.frustum(fw1, fw2, -fh * 0.6, fh, near, far);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        // Look up, not at the horizon.
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 3.0, 0.0], [0.0, 1.0, 0.0]);
        let s = if self.width < self.height {
            self.width as f32 / self.height as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
    }

    /// Pick a different tower, and fly down the other side of the line.
    fn reset(&mut self) {
        let n = self.towers.len();
        let o = self.which;
        while self.which == o {
            self.which = random() as usize % n;
        }
        self.left = !self.left;
        self.objs.clear();
        self.dead = None;
        self.trackball = Trackball::new();
    }

    /// The power lines from this tower back to the one behind it, sagging
    /// under their own weight.
    fn draw_wires(&self, g: &mut Glx, p: &Obj) -> i32 {
        let t = &self.towers[self.which];
        let cy = t.bbox[1] + t.bbox[4] / 2.0;
        let cz = t.bbox[2] + t.bbox[5] / 2.0;
        let diam = 0.002;
        let droop = 0.15;
        let segments = 20;
        let mut polys = 0;
        for w in &t.wires {
            let from = [
                w[0] / t.bbox[5],
                (w[1] - cy) / t.bbox[5],
                (w[2] - cz) / t.bbox[5],
            ];
            let to = [from[0] + p.pos[0], from[1] + p.pos[1], from[2] + p.pos[2]];
            let mut prev = [0.0f32; 3];
            for j in 0..=segments {
                let r = j as f32 / segments as f32;
                let off = (std::f32::consts::PI * r).sin() * droop;
                let cur = [
                    from[0] + (to[0] - from[0]) * r,
                    from[1] + (to[1] - from[1]) * r,
                    from[2] + (to[2] - from[2]) * r - off,
                ];
                if j > 0 {
                    if self.wire {
                        g.begin(Shape::Lines);
                        g.vertex3f(prev[0], prev[1], prev[2]);
                        g.vertex3f(cur[0], cur[1], cur[2]);
                        g.end();
                        polys += 1;
                    } else {
                        polys += self.meshes.line.draw(g, prev, cur, diam, 0.0);
                    }
                }
                prev = cur;
            }
        }
        polys
    }

    fn draw_objs(&mut self, g: &mut Glx) {
        let r = self.ratio;
        let lerp = |a: [f32; 3], b: [f32; 3]| {
            [
                a[0] + (b[0] - a[0]) * r,
                a[1] + (b[1] - a[1]) * r,
                a[2] + (b[2] - a[2]) * r,
            ]
        };
        let cur = Obj {
            pos: lerp(self.from.pos, self.to.pos),
            rot: lerp(self.from.rot, self.to.rot),
        };

        g.rotate(cur.rot[0] * 360.0, 1.0, 0.0, 0.0);
        g.rotate(cur.rot[1] * 360.0, 0.0, 1.0, 0.0);
        g.rotate(cur.rot[2] * 360.0, 0.0, 0.0, 1.0);
        g.translate(cur.pos[0], cur.pos[1], cur.pos[2]);

        let list = self.towers[self.which].list;

        // The tower just passed, still visible behind the camera.
        if let Some(o) = self.dead {
            g.push_matrix();
            g.rotate(o.rot[0] * 360.0, 1.0, 0.0, 0.0);
            g.rotate(o.rot[1] * 360.0, 0.0, 1.0, 0.0);
            g.rotate(o.rot[2] * 360.0, 0.0, 0.0, 1.0);
            g.translate(o.pos[0], o.pos[1], o.pos[2]);
            g.call_list(list);
            g.pop_matrix();
        }

        // Each tower is placed relative to the one in front of it, so the
        // matrix walks off into the distance as the list is drawn.
        let mut prev: Option<Obj> = None;
        for i in 0..self.objs.len() {
            let o = self.objs[i];
            g.call_list(list);
            if let Some(p) = prev {
                self.draw_wires(g, &p);
            }
            if i + 1 == self.objs.len() {
                break;
            }
            g.translate(-o.pos[0], -o.pos[1], -o.pos[2]);
            g.rotate(-o.rot[2] * 360.0, 0.0, 0.0, 1.0);
            g.rotate(-o.rot[1] * 360.0, 0.0, 1.0, 0.0);
            g.rotate(-o.rot[0] * 360.0, 1.0, 0.0, 0.0);
            prev = Some(o);
        }
    }

    fn tick_objs(&mut self) {
        while self.objs.len() < self.count.max(1) as usize {
            // A tower off to one side, and mostly the same side: the line of
            // them should wander rather than zigzag.
            let mut sign = if random().is_multiple_of(10) {
                1.0
            } else {
                -1.0
            };
            if self.left {
                sign = -sign;
            }
            self.objs.push_back(Obj {
                pos: [
                    sign * frand(1.5) as f32,
                    -20.0 * (1.0 + frand(0.2) as f32) * self.spacing,
                    0.0,
                ],
                rot: [0.0; 3],
            });
        }

        self.ratio += 0.0015 * self.speed; // Flight speed
        if self.ratio > 1.0 {
            // The first tower has arrived: drop it behind the camera and aim
            // at the next one.
            self.ratio = 0.0;
            self.dead = self.objs.pop_front();
        }
        self.to = self.objs[0];
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let meshes = Meshes::new();
    let towers: Vec<Tower> = tower_models()
        .iter()
        .map(|m| render_tower(&mut g.glx, &meshes, m, wire))
        .collect();

    let bg = resource_color(g, "background");
    g.glx.clear_color(bg[0], bg[1], bg[2], bg[3]);

    let mut this = HighVoltage {
        trackball: Trackball::new(),
        towers,
        meshes,
        which: usize::MAX,
        objs: VecDeque::new(),
        dead: None,
        from: Obj::default(),
        to: Obj::default(),
        ratio: 0.0,
        state: State::FadeIn,
        left: random().is_multiple_of(2),
        tick: 0.0,
        fg: resource_color(g, "foreground"),
        bg,
        width: g.width(),
        height: g.height(),
        count: g.res.int("count"),
        speed: g.res.float("speed") as f32,
        spacing: g.res.float("spacing") as f32,
        wire,
    };
    this.which = random() as usize % this.towers.len();
    Box::new(this)
}

impl Hack3d for HighVoltage {
    fn reshape(&mut self, _g: &mut Gl, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event {
            match key {
                ' ' | '\t' | '\r' | '\n' => {
                    self.state = State::FadeOut;
                    return true;
                }
                '>' | '.' | '+' | '=' => {
                    self.speed += 0.1;
                    return true;
                }
                '<' | ',' | '-' | '_' | '\u{8}' | '\u{7f}' => {
                    self.speed = (self.speed - 0.1).max(0.1);
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.set_projection(g);
        g.glx.clear();
        // Not one of depth testing, culling or lighting: the towers are a flat
        // drawing, and the far ones are meant to paint over the near ones.
        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.lighting(false);
        if !self.wire {
            g.glx.fog(Some(Fog::Exp2 {
                density: 0.001,
                color: [1.0, 1.0, 1.0, 1.0],
            }));
            g.glx.blend(Blend::Alpha);
        }

        // Change it up every ten minutes or so.
        if random().is_multiple_of(30 * 60 * 10) {
            self.state = State::FadeOut;
        }
        match self.state {
            State::FadeIn => {
                self.tick += 0.01 * self.speed;
                if self.tick >= 1.0 {
                    self.tick = 1.0;
                    self.state = State::Draw;
                }
            }
            State::Draw => {}
            State::FadeOut => {
                self.tick -= 0.05 * self.speed;
                if self.tick <= 0.0 {
                    self.tick = 0.0;
                    self.state = State::FadeIn;
                    self.reset();
                }
            }
        }

        g.glx.push_matrix();
        g.glx.scale(15.0, 15.0, 15.0);
        g.glx.mult_matrix(self.trackball.matrix());
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        // The tower feet sit a little below the horizon plane.
        g.glx.translate(0.0, 0.0, 0.4);
        // Close enough in that the wires do not drop out of the window.
        g.glx.translate(0.0, -4.0 * self.spacing, 0.0);
        // Off the centre line.
        let side = if self.left { -1.0 } else { 1.0 };
        g.glx.translate(0.6 * side, 0.0, 0.0);
        if self.towers[self.which].has_connections {
            // A street pole is looked at from further round, with its low
            // voltage wires pointing at the nearer edge of the window.
            g.glx.translate(0.6 * side, 0.0, 0.0);
            if self.left {
                g.glx.scale(-1.0, 1.0, 1.0);
            }
        }

        // Everything is one colour, and it is the fade.
        let t = self.tick;
        let c = [
            self.bg[0] * (1.0 - t) + self.fg[0] * t,
            self.bg[1] * (1.0 - t) + self.fg[1] * t,
            self.bg[2] * (1.0 - t) + self.fg[2] * t,
            self.fg[3],
        ];
        g.glx.color4f(c[0], c[1], c[2], c[3]);
        g.glx.material_ambient_diffuse(c);

        if !self.trackball.button_down() {
            self.tick_objs();
        }
        let glx = &mut g.glx;
        self.draw_objs(glx);

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:      30000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*count:      7",
    ".background: #FFFFCC",
    ".foreground: #444422",
    "*speed:      1.0",
    "*spacing:    1.0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 8.0, 0.01, 2, "1.0"),
    Opt::slider("spacing", "Spacing", 0.1, 5.0, 0.1, 1, "1.0"),
    Opt::slider("count", "Number of towers", 1.0, 40.0, 1.0, 0, "7"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "highvoltage",
    label: "High Voltage",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2024",
        video: Some("https://www.youtube.com/watch?v=KG8Zy2Kf7bc"),
        blurb: "A leisurely flight past some high voltage transmission towers. \
                Smell the ozone!",
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

    /// Ten towers, all of them traced as lines rather than modelled, except
    /// the wooden street pole, which is solid.
    #[test]
    fn the_towers_are_lines() {
        let models = tower_models();
        assert_eq!(models.len(), 10);
        let mut lines = 0;
        for m in &models {
            for s in [m.body, m.cross, m.cables, m.connections]
                .into_iter()
                .flatten()
            {
                let l = GlList::parse(s);
                if l.primitive == Shape::Lines {
                    assert_eq!(l.format, Format::V3f);
                    lines += 1;
                }
            }
        }
        assert_eq!(lines, 27, "{lines} of the 28 parts are lines");
        // Two towers have no body of their own and one has no cross-arm.
        assert_eq!(models.iter().filter(|m| m.body.is_none()).count(), 2);
        assert_eq!(models.iter().filter(|m| m.cross.is_none()).count(), 1);
    }

    /// Every tower's power lines have somewhere to hang from, and there are
    /// always an even number of them either side of the middle or on it.
    #[test]
    fn every_tower_has_attachment_points() {
        let mut g = Glx::new();
        let meshes = Meshes::new();
        let towers: Vec<Tower> = tower_models()
            .iter()
            .map(|m| render_tower(&mut g, &meshes, m, false))
            .collect();
        for (i, t) in towers.iter().enumerate() {
            assert!(!t.wires.is_empty(), "tower {i} has nowhere to hang a wire");
            assert!(t.wires.len() <= MAX_WIRES);
            assert!(t.bbox[5] > 0.0, "tower {i} is flat");
        }
    }

    /// The line of towers walks off into the distance: each is placed relative
    /// to the one in front of it, so the far ones are a long way back.
    #[test]
    fn the_towers_recede() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let depths: Vec<f32> = f.batches.iter().map(|b| b.modelview.0[14]).collect();
        let near = depths.iter().copied().fold(f32::MAX, f32::min);
        let far = depths.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            far - near > 100.0,
            "the towers span only {} in depth",
            far - near
        );
        assert!(f.batches.len() >= 7, "only {} draws", f.batches.len());
    }

    /// It fades up from the background colour rather than appearing, and the
    /// whole drawing is that one colour.
    #[test]
    fn it_fades_up_from_the_ground() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let first = r.frame().vertices[0].color;
        let bg = [1.0, 1.0, 0.8];
        assert!(
            (0..3).all(|i| (first[i] - bg[i]).abs() < 0.05),
            "it started dark: {first:?}"
        );
        for _ in 0..150 {
            r.step();
        }
        let f = r.frame();
        let later = f.vertices[0].color;
        assert!(later[0] < 0.4, "it never came up: {later:?}");
        assert!(
            f.vertices.iter().all(|v| v.color == later),
            "the drawing is not all one colour"
        );
    }

    /// A tower is one draw call, not six hundred. Every tube in it goes
    /// through [`TubeMesh`] rather than the matrix stack for exactly this
    /// reason: with `tube` it was six thousand batches a frame, which is more
    /// than the whole runtime can afford.
    #[test]
    fn a_tower_is_one_draw() {
        let mut r = start(StartArgs::new(1920, 1080, "", 20260811));
        for _ in 0..30 {
            r.step();
        }
        let f = r.frame();
        assert!(f.batches.len() < 40, "{} batches", f.batches.len());
        assert!(f.vertices.len() < 250_000, "{} vertices", f.vertices.len());
    }

    /// Nothing is lit, depth-tested or culled, and the fog is what gives the
    /// line of towers its depth.
    #[test]
    fn it_is_a_flat_drawing_in_fog() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        assert!(f.batches.iter().all(|b| !b.lighting), "something is lit");
        assert!(f.batches.iter().all(|b| !b.depth_test), "depth is tested");
        assert!(
            f.batches
                .iter()
                .all(|b| matches!(b.fog, Some(Fog::Exp2 { .. }))),
            "the fog is missing"
        );
    }
}
