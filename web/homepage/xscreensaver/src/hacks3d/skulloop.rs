//! Port of `hacks/glx/skulloop.c`.
//!
//! ```text
//! skulloop, Copyright © 2023-2025 Jamie Zawinski <jwz@jwz.org>
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
//! A skull with a skull in its mouth, and a skull in that one's mouth, going
//! in for ever. The camera falls forward into the chain and never arrives: as
//! each skull passes it is dropped off the front and a new one is added at the
//! back, so the loop is five deep however long you watch it.
//!
//! Where the next skull sits is picked from upstream's list of places a skull
//! can be, which are named for where the joke comes from: through an eye
//! socket, out of the nose, behind the head, or replacing a tooth. Every jaw
//! chatters on its own, and one in twenty has no jaw at all.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gllist::GlList;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

const SKULL_HALF: usize = 0;
const JAW_HALF: usize = 1;
const TEETH_UPPER_HALF: usize = 2;
const TEETH_LOWER_HALF: usize = 3;

const MODELS: [&str; 4] = [
    crate::models::SKULL_MODEL_SKULL_HALF,
    crate::models::SKULL_MODEL_JAW_HALF,
    crate::models::SKULL_MODEL_TEETH_UPPER_HALF,
    crate::models::SKULL_MODEL_TEETH_LOWER_HALF,
];

/// How far the jaw can drop.
const JAW_MAX: f32 = 22.0;

/// One skull in the chain.
#[derive(Clone, Default)]
struct Obj {
    id: u32,
    pos: [f32; 3],
    rot: [f32; 3],
    scale: f32,
    /// How far the jaw is open, or negative for a skull with no jaw at all.
    jaw_pos: f32,
    chatter: f32,
}

struct Skulloop {
    trackball: Trackball,
    lists: Vec<u32>,
    colors: Vec<[f32; 4]>,
    objs: Vec<Obj>,
    /// The one that has just passed the camera, still drawn behind it.
    dead: Option<Obj>,
    from: Obj,
    to: Obj,
    ratio: f32,
    last_id: u32,
    aspect: f32,
    scale: f32,
    speed: f32,
    length: f32,
    wire: bool,
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

impl Skulloop {
    /// Each model is half a skull; the other half is the same one mirrored.
    fn draw_component(&self, g: &mut Gl, i: usize) {
        // A display list here replays geometry and not state, so the colour
        // goes on where it is called.
        g.glx.material_ambient_diffuse(self.colors[i]);
        g.glx.material_specular([0.4, 0.4, 0.4, 1.0]);
        g.glx.material_shininess(80.0);
        g.glx.front_face_cw(false);
        g.glx.call_list(self.lists[i]);
        g.glx.push_matrix();
        g.glx.scale(-1.0, 1.0, 1.0);
        // The model seems to have a gap.
        g.glx.translate(-0.05, 0.0, 0.0);
        g.glx.front_face_cw(true);
        g.glx.call_list(self.lists[i]);
        g.glx.pop_matrix();
        g.glx.front_face_cw(false);
    }

    /// `draw_skull`: the cranium, then the jaw hinged where a jaw hinges.
    fn draw_skull(&self, g: &mut Gl, o: &Obj) {
        let head_base = [0.0f32, 200.0, 0.0];
        let jaw_base = [0.0f32, 270.0, 40.0];
        g.glx.rotate(0.0, 1.0, 0.0, 0.0);
        g.glx.translate(-head_base[0], -head_base[1], -head_base[2]);
        self.draw_component(g, SKULL_HALF);
        if o.jaw_pos >= -1.0 {
            self.draw_component(g, TEETH_UPPER_HALF);
        }
        if o.jaw_pos >= 0.0 {
            g.glx.translate(jaw_base[0], jaw_base[1], jaw_base[2]);
            g.glx.rotate(o.jaw_pos, 1.0, 0.0, 0.0);
            g.glx.translate(-jaw_base[0], -jaw_base[1], -jaw_base[2]);
            self.draw_component(g, JAW_HALF);
            self.draw_component(g, TEETH_LOWER_HALF);
        }
    }

    fn draw_obj(&self, g: &mut Gl, o: &Obj) {
        let s = 0.005;
        g.glx.push_matrix();
        g.glx.translate(0.0, -0.5, -0.2);
        g.glx.scale(s, s, s);
        self.draw_skull(g, o);
        g.glx.pop_matrix();
    }

    /// `draw_objs`: the chain, drawn by walking into it. Each skull's place
    /// is relative to the one it sits in, so the matrix is never popped: it
    /// accumulates all the way down.
    fn draw_objs(&self, g: &mut Gl) {
        let r = ease(Ease::InOutSine, self.ratio as f64) as f32;
        let lerp = |a: f32, b: f32| a + (b - a) * r;
        let cur_pos: [f32; 3] = std::array::from_fn(|k| lerp(self.from.pos[k], self.to.pos[k]));
        let cur_rot: [f32; 3] = std::array::from_fn(|k| lerp(self.from.rot[k], self.to.rot[k]));
        let cur_scale = lerp(self.from.scale, self.to.scale);

        let s = 1.0 / cur_scale;
        g.glx.scale(s, s, s);
        g.glx.rotate(cur_rot[0] * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(cur_rot[1] * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(cur_rot[2] * 360.0, 0.0, 0.0, 1.0);
        g.glx.translate(cur_pos[0], cur_pos[1], cur_pos[2]);

        // The one that has just gone past is drawn behind the camera, which
        // is what stops it popping out of existence.
        if let Some(o) = &self.dead {
            g.glx.push_matrix();
            let s = 1.0 / o.scale;
            g.glx.scale(s, s, s);
            g.glx.rotate(o.rot[0] * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(o.rot[1] * 360.0, 0.0, 1.0, 0.0);
            g.glx.rotate(o.rot[2] * 360.0, 0.0, 0.0, 1.0);
            g.glx.translate(o.pos[0], o.pos[1], o.pos[2]);
            self.draw_obj(g, o);
            g.glx.pop_matrix();
        }

        for (i, o) in self.objs.iter().enumerate() {
            self.draw_obj(g, o);
            if i + 1 == self.objs.len() {
                break;
            }
            // Into the next one's mouth, which is the same transform
            // backwards.
            g.glx.translate(-o.pos[0], -o.pos[1], -o.pos[2]);
            g.glx.rotate(-o.rot[2] * 360.0, 0.0, 0.0, 1.0);
            g.glx.rotate(-o.rot[1] * 360.0, 0.0, 1.0, 0.0);
            g.glx.rotate(-o.rot[0] * 360.0, 1.0, 0.0, 0.0);
            g.glx.scale(o.scale, o.scale, o.scale);
        }
    }

    /// Where the next skull goes. The names are upstream's.
    fn new_obj(&mut self) -> Obj {
        let mut o = Obj {
            jaw_pos: frand(20.0) as f32,
            chatter: frand(1.0) as f32 * self.speed,
            ..Obj::default()
        };
        if random().is_multiple_of(3) {
            o.chatter *= 5.0;
        }
        let n = if self.last_id < 2 {
            0.66
        } else {
            frand(1.0) as f32
        };

        if n < 0.50 {
            // Corinthian: through an eye socket.
            o.scale = 0.15 * (1.0 + frand(0.2) as f32);
            o.pos = [
                0.22 * if random() & 1 == 1 { 1.0 } else { -1.0 },
                -0.03,
                -0.4,
            ];
            o.rot[2] = 0.2 - frand(0.4) as f32;
            if random().is_multiple_of(10) {
                o.rot[2] = frand(0.5) as f32 - 1.0;
            }
        } else if n < 0.65 {
            // Nose.
            o.scale = 0.15 * (1.0 + frand(0.2) as f32);
            o.pos = [0.0, 0.12, -0.55];
        } else if n < 0.80 {
            // Zeiram (1991).
            o.scale = 0.15 * (1.0 + frand(0.2) as f32);
            o.pos = [0.0, -0.27, -0.53];
            o.rot[0] = 0.1;
        } else if n < 0.85 {
            // Malignant (2021): round the back of the head.
            o.scale = 1.0;
            o.pos = [0.0, 0.0, 0.4];
            o.rot[1] = 0.5;
            if random().is_multiple_of(10) {
                o.rot[2] = -0.5;
            }
        } else if n < 0.97 {
            // Grille: in place of a tooth.
            o.scale = 0.067;
            o.pos[1] = 0.40;
            if random() & 1 == 1 {
                // Incisor.
                o.pos[0] = 0.028 * if random() & 1 == 1 { 1.0 } else { -1.0 };
                o.pos[2] = -0.565;
            } else {
                // Lateral.
                o.pos[0] = 0.078 * if random() & 1 == 1 { 1.0 } else { -1.0 };
                o.rot[1] = 0.09 * if o.pos[0] > 0.0 { 1.0 } else { -1.0 };
                o.pos[2] = -0.56;
            }
        } else {
            // Cymothoa exigua: sitting on the tongue with the mouth open.
            o.scale = 0.28;
            o.pos = [0.0, 0.42, -0.28];
            o.jaw_pos = JAW_MAX;
            o.chatter = 0.0;
        }

        // One in twenty has no jaw, and one in seven of those has no upper
        // teeth either.
        if random().is_multiple_of(20) {
            o.jaw_pos = -1.0;
            if random().is_multiple_of(7) {
                o.jaw_pos = -2.0;
            }
        }
        self.last_id += 1;
        o.id = self.last_id;
        for k in 0..3 {
            if o.rot[k] > 0.5 {
                o.rot[k] = 1.0 - o.rot[k];
            }
        }
        o
    }

    /// `tick_objs`: chatter the jaws, top the chain back up, and drop the
    /// front one once the camera has gone past it.
    fn tick_objs(&mut self) {
        for o in &mut self.objs {
            if o.jaw_pos >= 0.0 {
                o.jaw_pos += o.chatter;
                if o.jaw_pos < 0.0 {
                    o.jaw_pos = 0.0;
                    o.chatter = -o.chatter;
                } else if o.jaw_pos > JAW_MAX {
                    o.jaw_pos = JAW_MAX;
                    o.chatter = -o.chatter;
                }
            }
        }
        while (self.objs.len() as f32) < self.length {
            let o = self.new_obj();
            self.objs.push(o);
        }

        self.ratio += 0.02 * self.speed;
        if self.ratio > 1.0 {
            // The first one has arrived at the camera.
            self.ratio = 0.0;
            self.dead = Some(self.objs.remove(0));
            self.from.scale = 1.0;
        }
        if let Some(first) = self.objs.first() {
            self.to = first.clone();
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let mut lists = Vec::new();
    let mut colors = Vec::new();
    for (i, src) in MODELS.iter().enumerate() {
        colors.push(resource_color(
            g,
            if i == SKULL_HALF || i == JAW_HALF {
                "skullColor"
            } else {
                "teethColor"
            },
        ));
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

    let mut this = Skulloop {
        trackball: Trackball::new(),
        lists,
        colors,
        objs: Vec::new(),
        dead: None,
        // At start-up the camera zooms in from a point.
        from: Obj {
            scale: 100.0,
            ..Obj::default()
        },
        to: Obj::default(),
        ratio: 0.0,
        last_id: 0,
        aspect: 1.0,
        scale: 1.0,
        speed: g.res.float("speed") as f32,
        length: (g.res.float("length") as f32).max(2.0),
        wire,
    };
    this.tick_objs();

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Skulloop {
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
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.lighting(!self.wire);
        g.glx.color_material(self.wire);
        if !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.4, 0.7, 0.4, 0.0);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        }
        if self.wire {
            g.glx.color4f(0.5, 1.0, 0.5, 1.0);
        }

        g.glx.push_matrix();
        g.glx.mult_matrix(self.trackball.matrix());
        // Hide the glitchy left/right model seam.
        g.glx.rotate(-2.0, 0.0, 1.0, 0.0);
        g.glx.scale(16.0, 16.0, 16.0);
        g.glx.translate(0.0, 0.08, 0.0);

        if !self.trackball.button_down() {
            self.tick_objs();
        }
        self.draw_objs(g);
        g.glx.pop_matrix();

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       30000",
    "*showFPS:     False",
    "*wireframe:   False",
    "*skullColor:  #777777",
    "*teethColor:  #FFFFFF",
    "*speed:       1.0",
    "*length:      5",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 5.0, 0.1, 1, "1.0"),
    Opt::slider("length", "Skulls", 2.0, 12.0, 1.0, 0, "5"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "skulloop",
    label: "Skulloop",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2023",
        video: Some("https://www.youtube.com/watch?v=qyVAs8iMV6k"),
        blurb: "A skull with a skull in its mouth, and so on, for ever.",
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

    /// Four models, each half a skull.
    #[test]
    fn a_skull_is_four_halves() {
        for (i, src) in MODELS.iter().enumerate() {
            let m = GlList::parse(src);
            assert!(m.points > 1000, "part {i} is only {} vertices", m.points);
            assert_eq!(m.primitive, crate::runtime::gl::Shape::Triangles);
        }
    }

    /// The chain stays the length it was asked for, however long it runs.
    #[test]
    fn the_loop_never_runs_out() {
        for length in [2.0f32, 5.0, 9.0] {
            let mut r = start(StartArgs::new(
                640,
                480,
                &format!("length={length}&speed=5"),
                20260811,
            ));
            for _ in 0..300 {
                r.step();
                let f = r.frame();
                assert!(!f.batches.is_empty(), "the loop emptied at {length}");
                assert!(
                    f.vertices
                        .iter()
                        .all(|v| v.pos.iter().all(|c| c.is_finite())),
                    "a vertex went to NaN"
                );
            }
        }
    }

    /// Every skull sits inside the one before it, so each is smaller than the
    /// last by its own scale.
    #[test]
    fn each_skull_is_deeper_in_than_the_last() {
        let mut r = start(StartArgs::new(640, 480, "length=5", 20260811));
        r.step();
        let f = r.frame();
        // The batches come out in chain order, and the matrices shrink.
        let scales: Vec<f32> = f
            .batches
            .iter()
            .map(|b| {
                let m = b.modelview.0;
                (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt()
            })
            .collect();
        assert!(scales.len() > 4, "only {} draws", scales.len());
        let first = scales[0];
        let last = scales[scales.len() - 1];
        assert!(
            last < first,
            "the chain does not shrink: {first} then {last}"
        );
    }

    /// A jaw swings between shut and its stop, and reverses at both.
    #[test]
    fn the_jaws_chatter() {
        let mut o = Obj {
            jaw_pos: 0.0,
            chatter: 3.0,
            ..Obj::default()
        };
        let mut seen_open = false;
        let mut seen_shut = false;
        for _ in 0..200 {
            if o.jaw_pos >= 0.0 {
                o.jaw_pos += o.chatter;
                if o.jaw_pos < 0.0 {
                    o.jaw_pos = 0.0;
                    o.chatter = -o.chatter;
                    seen_shut = true;
                } else if o.jaw_pos > JAW_MAX {
                    o.jaw_pos = JAW_MAX;
                    o.chatter = -o.chatter;
                    seen_open = true;
                }
            }
            assert!(
                (0.0..=JAW_MAX).contains(&o.jaw_pos),
                "the jaw came off at {}",
                o.jaw_pos
            );
        }
        assert!(seen_open && seen_shut, "it never went both ways");
    }
}
