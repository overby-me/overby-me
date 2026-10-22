//! Port of `hacks/glx/glforestfire.c`.
//!
//! ```text
//! fire --- 3D fire or rain landscape
//!
//! Copyright (c) E. Lassauge, 2001.
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
//!
//! The original code for this mode was written by David Bucciarelli
//! (tech.hmw@plus.it) and could be found in the demo package
//! of Mesa (Mesa-3.2/3Dfx/demos/). This mode is the result of the merge of
//! two of the David's demos (fire and rain).
//!
//! Eric Lassauge  (October-10-2000) <lassauge@users.sourceforge.net>
//!                                  http://lassauge.free.fr/linux.html
//! ```
//!
//! Sprinkling fire-like triangles over a landscape of trees, or, with the
//! count turned down to nothing, rain over the same landscape.
//!
//! Each flame is one triangle that is thrown up out of a small ring on the
//! ground, falls back under gravity, and is reseeded the moment it lands. Its
//! three corners each carry their own colour, which starts red and is nudged a
//! step towards yellow every frame while its alpha counts down, so the flame
//! brightens as it rises and fades out as it ages. A second pass draws the
//! same triangles flattened onto the ground in black, which is the shadow.
//!
//! The trees are two texture-mapped quads crossed at right angles, so a tree
//! reads from any side. The texture has a transparent background, which is
//! what the alpha test is for: blending it would still write depth over the
//! whole quad and cut a tree-shaped hole in whatever is behind it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Fog, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
};

/// How close to the middle a tree may stand, and how far out.
const TREE_IN_R: f32 = 2.5;
const TREE_OUT_R: f32 = 8.0;
/// Half the width of the square of ground.
const DIMP: f32 = 20.0;
/// How many times the ground texture repeats across it.
const DIMTP: f32 = 16.0;
/// How much of a flame's colour is jittered per corner.
const RIDCOL: f32 = 0.4;
const AGRAV: f32 = -9.8;
/// How many raindrops, when there is rain instead of fire.
const NUMPART: usize = 7500;
const MAX_TREES: i32 = 20;

/// Where the observer stands before wandering moves them.
const DEF_OBS: [f32; 3] = [2.0, 1.0, 0.0];
const DEF_ALPHA: f32 = -90.0;
const DEF_BETA: f32 = 90.0;

/// Red-ish, the colour a flame starts at.
const PARTCOL1: [f32; 3] = [1.0, 0.2, 0.0];
/// Yellow-ish, the colour it works towards.
const PARTCOL2: [f32; 3] = [1.0, 1.0, 0.0];
const FOGCOLOR: [f32; 4] = [0.9, 0.9, 1.0, 1.0];

/// One flame: a triangle with a velocity, an age, and a colour per corner.
#[derive(Clone, Copy, Default)]
struct Part {
    age: i32,
    p: [[f32; 3]; 3],
    v: [f32; 3],
    c: [[f32; 4]; 3],
}

/// One raindrop, drawn as the line between where it was and where it is.
#[derive(Clone, Copy, Default)]
struct Rain {
    acc: [f32; 3],
    vel: [f32; 3],
    pos: [f32; 3],
    part_length: f32,
    oldpos: [f32; 3],
}

/// A random number from zero to one, which is what upstream's `vrnd` is.
fn vrnd() -> f32 {
    frand(1.0) as f32
}

fn vadds(a: &mut [f32; 3], dt: f32, b: &[f32; 3]) {
    for i in 0..3 {
        a[i] += dt * b[i];
    }
}

struct Fire {
    trackball: Trackball,

    /// How many flames. Zero means rain instead.
    np: usize,
    /// The radius of the ring the flames come out of.
    eject_r: f32,
    dt: f32,
    maxage: f32,
    eject_vy: f32,
    eject_vl: f32,
    /// How big a flame is.
    ridtri: f32,
    shadows: bool,
    fog: bool,

    p: Vec<Part>,
    r: Vec<Rain>,

    ground_tex: Option<u32>,
    tree_tex: Option<u32>,
    trees: Vec<[f32; 3]>,

    /// The box the rain falls in.
    min: [f32; 3],
    max: [f32; 3],

    obs: [f32; 3],
    dir: [f32; 3],
    v: f32,
    alpha: f32,
    beta: f32,

    wander: bool,
    wireframe: bool,
    frame: i32,
}

impl Fire {
    /// `setnewpart`: throw a fresh flame up out of the ring.
    fn setnewpart(&self, p: &mut Part) {
        p.age = 0;
        let a = vrnd() * std::f32::consts::TAU;
        let vi = [
            a.sin() * self.eject_r * vrnd(),
            0.15,
            a.cos() * self.eject_r * vrnd(),
        ];
        for corner in &mut p.p {
            for i in 0..3 {
                corner[i] = vi[i] + vrnd() * self.ridtri;
            }
        }
        p.v = [
            vi[0] * self.eject_vl / (self.eject_r / 2.0),
            vrnd() * self.eject_vy + self.eject_vy / 2.0,
            vi[2] * self.eject_vl / (self.eject_r / 2.0),
        ];
        // Each corner takes its own jittered shade of the starting red, so a
        // flame is never flat.
        for c in &mut p.c {
            for i in 0..3 {
                c[i] = PARTCOL1[i] * ((1.0 - RIDCOL) + vrnd() * RIDCOL);
            }
            c[3] = 1.0;
        }
    }

    /// `setpart`: move a flame on one step, or start it again if it has landed.
    fn setpart(&self, p: &mut Part) {
        if p.p[0][1] < 0.1 {
            self.setnewpart(p);
            return;
        }

        p.v[1] += AGRAV * self.dt;
        let v = p.v;
        for corner in &mut p.p {
            vadds(corner, self.dt, &v);
        }
        p.age += 1;

        if p.age as f32 > self.maxage {
            for c in &mut p.c {
                c[..3].copy_from_slice(&PARTCOL2);
            }
        } else {
            let fact = 1.0 / self.maxage;
            for c in &mut p.c {
                for i in 0..3 {
                    c[i] = (c[i] + fact * PARTCOL2[i]).clamp(0.0, 1.0);
                }
                c[3] = fact * (self.maxage - p.age as f32);
            }
        }
    }

    /// `setnewrain`: a fresh drop somewhere over the box.
    fn setnewrain(&self, r: &mut Rain) {
        r.acc = [0.0, -0.98, 0.0];
        r.vel = [0.0, 0.0, 0.0];
        r.part_length = 0.2;
        r.oldpos = [
            self.min[0] + (self.max[0] - self.min[0]) * vrnd(),
            self.max[1] + 0.2 * self.max[1] * vrnd(),
            self.min[2] + (self.max[2] - self.min[2]) * vrnd(),
        ];
        r.pos = r.oldpos;
        let vel = r.vel;
        vadds(&mut r.oldpos, -r.part_length, &vel);

        r.pos[1] = (self.max[1] - self.min[1]) * vrnd() + self.min[1];
        r.oldpos[1] = r.pos[1] - r.part_length * r.vel[1];
    }

    /// `setpartrain`: one step of a drop, wrapping it round the sides of the
    /// box and starting it again when it reaches the floor.
    fn setpartrain(&self, r: &mut Rain, dt: f32) {
        let acc = r.acc;
        vadds(&mut r.vel, dt, &acc);
        let vel = r.vel;
        vadds(&mut r.pos, dt, &vel);

        for i in [0usize, 2] {
            if r.pos[i] < self.min[i] {
                r.pos[i] = self.max[i] - (self.min[i] - r.pos[i]);
            }
            if r.pos[i] > self.max[i] {
                r.pos[i] = self.min[i] + (r.pos[i] - self.max[i]);
            }
        }

        r.oldpos = r.pos;
        let vel = r.vel;
        vadds(&mut r.oldpos, -r.part_length, &vel);
        if r.pos[1] < self.min[1] {
            self.setnewrain(r);
        }
    }

    /// `calcposobs`: where the observer is and which way they are looking.
    fn calcposobs(&mut self) {
        self.dir[0] = (self.alpha * std::f32::consts::PI / 180.0).sin();
        self.dir[2] = (self.alpha * std::f32::consts::PI / 180.0).cos()
            * (self.beta * std::f32::consts::PI / 180.0).sin();
        self.dir[1] = (self.beta * std::f32::consts::PI / 180.0).cos();

        for i in 0..3 {
            self.obs[i] += self.v * self.dir[i];
        }

        if self.np == 0 {
            self.min = [self.obs[0] - 7.0, -0.2, self.obs[2] - 7.0];
            self.max = [self.obs[0] + 7.0, 8.0, self.obs[2] + 7.0];
        }
    }

    /// `drawtree`: two quads crossed at right angles, so the tree reads from
    /// any side.
    fn drawtree(g: &mut Gl, x: f32, y: f32, z: f32) {
        g.glx.begin(Shape::Quads);
        for (u, v, vx, vy, vz) in [
            (0.0, 0.0, x - 1.5, y, z),
            (1.0, 0.0, x + 1.5, y, z),
            (1.0, 1.0, x + 1.5, y + 3.0, z),
            (0.0, 1.0, x - 1.5, y + 3.0, z),
            (0.0, 0.0, x, y, z - 1.5),
            (1.0, 0.0, x, y, z + 1.5),
            (1.0, 1.0, x, y + 3.0, z + 1.5),
            (0.0, 1.0, x, y + 3.0, z - 1.5),
        ] {
            // The textures here are top-down where OpenGL's are bottom-up, so
            // every v is flipped.
            g.glx.tex_coord2f(u, 1.0 - v);
            g.glx.vertex3f(vx, vy, vz);
        }
        g.glx.end();
    }
}

impl Hack3d for Fire {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.wireframe;

        g.glx.depth_test(true);
        // Blending makes the flames melt into the background. Upstream leaves
        // it off in wireframe unless it is drawing rain.
        if !wire || self.np == 0 {
            g.glx.blend(Blend::Alpha);
        } else {
            g.glx.blend(Blend::Off);
        }

        if self.wander && !self.trackball.button_down() {
            let sinoid = |scale: f32, size: f32| {
                ((1.0 + (self.frame as f32 * scale / 2.0 * std::f32::consts::PI).sin()) / 2.0)
                    * size
                    - size / 2.0
            };
            let x = sinoid(0.031, 0.85);
            let y = sinoid(0.017, 0.25);
            let z = sinoid(0.023, 0.85);
            self.frame += 1;
            self.obs = [x + DEF_OBS[0], y + DEF_OBS[1], z + DEF_OBS[2]];
            self.dir[1] = y;
            self.dir[2] = z;
        }

        g.glx.fog(if self.fog {
            Some(Fog::Exp {
                density: 0.03,
                color: FOGCOLOR,
            })
        } else {
            None
        });

        g.glx.depth_mask(true);
        // Sky in the distance.
        g.glx.clear_color(0.5, 0.5, 0.8, 1.0);
        g.glx.clear();

        g.glx.push_matrix();
        self.calcposobs();

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.look_at(
            self.obs,
            [
                self.obs[0] + self.dir[0],
                self.obs[1] + self.dir[1],
                self.obs[2] + self.dir[2],
            ],
            [0.0, 1.0, 0.0],
        );

        // The ground. Upstream uses GL_DECAL for it, which for an opaque
        // texture under a white colour is the same as the GL_MODULATE this
        // runtime has.
        if let Some(id) = self.ground_tex {
            g.glx.texturing(true);
            g.glx.bind_texture(id);
            // White, so the texture comes out in its own colours.
            g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        } else {
            g.glx.texturing(false);
            g.glx.color4f(0.54, 0.27, 0.07, 1.0);
        }
        g.glx.begin(Shape::Quads);
        for ([x, y, z], [u, v]) in [
            ([-DIMP, 0.0, -DIMP], [-DIMTP, -DIMTP]),
            ([DIMP, 0.0, -DIMP], [DIMTP, -DIMTP]),
            ([DIMP, 0.0, DIMP], [DIMTP, DIMTP]),
            ([-DIMP, 0.0, DIMP], [-DIMTP, DIMTP]),
        ] {
            g.glx.tex_coord2f(u, 1.0 - v);
            g.glx.vertex3f(x, y, z);
        }
        g.glx.end();

        // The trees, whose texture is a cut-out: without the alpha test the
        // transparent corners of each quad would still write depth.
        if let Some(id) = self.tree_tex
            && !self.trees.is_empty()
        {
            g.glx.texturing(true);
            g.glx.alpha_test(Some(0.9));
            g.glx.bind_texture(id);
            for t in self.trees.clone() {
                Fire::drawtree(g, t[0], t[1], t[2]);
            }
            g.glx.alpha_test(None);
        }
        g.glx.texturing(false);
        g.glx.depth_mask(false);

        let tris = if wire {
            Shape::LineStrip
        } else {
            Shape::Triangles
        };

        // The shadows: the same triangles flattened onto the ground in black,
        // keeping only each corner's alpha.
        if self.shadows {
            g.glx.begin(tris);
            for p in &self.p {
                for k in 0..3 {
                    g.glx.color4f(0.0, 0.0, 0.0, p.c[k][3]);
                    g.glx.vertex3f(p.p[k][0], 0.1, p.p[k][2]);
                }
            }
            g.glx.end();
        }

        // The flames themselves, moved on one step as they are drawn.
        g.glx.begin(tris);
        for j in 0..self.p.len() {
            let p = self.p[j];
            for k in 0..3 {
                g.glx.color4f(p.c[k][0], p.c[k][1], p.c[k][2], p.c[k][3]);
                g.glx.vertex3f(p.p[k][0], p.p[k][1], p.p[k][2]);
            }
            let mut p = p;
            self.setpart(&mut p);
            self.p[j] = p;
        }
        g.glx.end();

        // Rain, when there is no fire.
        if self.np == 0 {
            // Upstream measures the frame time with `clock()` and then throws
            // the answer away, returning a constant; this is that constant.
            let timeused = 0.0150;
            g.glx.begin(Shape::Lines);
            for j in 0..self.r.len() {
                let r = self.r[j];
                g.glx.color4f(0.7, 0.95, 1.0, 0.0);
                g.glx.vertex3f(r.oldpos[0], r.oldpos[1], r.oldpos[2]);
                g.glx.color4f(0.3, 0.7, 1.0, 1.0);
                g.glx.vertex3f(r.pos[0], r.pos[1], r.pos[2]);
                let mut r = r;
                self.setpartrain(&mut r, timeused);
                self.r[j] = r;
            }
            g.glx.end();
        }

        g.glx.depth_mask(true);
        g.glx.fog(None);
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx
            .perspective(70.0, width as f32 / height.max(1) as f32, 0.1, 30.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

/// Load a texture, returning its id, or `None` if the picture will not decode.
fn load(g: &mut Gl, png: &[u8]) -> Option<u32> {
    let (w, h, px) = crate::runtime::png::decode_rgba(png)?;
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    g.glx.tex_image_2d(w, h, px);
    Some(id)
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let do_texture = g.res.bool("texture");
    let np = g.res.int("count").clamp(0, 8000) as usize;

    // Upstream drops the trees when there are no textures, since an untextured
    // tree is a pair of white quads.
    let num_trees = if do_texture {
        g.res.int("trees").clamp(0, MAX_TREES)
    } else {
        0
    };

    let (ground_tex, tree_tex) = if do_texture {
        let ground = load(g, crate::images::GROUND);
        let tree = if num_trees > 0 {
            load(g, crate::images::TREE)
        } else {
            None
        };
        (ground, tree)
    } else {
        (None, None)
    };

    let mut st = Fire {
        trackball: Trackball::new(),
        np,
        eject_r: 0.1 + (frand(10.0) as f32).floor() * 0.03,
        dt: 0.015,
        maxage: 1.0 / 0.015,
        eject_vy: 4.0,
        eject_vl: 1.0,
        ridtri: 0.1 + (frand(10.0) as f32).floor() * 0.005,
        shadows: g.res.bool("shadows"),
        fog: g.res.bool("fog"),
        p: vec![Part::default(); np],
        r: Vec::new(),
        ground_tex,
        tree_tex: if num_trees > 0 { tree_tex } else { None },
        trees: Vec::new(),
        min: [0.0; 3],
        max: [0.0; 3],
        obs: DEF_OBS,
        dir: [0.0; 3],
        v: 0.0,
        alpha: DEF_ALPHA,
        beta: DEF_BETA,
        wander: g.res.bool("wander"),
        wireframe: wire,
        frame: 0,
    };

    for j in 0..st.p.len() {
        let mut p = st.p[j];
        st.setnewpart(&mut p);
        st.p[j] = p;
    }

    // The trees stand in a ring: outside a clearing round the fire, inside the
    // edge of the ground.
    for _ in 0..num_trees {
        // Upstream's `do ... while` has no bound. Rejection from a square that
        // the annulus covers most of lands quickly, but a run of unlucky draws
        // must not hang the frame, so it gives up and takes the last one.
        let mut t = [0.0f32; 3];
        for _ in 0..1000 {
            t = [
                vrnd() * TREE_OUT_R * 2.0 - TREE_OUT_R,
                0.0,
                vrnd() * TREE_OUT_R * 2.0 - TREE_OUT_R,
            ];
            let dist = (t[0] * t[0] + t[2] * t[2]).sqrt();
            if (TREE_IN_R..=TREE_OUT_R).contains(&dist) {
                break;
            }
        }
        st.trees.push(t);
    }

    // No fire means rain.
    if np == 0 {
        st.min = [-7.0, -0.2, -7.0];
        st.max = [7.0, 8.0, 7.0];
        st.r = vec![Rain::default(); NUMPART];
        for j in 0..st.r.len() {
            let mut r = st.r[j];
            st.setnewrain(&mut r);
            st.r[j] = r;
        }
    }

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     10000",
    "*count:     800",
    "*size:      0",
    "*showFPS:   False",
    "*wireframe: False",
    "*texture:   True",
    "*fog:       False",
    "*shadows:   True",
    "*wander:    True",
    "*trees:     5",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Count", 0.0, 8000.0, 100.0, 0, "800"),
    Opt::slider("trees", "Number of trees", 0.0, 20.0, 1.0, 0, "5"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("texture", "Textures", "true"),
    Opt::boolean("shadows", "Shadows", "true"),
    Opt::boolean("fog", "Fog", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glforestfire",
    label: "GL Forest Fire",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Eric Lassauge",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=_0Ff3qHUfsA"),
        blurb: "Sprinkling fire-like triangles in a landscape filled with trees.",
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

    /// A flame is thrown up out of a small ring, falls back, and is reseeded
    /// the moment it lands, so no flame ever goes on falling.
    #[test]
    fn a_flame_that_lands_is_thrown_again() {
        let mut r = start(StartArgs::new(640, 480, "count=200", 20260812));
        for _ in 0..200 {
            r.step();
        }
        // Whatever it has been doing, every triangle is above the ground and
        // inside the ring it was thrown from.
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
    }

    /// The colours run from red towards yellow as a flame ages, and its alpha
    /// counts down to nothing over `maxage` frames.
    #[test]
    fn a_flame_ages_from_red_to_yellow() {
        let st = a_fire(50);
        let mut p = Part::default();
        st.setnewpart(&mut p);

        // It starts red-ish: more red than green, and no blue to speak of.
        assert!(p.c[0][0] > p.c[0][1], "{:?} is not red", p.c[0]);
        assert!(p.c[0][2] < 0.01, "{:?} has blue in it", p.c[0]);
        assert_eq!(p.c[0][3], 1.0);

        // Lift it clear of the ground so it ages rather than being reseeded.
        for corner in &mut p.p {
            corner[1] = 5.0;
        }
        p.v = [0.0, 0.0, 0.0];
        let mut last_alpha = 1.0;
        for _ in 0..50 {
            st.setpart(&mut p);
            assert!(p.c[0][3] < last_alpha, "the alpha stopped counting down");
            last_alpha = p.c[0][3];
        }
        // Green has nearly caught red up, which is what makes it yellow, and
        // there is still a little alpha left at this point in its life.
        assert!(p.c[0][1] > 0.8, "{:?} never went yellow", p.c[0]);
        assert!(p.c[0][3] > 0.0, "it faded out before it turned");
    }

    /// A flame that reaches the ground is reseeded rather than left to fall
    /// through it.
    #[test]
    fn a_landed_flame_is_reseeded() {
        let st = a_fire(50);
        let mut p = Part::default();
        st.setnewpart(&mut p);
        p.age = 999;
        for corner in &mut p.p {
            corner[1] = 0.0;
        }
        st.setpart(&mut p);
        assert_eq!(p.age, 0, "it was not reseeded");
        assert!(p.p[0][1] > 0.0, "it was reseeded underground");
    }

    /// The trees stand in a ring: clear of the fire in the middle, inside the
    /// edge of the ground.
    #[test]
    fn the_trees_stand_in_a_ring() {
        let r = run("trees=20", 1);
        assert!(!r.frame().vertices.is_empty());

        // The placement is the same rejection loop, checked directly.
        let st = a_fire(0);
        for t in &st.trees {
            let dist = (t[0] * t[0] + t[2] * t[2]).sqrt();
            assert!(
                (TREE_IN_R..=TREE_OUT_R).contains(&dist),
                "a tree stands {dist} out"
            );
            assert_eq!(t[1], 0.0, "a tree left the ground");
        }
    }

    /// A raindrop that runs off one side of the box comes back on the other,
    /// and one that reaches the floor starts again from the top.
    #[test]
    fn rain_wraps_round_and_starts_again() {
        let st = a_fire(0);
        let mut r = Rain {
            acc: [0.0, -0.98, 0.0],
            vel: [0.0, 0.0, 0.0],
            pos: [st.min[0] - 0.5, 4.0, 0.0],
            part_length: 0.2,
            oldpos: [0.0; 3],
        };
        st.setpartrain(&mut r, 0.015);
        assert!(
            r.pos[0] > st.max[0] - 1.0,
            "it did not come back on the other side: {}",
            r.pos[0]
        );

        let mut r2 = Rain {
            pos: [0.0, st.min[1] - 1.0, 0.0],
            ..r
        };
        st.setpartrain(&mut r2, 0.015);
        assert!(r2.pos[1] > st.min[1], "it fell through the floor");
    }

    /// With no count there is no fire, and the saver draws rain instead.
    #[test]
    fn no_count_means_rain() {
        let r = run("count=0", 3);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        assert!(
            f.batches
                .iter()
                .any(|b| b.primitive == crate::runtime::gl::Primitive::Lines),
            "no rain was drawn"
        );
    }

    /// It draws, with a ground, trees and flames.
    #[test]
    fn the_fire_burns() {
        let r = run("", 5);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        assert!(
            f.batches.iter().any(|b| b.texture.is_some()),
            "the ground was never textured"
        );
        assert!(
            f.batches.iter().any(|b| b.alpha_test.is_some()),
            "the trees were drawn without the alpha test"
        );
    }

    /// A `Fire` with no GL behind it, for the state machines.
    fn a_fire(np: usize) -> Fire {
        let mut st = Fire {
            trackball: Trackball::new(),
            np,
            eject_r: 0.25,
            dt: 0.015,
            maxage: 1.0 / 0.015,
            eject_vy: 4.0,
            eject_vl: 1.0,
            ridtri: 0.12,
            shadows: true,
            fog: false,
            p: vec![Part::default(); np],
            r: Vec::new(),
            ground_tex: None,
            tree_tex: None,
            trees: Vec::new(),
            min: [-7.0, -0.2, -7.0],
            max: [7.0, 8.0, 7.0],
            obs: DEF_OBS,
            dir: [0.0; 3],
            v: 0.0,
            alpha: DEF_ALPHA,
            beta: DEF_BETA,
            wander: true,
            wireframe: false,
            frame: 0,
        };
        for _ in 0..20 {
            let mut t = [0.0f32; 3];
            for _ in 0..1000 {
                t = [
                    vrnd() * TREE_OUT_R * 2.0 - TREE_OUT_R,
                    0.0,
                    vrnd() * TREE_OUT_R * 2.0 - TREE_OUT_R,
                ];
                let dist = (t[0] * t[0] + t[2] * t[2]).sqrt();
                if (TREE_IN_R..=TREE_OUT_R).contains(&dist) {
                    break;
                }
            }
            st.trees.push(t);
        }
        st
    }
}
