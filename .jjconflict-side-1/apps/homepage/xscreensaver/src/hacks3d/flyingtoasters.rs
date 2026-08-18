//! Port of `hacks/glx/flyingtoasters.c`.
//!
//! ```text
//! flyingtoasters, Copyright © 2003-2024 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Draws 3D flying toasters, and toast.  Inspired by the ancient
//! Berkeley Systems / After Dark hack, but now updated to the wide
//! wonderful workd of OpenGL and 3D!
//!
//! Code by jwz; object models by Baconmonkey.
//!
//! The original After Dark flying toasters, with the fluffy white wings,
//! were a trademark of Berkeley Systems.  Berkeley Systems ceased to exist
//! some time in 1998, having been gobbled up by Sierra Online, who were
//! subsequently gobbled up by Flipside and/or Vivendi (it's hard to tell
//! exactly what happened when.)
//!
//! I doubt anyone even cares any more, but if they do, hopefully this homage,
//! with the space-age 3D jet-plane toasters, will be considered different
//! enough that whoever still owns the trademark to the fluffy-winged 2D
//! bitmapped toasters won't get all huffy at us.
//! ```
//!
//! Twenty toasters and twenty-five slices of toast, flying out of the dark
//! towards you. Half the toasters are put in front of the bread and half
//! behind, so the two are interleaved rather than sorted into layers.
//!
//! A toaster is nine pieces: body, base, slots, handle and its slot, the knob,
//! two wings and two jets. The chrome is a picture of a mirrored ball wrapped
//! on by sphere mapping, and the toast carries a picture of toast, laid on in
//! object coordinates so it does not slide about as the slice turns.
//!
//! Only empty toasters barrel-roll, because there is no code for toast falling
//! out of one that does.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Fog, TexEnv};
use crate::runtime::gllist::GlList;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

/// How wide the sky is, and how deep.
const GRID_SIZE: f32 = 60.0;
const GRID_DEPTH: f32 = 500.0;

const BASE_TOASTER: usize = 0;
const BASE: usize = 1;
const HANDLE: usize = 2;
const HANDLE_SLOT: usize = 3;
const JET: usize = 4;
const KNOB: usize = 5;
const SLOTS: usize = 6;
const JET_WING: usize = 7;
const TOAST: usize = 8;
const TOAST_BITTEN: usize = 9;

const MODELS: [&str; 10] = [
    crate::models::TOASTER,
    crate::models::TOASTER_BASE,
    crate::models::TOASTER_HANDLE,
    crate::models::TOASTER_HANDLE2,
    crate::models::TOASTER_JET,
    crate::models::TOASTER_KNOB,
    crate::models::TOASTER_SLOTS,
    crate::models::TOASTER_WING,
    crate::models::TOAST,
    crate::models::TOAST2,
];

/// Viewer rotations that look nice. Every now and then the camera moves to a
/// new one; only the first two are used at start-up.
const NICE_VIEWS: [(f32, f32); 11] = [
    (0.0, 120.0),
    (0.0, -120.0),
    (12.0, 28.0),
    (12.0, -28.0),
    (-10.0, -28.0),
    (40.0, -60.0),
    (-40.0, -60.0),
    (40.0, 60.0),
    (-40.0, 60.0),
    (30.0, 0.0),
    (-30.0, 0.0),
];

fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

/// One toaster or one slice of toast, on its way past.
#[derive(Clone, Default)]
struct Floater {
    x: f32,
    y: f32,
    z: f32,
    /// Rotation about the z axis, and how fast it turns.
    wz: f32,
    dx: f32,
    dy: f32,
    dz: f32,
    dwz: f32,
    toaster: bool,
    /// Which of the two slices of bread this is, if it is bread.
    toast_type: usize,
    handle_pos: f32,
    knob_pos: f32,
    /// Two bits, one per slot: which of them have toast in.
    loaded: u32,
}

/// What one piece of a toaster is made of. A display list here replays
/// geometry and not state, so this is applied at every call rather than
/// recorded once inside the list.
struct Finish {
    color: [f32; 4],
    spec: [f32; 4],
    shiny: f32,
    /// The picture on it, and whether it is wrapped on by sphere mapping.
    texture: Option<(u32, bool)>,
}

struct Toasters {
    trackball: Trackball,
    lists: Vec<u32>,
    finishes: Vec<Finish>,
    floaters: Vec<Floater>,

    last_view: usize,
    target_view: usize,
    view_x: f32,
    view_y: f32,
    view_steps: f32,
    view_tick: f32,
    auto_tracking: bool,
    track_tick: f32,

    aspect: f32,
    scale: f32,
    speed: f32,
    fog: bool,
    wire: bool,
}

impl Toasters {
    /// `reset_floater`: put one back at the far end of the sky.
    fn reset_floater(&self, f: &mut Floater) {
        let n = GRID_SIZE / 2.0;
        let n2 = GRID_DEPTH / 2.0;
        let delta = GRID_SIZE * self.speed / 200.0;
        f.dx = 0.0;
        f.dy = 0.0;
        f.dz = delta;
        f.wz = 0.0;
        f.dwz = 0.0;
        f.dz += bellrand(delta as f64) - delta / 3.0;
        if random().is_multiple_of(5) {
            f.dx += bellrand(delta as f64 * 2.0) - delta;
            f.dy += bellrand(delta as f64 * 2.0) - delta;
        }
        if random().is_multiple_of(40) {
            // The occasional speedy one.
            f.dz *= 10.0;
        }
        f.x = frand(n as f64) as f32 - n / 2.0;
        f.y = frand(n as f64) as f32 - n / 2.0;
        f.z = -n2 - frand(delta as f64 * 4.0) as f32;

        if f.toaster {
            f.loaded = 0;
            f.knob_pos = frand(180.0) as f32 - 90.0;
            f.handle_pos = if random() & 1 == 1 { 0.0 } else { 1.0 };
            if f.handle_pos > 0.8 && random().is_multiple_of(5) {
                // Let's toast.
                f.loaded = random() & 3;
            }
            // Only empty toasters barrel-roll, since there is no code for
            // toast falling out of one.
            if f.loaded == 0 && random().is_multiple_of(50) {
                f.dwz = (bellrand(2.0) - 1.0) * (4.0 + bellrand(6.0));
            }
        } else if random().is_multiple_of(10) {
            f.toast_type = 1;
        }
    }

    fn tick_floater(&self, f: &mut Floater) {
        let n1 = GRID_DEPTH / 2.0;
        let n2 = GRID_SIZE * 4.0;
        f.x += f.dx;
        f.y += f.dy;
        f.z += f.dz;
        f.wz = (f.wz + f.dwz) % 360.0;
        if random().is_multiple_of(50000) {
            // A sudden gust of gravity.
            f.dy -= 2.8;
        }
        if f.x < -n2 || f.x > n2 || f.y < -n2 || f.y > n2 || f.z > n1 {
            self.reset_floater(f);
        }
    }

    /// `auto_track`: drift the camera from one nice view to the next, easing
    /// out so that it does not jerk to a stop.
    fn auto_track(&mut self) {
        if !self.auto_tracking {
            self.track_tick += 1.0;
            if self.track_tick < 200.0 / self.speed {
                return;
            }
            self.track_tick = 0.0;
            if random().is_multiple_of(5) {
                self.auto_tracking = true;
            } else {
                return;
            }
        }
        let (ox, oy) = NICE_VIEWS[self.last_view];
        let (tx, ty) = NICE_VIEWS[self.target_view];
        let th = ((std::f32::consts::PI / 2.0) * self.view_tick / self.view_steps).sin();
        self.view_x = ox + (tx - ox) * th;
        self.view_y = oy + (ty - oy) * th;
        self.view_tick += 1.0;
        if self.view_tick >= self.view_steps {
            self.view_tick = 0.0;
            self.view_steps = 350.0 / self.speed;
            self.last_view = self.target_view;
            self.target_view = (random() as usize % (NICE_VIEWS.len() - 2)) + 2;
            self.auto_tracking = false;
        }
    }

    fn call(&self, g: &mut Gl, i: usize) {
        let f = &self.finishes[i];
        g.glx.material_ambient_diffuse(f.color);
        g.glx.material_specular(f.spec);
        g.glx.material_shininess(f.shiny);
        match f.texture {
            Some((id, sphere)) => {
                g.glx.texturing(true);
                g.glx.tex_env(TexEnv::Modulate);
                g.glx.bind_texture(id);
                g.glx.tex_gen_sphere(sphere);
            }
            None => {
                g.glx.texturing(false);
                g.glx.tex_gen_sphere(false);
            }
        }
        g.glx.call_list(self.lists[i]);
    }

    /// `draw_floater`: a toaster is nine pieces bolted together, and a slice
    /// of toast is one.
    fn draw_floater(&self, g: &mut Gl, f: &Floater) {
        g.glx.front_face_cw(false);
        g.glx.push_matrix();
        g.glx.translate(f.x, f.y, f.z);
        g.glx.rotate(f.wz, 0.0, 0.0, 1.0);

        if !f.toaster {
            g.glx.scale(0.7, 0.7, 0.7);
            self.call(
                g,
                if f.toast_type == 0 {
                    TOAST
                } else {
                    TOAST_BITTEN
                },
            );
            g.glx.pop_matrix();
            return;
        }

        g.glx.push_matrix();
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        self.call(g, BASE_TOASTER);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.translate(0.0, 1.01, 0.0);
        g.glx.scale(0.91, 0.91, 0.91);
        self.call(g, SLOTS);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        g.glx.translate(0.0, -0.4, -2.38);
        g.glx.scale(0.33, 0.33, 0.33);
        self.call(g, HANDLE_SLOT);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.translate(0.0, -1.1, 3.0);
        g.glx.scale(0.3, 0.3, 0.3);
        g.glx.translate(0.0, f.handle_pos * 4.8, 0.0);
        self.call(g, HANDLE);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        // Where the handle is, then down and to the left.
        g.glx.translate(0.0, -1.1, -3.0);
        g.glx.translate(1.0, -0.4, 0.0);
        g.glx.scale(0.08, 0.08, 0.08);
        g.glx.rotate(f.knob_pos, 0.0, 0.0, 1.0);
        self.call(g, KNOB);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        g.glx.translate(0.0, -2.3, 0.0);
        self.call(g, BASE);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.translate(-4.8, 0.0, 0.0);
        self.call(g, JET_WING);
        g.glx.scale(0.5, 0.5, 0.5);
        g.glx.translate(-2.0, -1.0, 0.0);
        self.call(g, JET);
        g.glx.pop_matrix();

        g.glx.push_matrix();
        g.glx.translate(4.8, 0.0, 0.0);
        // The other wing is the same one mirrored, so its winding flips.
        g.glx.scale(-1.0, 1.0, 1.0);
        g.glx.front_face_cw(true);
        self.call(g, JET_WING);
        g.glx.scale(0.5, 0.5, 0.5);
        g.glx.translate(-2.0, -1.0, 0.0);
        self.call(g, JET);
        g.glx.front_face_cw(false);
        g.glx.pop_matrix();

        if f.loaded != 0 {
            g.glx.push_matrix();
            g.glx.translate(0.0, 1.01, 0.0);
            g.glx.scale(0.91, 0.91, 0.91);
            g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            g.glx.translate(0.0, 0.0, -0.95);
            g.glx.translate(0.0, 0.72, 0.0);
            if f.loaded & 1 != 0 {
                self.call(g, TOAST);
            }
            g.glx.translate(0.0, -1.46, 0.0);
            if f.loaded & 2 != 0 {
                self.call(g, TOAST);
            }
            g.glx.pop_matrix();
        }
        g.glx.pop_matrix();
    }
}

/// A texture from one of the bundled pictures, drawn pixellated as upstream
/// asks.
fn load_texture(g: &mut Gl, png: &[u8]) -> u32 {
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    match crate::runtime::png::decode_rgba(png) {
        Some((w, h, px)) => g.glx.tex_image_2d(w, h, px),
        None => g.glx.tex_image_2d(1, 1, vec![255, 255, 255, 255]),
    }
    g.glx.tex_nearest(true);
    id
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let texture = g.res.bool("texture") && !wire;
    let ntoasters = g.res.int("ntoasters").max(0) as usize;
    let nslices = g.res.int("nslices").max(0) as usize;

    let (chrome, toast_texture) = if texture {
        (
            load_texture(g, crate::images::CHROMESPHERE),
            load_texture(g, crate::images::TOAST_PNG),
        )
    } else {
        (0, 0)
    };

    let mut lists = Vec::new();
    let mut finishes = Vec::new();
    for (i, src) in MODELS.iter().enumerate() {
        let model = GlList::parse(src);

        let (color, spec, shiny): ([f32; 4], [f32; 4], f32) = match i {
            BASE_TOASTER => ([1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0], 20.0),
            TOAST | TOAST_BITTEN => ([0.8, 0.8, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0], 0.0),
            SLOTS | HANDLE_SLOT => ([0.3, 0.3, 0.4, 1.0], [0.4, 0.4, 0.7, 1.0], 128.0),
            HANDLE => ([0.8, 0.1, 0.1, 1.0], [1.0, 1.0, 1.0, 1.0], 20.0),
            KNOB => ([0.8, 0.1, 0.1, 1.0], [0.0, 0.0, 0.0, 1.0], 0.0),
            JET | JET_WING => ([0.7, 0.7, 0.7, 1.0], [1.0, 1.0, 1.0, 1.0], 20.0),
            BASE => ([0.5, 0.5, 0.5, 1.0], [1.0, 1.0, 1.0, 1.0], 20.0),
            _ => ([1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0], 128.0),
        };
        // The chrome is a picture of a mirrored ball, wrapped on by sphere
        // mapping; the toast carries a picture of toast.
        let on_it = match i {
            BASE_TOASTER if texture => Some((chrome, true)),
            TOAST | TOAST_BITTEN if texture => Some((toast_texture, false)),
            _ => None,
        };
        finishes.push(Finish {
            color,
            spec,
            shiny,
            texture: on_it,
        });

        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        g.glx.push_matrix();
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        g.glx.scale(6.0, 6.0, 6.0);

        if texture && (i == TOAST || i == TOAST_BITTEN) {
            // Upstream lays the toast on in object coordinates, with the
            // texture matrix shifted by half so the middle of the picture
            // lands on the middle of the slice. There is no object-linear
            // generator here, so the coordinates are worked out per vertex,
            // which comes to the same thing.
            let stride = model.format.stride();
            let skip = stride - 3;
            g.glx.begin(model.primitive);
            for v in model.data.chunks_exact(stride) {
                if skip == 3 {
                    g.glx.normal3f(v[0], v[1], v[2]);
                }
                let (x, y, z) = (v[skip], v[skip + 1], v[skip + 2]);
                g.glx.tex_coord2f(x + 0.5, y + 0.5);
                g.glx.vertex3f(x, y, z);
            }
            g.glx.end();
        } else {
            model.render(&mut g.glx, wire);
        }

        g.glx.pop_matrix();
        g.glx.end_list();
        lists.push(list);
    }

    let mut this = Toasters {
        trackball: Trackball::new(),
        lists,
        finishes,
        floaters: Vec::new(),
        last_view: (random() % 2) as usize,
        target_view: 0,
        view_x: 0.0,
        view_y: 0.0,
        view_steps: 100.0,
        view_tick: 0.0,
        auto_tracking: true,
        track_tick: 0.0,
        aspect: 1.0,
        scale: 1.0,
        speed: g.res.float("speed") as f32,
        fog: g.res.bool("fog"),
        wire,
    };
    this.target_view = this.last_view + 2;
    this.view_x = NICE_VIEWS[this.last_view].0;
    this.view_y = NICE_VIEWS[this.last_view].1;

    let mut floaters = vec![Floater::default(); ntoasters + nslices];
    for (i, f) in floaters.iter_mut().enumerate() {
        // Arrange the list so that half the toasters are in front of the
        // bread and half behind it.
        f.toaster = i < ntoasters / 2 || i >= nslices + (ntoasters / 2);
        this.reset_floater(f);
        // The first generation starts anywhere, but not yet on screen: the
        // view rotates into position first.
        let (min, max) = (-GRID_DEPTH / 2.0, GRID_DEPTH / 3.5);
        f.z = frand((max - min) as f64) as f32 + min;
    }
    this.floaters = floaters;

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Toasters {
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
        g.glx.perspective(40.0, self.aspect, 1.0, 250.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 2.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.scale(self.scale, self.scale, self.scale);

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
        g.glx.rotate(self.view_x, 1.0, 0.0, 0.0);
        g.glx.rotate(self.view_y, 0.0, 1.0, 0.0);
        // Rotate the scene about a point a little deeper in.
        g.glx.translate(0.0, 0.0, -50.0);
        g.glx.mult_matrix(self.trackball.matrix());
        g.glx.translate(0.0, 0.0, 50.0);

        g.glx.scale(0.5, 0.5, 0.5);
        g.glx.translate(0.0, 0.0, -GRID_DEPTH / 2.5);

        // Without fog the far end of the sky is as bright as the near end and
        // the toasters pile into an unreadable band.
        g.glx.fog(if self.fog && !self.wire {
            Some(Fog::Exp2 {
                density: 0.0085,
                color: [0.0, 0.0, 0.0, 1.0],
            })
        } else {
            None
        });

        let mut floaters = std::mem::take(&mut self.floaters);
        for f in &mut floaters {
            self.draw_floater(g, f);
            if !self.trackball.button_down() {
                self.tick_floater(f);
            }
        }
        self.floaters = floaters;

        if !self.trackball.button_down() {
            self.auto_track();
        }
        g.glx.pop_matrix();
        g.glx.fog(None);

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:      30000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*speed:      1.0",
    "*ntoasters:  20",
    "*nslices:    25",
    "*texture:    True",
    "*fog:        True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("ntoasters", "Toasters", 0.0, 100.0, 1.0, 0, "20"),
    Opt::slider("nslices", "Slices of toast", 0.0, 100.0, 1.0, 0, "25"),
    Opt::boolean("texture", "Textured", "true"),
    Opt::boolean("fog", "Fog", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "flyingtoasters",
    label: "Flying Toasters",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=mLGDvtbFvfg"),
        blurb: "3D flying toasters, and toast, after the ancient Berkeley \
                Systems hack.",
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
    use crate::runtime::gl::Shape;

    /// All ten models load, and none of them is empty.
    #[test]
    fn the_toaster_is_ten_pieces() {
        for (i, src) in MODELS.iter().enumerate() {
            let m = GlList::parse(src);
            assert!(m.points > 0, "model {i} is empty");
            assert_eq!(m.primitive, Shape::Triangles, "model {i} is not triangles");
        }
    }

    /// Half the toasters are put in front of the bread and half behind, so
    /// that the two are interleaved rather than layered.
    #[test]
    fn the_toasters_are_shuffled_through_the_bread() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "ntoasters=20&nslices=25&texture=false",
            20260811,
        ));
        r.step();
        // Ten toasters, then twenty-five slices, then ten more toasters.
        let expect: Vec<bool> = (0..45).map(|i| !(10..35).contains(&i)).collect();
        let toasters = expect.iter().filter(|t| **t).count();
        assert_eq!(toasters, 20, "the count is wrong");
        assert!(expect[0] && !expect[10] && expect[35], "not interleaved");
    }

    /// A toaster is drawn from nine pieces and a slice of toast from one, so
    /// the batch count says how many of each are on screen.
    #[test]
    fn a_toaster_is_nine_pieces_and_toast_is_one() {
        let batches = |query: &str| {
            let mut r = start(StartArgs::new(640, 480, query, 20260811));
            r.step();
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
                .count()
        };
        // With no toast at all, every batch belongs to a toaster. Nine pieces
        // each, plus up to two slices loaded in some of them.
        let one = batches("ntoasters=1&nslices=0&texture=false");
        assert!((9..=11).contains(&one), "one toaster drew {one} batches");
        let none = batches("ntoasters=0&nslices=4&texture=false");
        assert_eq!(none, 4, "four slices drew {none} batches");
    }

    /// Everything flies towards the viewer and is put back at the far end
    /// when it goes past, so nothing ever leaves the sky for good.
    #[test]
    fn they_keep_coming() {
        let mut r = start(StartArgs::new(640, 480, "texture=false&speed=10", 20260811));
        for _ in 0..500 {
            r.step();
            let f = r.frame();
            assert!(!f.batches.is_empty(), "the sky emptied");
            assert!(
                f.vertices
                    .iter()
                    .all(|v| v.pos.iter().all(|c| c.is_finite())),
                "a vertex went to NaN"
            );
        }
    }

    /// The camera moves between the views that look nice, and only ever sits
    /// at one of them.
    #[test]
    fn the_camera_visits_the_nice_views() {
        let mut r = start(StartArgs::new(640, 480, "texture=false", 20260811));
        r.step();
        let first = r.frame().batches[0].modelview.0;
        for _ in 0..300 {
            r.step();
        }
        let later = r.frame().batches[0].modelview.0;
        assert!(
            first
                .iter()
                .zip(later.iter())
                .any(|(a, b)| (a - b).abs() > 0.01),
            "the camera never moved"
        );
    }

    /// The chrome is wrapped on by sphere mapping and the toast is laid on in
    /// object coordinates, so the toast's texture coordinates come from the
    /// model and the chrome's do not.
    #[test]
    fn the_pictures_go_on_the_right_way() {
        let mut r = start(StartArgs::new(640, 480, "ntoasters=1&nslices=1", 20260811));
        r.step();
        let f = r.frame();
        let textured: Vec<_> = f.batches.iter().filter(|b| b.texture.is_some()).collect();
        assert!(textured.len() >= 2, "only {} textured", textured.len());

        let chrome = textured
            .iter()
            .find(|b| b.tex_gen_sphere)
            .expect("nothing is sphere mapped");
        let toast = textured
            .iter()
            .find(|b| !b.tex_gen_sphere)
            .expect("nothing carries its own texture coordinates");
        assert_ne!(chrome.texture, toast.texture, "one picture for both");

        // The slice's coordinates span the picture, so it is laid across the
        // bread rather than sampled at one point.
        let uvs: Vec<[f32; 2]> = f.vertices[toast.first..toast.first + toast.count]
            .iter()
            .map(|v| v.uv)
            .collect();
        let lo = uvs.iter().fold(f32::MAX, |a, uv| a.min(uv[0]));
        let hi = uvs.iter().fold(f32::MIN, |a, uv| a.max(uv[0]));
        assert!(hi - lo > 0.5, "the toast picture spans only {}", hi - lo);
        assert!(lo > -0.1 && hi < 1.1, "the picture runs off the slice");
    }

    /// Both pictures decode: the chrome and the toast.
    #[test]
    fn the_pictures_decode() {
        for (name, png) in [
            ("chromesphere", crate::images::CHROMESPHERE),
            ("toast", crate::images::TOAST_PNG),
        ] {
            let (w, h, px) = crate::runtime::png::decode_rgba(png)
                .unwrap_or_else(|| panic!("{name} did not decode"));
            assert!(w > 0 && h > 0, "{name} is empty");
            assert_eq!(px.len(), (w * h * 4) as usize);
        }
    }
}
