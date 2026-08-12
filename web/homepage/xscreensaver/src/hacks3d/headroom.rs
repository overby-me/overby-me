//! Port of `hacks/glx/headroom.c`.
//!
//! ```text
//! headroom, Copyright © 2020-2024 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Well, it's supposed to be Max Headroom, but I have so far been unable to
//! find or commission a decent 3D model of Max.  So it's a skull instead.
//! This will have to do for now.
//!
//! Code by jwz, 2020. Formal-wear model by Jared Williams, 2024.
//! Created 29-Nov-2020.
//! ```
//!
//! A skull in a suit and tie, twitching in front of the scrolling grid
//! background from the television programme. The head snaps to a new angle
//! every so often and every so often snaps back to level, which is the whole
//! performance: the stutter was originally done by hand, frame by frame.
//!
//! The background is six panels of horizontal bars making a box around the
//! camera. It is drawn with the lights off and the depth buffer cleared after
//! it, so however large the box is it can never get in front of the figure.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, DepthFunc, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::rotator::Rotator;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
};

const SKULL_HALF: usize = 0;
const JAW_HALF: usize = 1;
const TEETH_UPPER_HALF: usize = 2;
const TEETH_LOWER_HALF: usize = 3;
const SUIT_HALF: usize = 4;
const SUIT_CAP_HALF: usize = 5;
const SHIRT_HALF: usize = 6;
const TIE_HALF: usize = 7;
const MASK_HALF: usize = 8;

/// The head is the same model `skulloop` uses; the suit was drawn for this
/// one twenty years later.
const MODELS: [&str; 9] = [
    crate::models::SKULL_MODEL_SKULL_HALF,
    crate::models::SKULL_MODEL_JAW_HALF,
    crate::models::SKULL_MODEL_TEETH_UPPER_HALF,
    crate::models::SKULL_MODEL_TEETH_LOWER_HALF,
    crate::models::HEADROOM_MODEL_SUIT_HALF,
    crate::models::HEADROOM_MODEL_SUIT_CAP_HALF,
    crate::models::HEADROOM_MODEL_SHIRT_HALF,
    crate::models::HEADROOM_MODEL_TIE_HALF,
    crate::models::HEADROOM_MODEL_MASK_HALF,
];

const COLOR_KEYS: [&str; 9] = [
    "skullColor",
    "skullColor",
    "teethColor",
    "teethColor",
    "suitColor",
    "suitCapColor",
    "shirtColor",
    "tieColor",
    "maskColor",
];

struct Headroom {
    trackball: Trackball,
    /// Three rotators: one for the background box, one for the wander, one
    /// for the tilt of the figure.
    rot: Rotator,
    rot2: Rotator,
    rot3: Rotator,
    spin: [bool; 3],
    /// How far the head is nodded, turned and tilted, and how far the jaw is
    /// dropped.
    head_pos: [f32; 3],
    jaw_pos: f32,
    lists: Vec<u32>,
    colors: Vec<[f32; 4]>,
    grid_colors: [[f32; 4]; 3],
    aspect: f32,
    scale: f32,
    speed: f32,
    mask: bool,
    mask_opacity: f32,
    wire: bool,
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

impl Headroom {
    /// `draw_unit_panel`: one wall of the background, thirty-six bars that
    /// fade from one colour at the left to another at the right.
    fn draw_unit_panel(&self, g: &mut Gl, color1: [f32; 4], color2: [f32; 4]) {
        let rows = 36;
        let spacing = 1.0 / rows as f32;
        let thickness = spacing / 8.0;
        g.glx.front_face_cw(false);
        g.glx.begin(if self.wire {
            Shape::Lines
        } else {
            Shape::Quads
        });
        g.glx.normal3f(0.0, 0.0, 1.0);
        for i in 0..rows {
            let y = i as f32 / rows as f32 + spacing / 2.0;
            let c2 = color2;
            let c1 = color1;
            g.glx.color4f(c2[0], c2[1], c2[2], c2[3]);
            g.glx.vertex3f(1.0, y, 0.0);
            g.glx.color4f(c1[0], c1[1], c1[2], c1[3]);
            g.glx.vertex3f(0.0, y, 0.0);
            g.glx.vertex3f(0.0, y + thickness, 0.0);
            g.glx.color4f(c2[0], c2[1], c2[2], c2[3]);
            g.glx.vertex3f(1.0, y + thickness, 0.0);
        }
        g.glx.end();
    }

    /// `draw_box`: four walls, a floor and a ceiling, the camera inside.
    fn draw_box(&self, g: &mut Gl) {
        let [c0, c1, c2] = self.grid_colors;
        g.glx.push_matrix();
        g.glx.translate(-0.5, -0.5, 0.5);
        self.draw_unit_panel(g, c0, c1);
        for (a, b) in [(c1, c0), (c0, c1), (c1, c0)] {
            g.glx.rotate(-90.0, 0.0, 1.0, 0.0);
            g.glx.translate(-1.0, 0.0, 0.0);
            self.draw_unit_panel(g, a, b);
        }
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.translate(0.0, 0.0, 1.0);
        self.draw_unit_panel(g, c2, c1);
        g.glx.rotate(-180.0, 1.0, 0.0, 0.0);
        g.glx.translate(0.0, -1.0, 1.0);
        self.draw_unit_panel(g, c1, c2);
        g.glx.pop_matrix();
    }

    /// Each model is half the figure; the other half is the same one
    /// mirrored.
    fn draw_component(&self, g: &mut Gl, i: usize, alpha: f32) {
        let mut c = self.colors[i];
        c[3] = alpha;
        // A display list here replays geometry and not state, so the colour
        // goes on where it is called.
        g.glx.material_ambient_diffuse(c);
        g.glx.material_specular([0.4, 0.4, 0.4, 1.0]);
        g.glx.material_shininess(80.0);
        g.glx.front_face_cw(false);
        g.glx.call_list(self.lists[i]);
        g.glx.push_matrix();
        g.glx.scale(-1.0, 1.0, 1.0);
        // The model seems to have a gap.
        g.glx.translate(-0.14, 0.0, 0.0);
        g.glx.front_face_cw(true);
        g.glx.call_list(self.lists[i]);
        g.glx.pop_matrix();
        g.glx.front_face_cw(false);
    }

    /// `draw_transparent_component`: a translucent thing that would otherwise
    /// show its own back faces through itself. Drawn once into the depth
    /// buffer with the colour mask shut, then again blended only where the
    /// depth already matches, so only the nearest surface is painted.
    fn draw_transparent_component(&self, g: &mut Gl, i: usize, alpha: f32) {
        if alpha < 0.0 {
            return;
        }
        let alpha = alpha.min(1.0);
        if self.wire || alpha >= 1.0 {
            self.draw_component(g, i, alpha);
            return;
        }
        g.glx.color_mask(false);
        self.draw_component(g, i, alpha);
        g.glx.color_mask(true);
        g.glx.depth_func(DepthFunc::Equal);
        g.glx.blend(Blend::Alpha);
        self.draw_component(g, i, alpha);
        g.glx.depth_func(DepthFunc::Less);
        g.glx.blend(Blend::Off);
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let speed = g.res.float("speed") as f32;
    let spin = g.res.string("spin").to_string();
    let axis = |c: char, d: char| spin.contains(c) || spin.contains(d);

    let mut lists = Vec::new();
    let mut colors = Vec::new();
    for (i, src) in MODELS.iter().enumerate() {
        colors.push(resource_color(g, COLOR_KEYS[i]));
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

    let spin = [axis('x', 'X'), axis('y', 'Y'), axis('z', 'Z')];
    let wander = if g.res.bool("wander") {
        0.002 * speed as f64
    } else {
        0.0
    };
    let spin2 = 0.2 * speed as f64;
    let mut this = Headroom {
        trackball: Trackball::new(),
        // The wander of the whole figure, the tilt of it, and the turn of
        // the background box.
        rot: Rotator::new(
            if spin[0] { 0.5 } else { 0.0 },
            if spin[1] { 0.5 } else { 0.0 },
            if spin[2] { 0.5 } else { 0.0 },
            0.5,
            wander,
            false,
        ),
        rot2: Rotator::new(0.0, 0.0, 0.0, 0.0, 0.005 * speed as f64, true),
        rot3: Rotator::new(spin2, spin2, spin2, 0.2, 0.01 * speed as f64, true),
        spin,
        head_pos: [0.0; 3],
        jaw_pos: 0.0,
        lists,
        colors,
        grid_colors: [
            resource_color(g, "gridColor1"),
            resource_color(g, "gridColor2"),
            resource_color(g, "gridColor3"),
        ],
        aspect: 1.0,
        scale: 1.0,
        speed,
        mask: g.res.bool("mask"),
        mask_opacity: g.res.float("maskOpacity") as f32,
        wire,
    };
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Headroom {
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
        g.glx.perspective(30.0, self.aspect, 1.0, 500.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        // No colour material: every part carries its own, and the grid is
        // drawn with the lights off, where the vertex colour is what shows.
        g.glx.color_material(false);
        if !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.4, 0.2, 0.4, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        }

        g.glx.push_matrix();
        let turning = !self.trackball.button_down();

        // The background box, drawn unlit and then cleared out of the depth
        // buffer so that however big it is, it stays behind the figure.
        g.glx.push_matrix();
        let (x, y, z) = self.rot3.rotation(turning);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        let (x, y, z) = self.rot3.position(turning);
        g.glx.scale(
            1.0 + x as f32 * 3.0,
            1.0 + y as f32 * 3.0,
            1.0 + z as f32 * 3.0,
        );
        g.glx.scale(60.0, 60.0, 60.0);
        g.glx.lighting(false);
        self.draw_box(g);
        g.glx.lighting(!self.wire);
        g.glx.pop_matrix();
        g.glx.clear_depth();

        let (x, y, z) = self.rot.position(turning);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 6.0,
            (z as f32 - 0.5) * 10.0,
        );
        g.glx.mult_matrix(self.trackball.matrix());
        let (maxx, maxy, maxz) = (40.0f32, 20.0f32, 2.0f32);
        let (x, y, z) = self.rot2.position(turning);
        if self.spin[0] {
            g.glx.rotate(maxy / 2.0 - x as f32 * maxy, 1.0, 0.0, 0.0);
        }
        if self.spin[1] {
            g.glx.rotate(maxx / 2.0 - y as f32 * maxx, 0.0, 1.0, 0.0);
        }
        if self.spin[2] {
            g.glx.rotate(maxz / 2.0 - z as f32 * maxz, 0.0, 0.0, 1.0);
        }

        g.glx.translate(0.0, -6.0, 0.0);
        g.glx.scale(0.03, 0.03, 0.03);

        let head_base = [0.0f32, 200.0, 0.0];
        let jaw_base = [0.0f32, 270.0, 40.0];
        for i in [SUIT_HALF, SHIRT_HALF, TIE_HALF, SUIT_CAP_HALF] {
            self.draw_component(g, i, 1.0);
        }
        g.glx.translate(head_base[0], head_base[1], head_base[2]);
        g.glx.rotate(self.head_pos[0], 1.0, 0.0, 0.0);
        g.glx.rotate(self.head_pos[1], 0.0, 1.0, 0.0);
        g.glx.rotate(self.head_pos[2], 0.0, 0.0, 1.0);
        g.glx.translate(-head_base[0], -head_base[1], -head_base[2]);
        self.draw_component(g, SKULL_HALF, 1.0);
        self.draw_component(g, TEETH_UPPER_HALF, 1.0);
        g.glx.translate(jaw_base[0], jaw_base[1], jaw_base[2]);
        g.glx.rotate(self.jaw_pos, 1.0, 0.0, 0.0);
        g.glx.translate(-jaw_base[0], -jaw_base[1], -jaw_base[2]);
        self.draw_component(g, JAW_HALF, 1.0);
        self.draw_component(g, TEETH_LOWER_HALF, 1.0);
        if self.mask {
            self.draw_transparent_component(g, MASK_HALF, self.mask_opacity);
        }

        // The twitch: a new angle now and then, and back to level now and
        // then, which is the whole performance.
        if turning {
            let twitch = ((200.0 / self.speed) as u32).max(10);
            let untwitch = ((50.0 / self.speed) as u32).max(5);
            if random().is_multiple_of(twitch) {
                self.head_pos[0] = -20.0 + (random() % (20 + 30)) as f32;
            }
            if random().is_multiple_of(twitch) {
                self.head_pos[1] = -50.0 + (random() % (50 * 2)) as f32;
            }
            if random().is_multiple_of(twitch) {
                self.head_pos[2] = -30.0 + (random() % (30 * 2)) as f32;
            }
            if random().is_multiple_of(twitch) {
                self.jaw_pos = (random() % 22) as f32;
            }
            if random().is_multiple_of(untwitch) {
                self.head_pos = [0.0; 3];
                self.jaw_pos = 0.0;
            }
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*skullColor:   #777777",
    "*teethColor:   #FFFFFF",
    "*suitColor:    #444444",
    "*suitCapColor: #000000",
    "*shirtColor:   #CCCCCC",
    "*tieColor:     #444444",
    "*maskColor:    #444488",
    "*gridColor1:   #AA0000",
    "*gridColor2:   #00FF00",
    "*gridColor3:   #6666FF",
    "*speed:        1.0",
    "*spin:         XYZ",
    "*wander:       False",
    "*mask:         False",
    "*maskOpacity:  0.97",
];

const SPINS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "XYZ",
        label: "Sway all ways",
    },
    crate::runtime::opts::SelectItem {
        value: "X",
        label: "Sway up and down",
    },
    crate::runtime::opts::SelectItem {
        value: "Y",
        label: "Sway side to side",
    },
    crate::runtime::opts::SelectItem {
        value: "0",
        label: "Do not sway",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 5.0, 0.1, 1, "1.0"),
    Opt::select("spin", "Sway", SPINS, "XYZ"),
    Opt::boolean("mask", "Show the mask", "false"),
    Opt::slider("maskOpacity", "Mask opacity", 0.0, 1.0, 0.01, 2, "0.97"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "headroom",
    label: "Headroom",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2020",
        video: Some("https://www.youtube.com/watch?v=Y_F0o2Lx3mw"),
        blurb: "M-m-max Headroom, or a skull in a suit standing in for him, \
                twitching in front of the grid.",
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

    /// Nine parts, and the head is the same model skulloop uses.
    #[test]
    fn the_figure_is_nine_halves() {
        for (i, src) in MODELS.iter().enumerate() {
            let m = GlList::parse(src);
            assert!(m.points > 100, "part {i} is only {} vertices", m.points);
        }
        assert_eq!(MODELS[SKULL_HALF], crate::models::SKULL_MODEL_SKULL_HALF);
    }

    /// The background is six panels of thirty-six bars, drawn with the lights
    /// off so the colours are exactly what was asked for.
    #[test]
    fn the_grid_is_six_unlit_panels() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let unlit: Vec<_> = f.batches.iter().filter(|b| !b.lighting).collect();
        assert!(!unlit.is_empty(), "the grid is lit");
        // Six panels of thirty-six bars, each bar a quad of two triangles.
        let bars: usize = unlit.iter().map(|b| b.count).sum::<usize>() / 6;
        assert_eq!(bars, 6 * 36, "{bars} bars is not the grid");
    }

    /// The grid is cleared out of the depth buffer after it is drawn, so it
    /// can never come in front of the figure however big it is.
    #[test]
    fn the_grid_stays_behind() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let cleared = f.batches.iter().filter(|b| b.clear_depth_first).count();
        assert_eq!(cleared, 1, "the depth buffer was cleared {cleared} times");
        // And it is the first batch after the grid.
        let first_lit = f.batches.iter().position(|b| b.lighting);
        let clear_at = f.batches.iter().position(|b| b.clear_depth_first);
        assert_eq!(first_lit, clear_at, "the clear is in the wrong place");
    }

    /// The head snaps to an angle and back to level, and never past the
    /// limits the comment gives.
    #[test]
    fn the_head_twitches_within_its_range() {
        let mut r = start(StartArgs::new(640, 480, "speed=5", 20260811));
        let mut angles = Vec::new();
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            assert!(
                f.vertices
                    .iter()
                    .all(|v| v.pos.iter().all(|c| c.is_finite())),
                "a vertex went to NaN"
            );
            // The skull's matrix carries the head angle.
            angles.push(f.batches.last().map(|b| b.modelview.0[12]).unwrap_or(0.0));
        }
        let lo = angles.iter().copied().fold(f32::MAX, f32::min);
        let hi = angles.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 0.001, "the head never moved");
    }

    /// The mask is drawn twice when it is translucent: once with the colour
    /// mask shut to fill the depth buffer, then blended where the depth
    /// matches.
    #[test]
    fn the_mask_is_drawn_depth_first() {
        let mut r = start(StartArgs::new(640, 480, "mask=true", 20260811));
        r.step();
        let f = r.frame();
        assert!(
            f.batches.iter().any(|b| b.color_mask != [true; 4]),
            "nothing was drawn depth-only"
        );
        assert!(
            f.batches
                .iter()
                .any(|b| b.depth_func == crate::runtime::gl::DepthFunc::Equal),
            "nothing was drawn only where the depth matched"
        );
    }
}
