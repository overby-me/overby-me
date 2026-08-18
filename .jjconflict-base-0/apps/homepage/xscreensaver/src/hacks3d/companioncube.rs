//! Port of `hacks/glx/companion.c`.
//!
//! ```text
//! companioncube, Copyright (c) 2011-2018 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! The symptoms most commonly produced by Enrichment Center testing are
//! superstition, perceiving inanimate objects as alive, and hallucinations.
//! The Enrichment Center reminds you that the weighted companion cube will
//! never threaten to stab you and, in fact, cannot speak.  In the event that
//! the Weighted Companion Cube does speak, the Enrichment Center urges you to
//! disregard its advice.
//! ```
//!
//! Weighted Storage Cubes bouncing up from below the bottom of the screen and
//! falling back out of it, turning slowly as they go.
//!
//! Only three of the shapes were modelled: the rounded corner piece, the
//! recessed disc, and the heart in the middle of it. Everything else is a
//! handful of quads, and the whole cube is those three shapes repeated four
//! times a face and six times over.
//!
//! Upstream compiles the assembled cube into a display list once and then
//! calls it per cube, which on a graphics card means the vertices are sent
//! over once and drawn many times. The recorder here has no equivalent: a
//! display list holds the commands that made it and replaying one emits the
//! vertices again, so the cube is simply built per cube. That is the same
//! amount of drawing and more traffic, which at the default of three cubes is
//! not worth a mechanism.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::Shape;
use crate::runtime::gllist::GlList;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

const SPEED_SCALE: f32 = 0.2;
const BOTTOM: f32 = 28.0;

/// A number in nought to `n`, but bunched up in the middle.
fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

fn randsign() -> f32 {
    if random() & 1 != 0 { 1.0 } else { -1.0 }
}

struct Floater {
    x: f32,
    y: f32,
    z: f32,
    ix: f32,
    iz: f32,
    dx: f32,
    dy: f32,
    dz: f32,
    ddx: f32,
    ddy: f32,
    ddz: f32,
    zr: f32,
    rot: Rotator,
    /// This one turns end over end as it flies, which most of them do not.
    spinner_p: bool,
}

struct CompanionCube {
    trackball: Trackball,
    floaters: Vec<Floater>,

    quad: GlList,
    disc: GlList,
    heart: GlList,

    speed: f32,
    do_spin: bool,
    do_wander: bool,
    wireframe: bool,
}

/// `reset_floater`: put one back at the bottom and throw it up again.
fn reset_floater(f: &mut Floater, n: usize, speed: f32, do_spin: bool, do_wander: bool) {
    f.y = -BOTTOM;
    f.x = f.ix;
    f.z = f.iz;

    /* Yes, I know I'm varying the force of gravity instead of varying the
    launch velocity.  That's intentional: empirical studies indicate that it's
    way, way funnier that way. */
    f.dy = 5.0;
    f.dx = 0.0;
    f.dz = 0.0;

    /* -0.18 max  -0.3 top -0.4 middle  -0.6 bottom */
    f.ddy = speed * SPEED_SCALE * (-0.6 + bellrand(0.45));
    f.ddx = 0.0;
    f.ddz = 0.0;

    f.spinner_p = if do_spin || do_wander {
        false
    } else {
        !random().is_multiple_of(3 * n as u32)
    };

    if random().is_multiple_of(30 * n as u32) {
        f.dx = bellrand(1.8) * randsign();
        f.dz = bellrand(1.8) * randsign();
    }

    f.zr = frand(180.0) as f32;
    if do_spin || do_wander {
        f.y = 0.0;
        if n > 2 {
            f.y += frand(3.0) as f32 * randsign();
        }
    }
}

impl CompanionCube {
    /// `tick_floater`. With spin or wander on, nothing is thrown at all: the
    /// cubes hang where they were put and turn on the spot.
    fn tick_floater(&mut self, i: usize) {
        if self.trackball.button_down() || self.do_spin || self.do_wander {
            return;
        }
        let speed = self.speed;
        let n = self.floaters.len();
        let f = &mut self.floaters[i];
        f.dx += f.ddx;
        f.dy += f.ddy;
        f.dz += f.ddz;

        f.x += f.dx * speed * SPEED_SCALE;
        f.y += f.dy * speed * SPEED_SCALE;
        f.z += f.dz * speed * SPEED_SCALE;

        if f.y < -BOTTOM
            || f.x < -BOTTOM * 8.0
            || f.x > BOTTOM * 8.0
            || f.z < -BOTTOM * 8.0
            || f.z > BOTTOM * 8.0
        {
            reset_floater(f, n, speed, self.do_spin, self.do_wander);
        }
    }

    /// `build_corner`: one of the rounded corner pieces.
    fn build_corner(&self, g: &mut Gl) {
        g.glx.push_matrix();
        g.glx.translate(-0.5, -0.5, -0.5);
        let s = 0.659;
        g.glx.scale(s, s, s);

        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        g.glx.translate(-0.12, -1.64, 0.12);
        self.quad.render(&mut g.glx, self.wireframe);
        g.glx.pop_matrix();
    }

    /// `build_face`: the flat plate with its groove, the four corners, the
    /// disc and the heart inside it.
    fn build_face(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let base_color = [0.53, 0.60, 0.66, 1.00];
        let heart_color = [0.92, 0.67, 1.00, 1.00];
        let disc_color = [0.75, 0.92, 1.00, 1.00];
        let corner_color = [0.75, 0.92, 1.00, 1.00];

        if !wire {
            let w = 0.010;
            g.glx.material_ambient_diffuse(base_color);
            g.glx.push_matrix();
            g.glx.normal3f(0.0, 0.0, -1.0);
            g.glx.translate(-0.5, -0.5, -0.5);

            // The four quarters of the plate, with the groove between them.
            g.glx.begin(Shape::Quads);
            for q in [
                [
                    [0.0, 0.0],
                    [0.0, 0.5 - w],
                    [0.5 - w, 0.5 - w],
                    [0.5 - w, 0.0],
                ],
                [
                    [0.5 + w, 0.0],
                    [0.5 + w, 0.5 - w],
                    [1.0, 0.5 - w],
                    [1.0, 0.0],
                ],
                [
                    [0.0, 0.5 + w],
                    [0.0, 1.0],
                    [0.5 - w, 1.0],
                    [0.5 - w, 0.5 + w],
                ],
                [
                    [0.5 + w, 0.5 + w],
                    [0.5 + w, 1.0],
                    [1.0, 1.0],
                    [1.0, 0.5 + w],
                ],
            ] {
                for v in q {
                    g.glx.vertex3f(v[0], v[1], 0.0);
                }
            }
            g.glx.end();

            g.glx.material_ambient_diffuse(heart_color);

            // The four walls of the groove, each facing into it.
            for (n, q) in [
                (
                    [0.0, -1.0, 0.0],
                    [
                        [0.0, 0.5 + w, 0.0],
                        [1.0, 0.5 + w, 0.0],
                        [1.0, 0.5 + w, w],
                        [0.0, 0.5 + w, w],
                    ],
                ),
                (
                    [0.0, 1.0, 0.0],
                    [
                        [0.0, 0.5 - w, w],
                        [1.0, 0.5 - w, w],
                        [1.0, 0.5 - w, 0.0],
                        [0.0, 0.5 - w, 0.0],
                    ],
                ),
                (
                    [-1.0, 0.0, 0.0],
                    [
                        [0.5 + w, 0.0, w],
                        [0.5 + w, 1.0, w],
                        [0.5 + w, 1.0, 0.0],
                        [0.5 + w, 0.0, 0.0],
                    ],
                ),
                (
                    [1.0, 0.0, 0.0],
                    [
                        [0.5 - w, 0.0, 0.0],
                        [0.5 - w, 1.0, 0.0],
                        [0.5 - w, 1.0, w],
                        [0.5 - w, 0.0, w],
                    ],
                ),
            ] {
                g.glx.normal3f(n[0], n[1], n[2]);
                g.glx.begin(Shape::Quads);
                for v in q {
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
                g.glx.end();
            }

            // The floor of the groove, which is the whole face set back.
            g.glx.material_ambient_diffuse(heart_color);
            g.glx.normal3f(0.0, 0.0, -1.0);
            g.glx.translate(0.0, 0.0, w);
            g.glx.begin(Shape::Quads);
            for v in [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]] {
                g.glx.vertex3f(v[0], v[1], 0.0);
            }
            g.glx.end();

            g.glx.pop_matrix();
        }

        g.glx.material_ambient_diffuse(corner_color);

        g.glx.push_matrix();
        for i in 0..4 {
            self.build_corner(g);
            if i < 3 {
                g.glx.rotate(90.0, 0.0, 0.0, 1.0);
            }
        }

        g.glx.rotate(90.0, 0.0, 0.0, 1.0);
        g.glx.translate(0.585, -0.585, -0.5655);

        let s = 10.5;
        g.glx.scale(s, s, s);
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);

        if !wire {
            g.glx.material_ambient_diffuse(heart_color);
            self.heart.render(&mut g.glx, wire);
        }

        g.glx.material_ambient_diffuse(disc_color);
        self.disc.render(&mut g.glx, wire);

        g.glx.pop_matrix();
    }

    /// `build_cube`: the same face six times over.
    fn build_cube(&self, g: &mut Gl) {
        g.glx.push_matrix();
        for (i, (angle, axis)) in [
            (90.0, [0.0, 1.0, 0.0]),
            (90.0, [0.0, 1.0, 0.0]),
            (90.0, [0.0, 1.0, 0.0]),
            (90.0, [1.0, 0.0, 0.0]),
            (180.0, [1.0, 0.0, 0.0]),
            (0.0, [1.0, 0.0, 0.0]),
        ]
        .iter()
        .enumerate()
        {
            self.build_face(g);
            if i < 5 {
                g.glx.rotate(*angle, axis[0], axis[1], axis[2]);
            }
        }
        g.glx.pop_matrix();
    }

    fn draw_floater(&mut self, g: &mut Gl, i: usize) {
        let down = self.trackball.button_down();
        let do_spin = self.do_spin;
        let f = &mut self.floaters[i];
        let (px, py, pz) = f.rot.position(!down);
        // Upstream reads the position and then, only when spinning, reads the
        // rotation over the top of it, so a lone spinner turns by the numbers
        // that were meant to be a position.
        let (rx, ry, rz) = if do_spin {
            f.rot.rotation(!down)
        } else {
            (px, py, pz)
        };
        let (fx, fy, fz, zr, spinner_p) = (f.x, f.y, f.z, f.zr, f.spinner_p);

        g.glx.push_matrix();
        g.glx.translate(fx, fy, fz);

        if self.do_wander {
            g.glx.translate(px as f32, py as f32, pz as f32);
        }

        if self.do_spin || spinner_p {
            g.glx.rotate(rx as f32 * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(ry as f32 * 360.0, 0.0, 1.0, 0.0);
            g.glx.rotate(rz as f32 * 360.0, 0.0, 0.0, 1.0);
        } else {
            g.glx.rotate(zr * 360.0, 0.0, 1.0, 0.0);
        }

        let n = self.floaters.len();
        let mut s = 1.5;
        if n > 99 {
            s *= 0.05;
        } else if n > 25 {
            s *= 0.18;
        } else if n > 9 {
            s *= 0.3;
        } else if n > 1 {
            s *= 0.7;
        }
        s *= 2.0;
        if (self.do_spin || self.do_wander) && n > 1 {
            s *= 0.7;
        }
        g.glx.scale(s, s, s);

        self.build_cube(g);
        g.glx.pop_matrix();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let do_spin = g.res.bool("spin");
    let do_wander = g.res.bool("wander");
    let speed = (g.res.float("speed") as f32).max(0.001);
    let nfloaters = g.res.int("count").max(1) as usize;

    let mut this = CompanionCube {
        trackball: Trackball::new(),
        floaters: Vec::with_capacity(nfloaters),
        quad: GlList::parse(crate::models::COMPANION_QUAD),
        disc: GlList::parse(crate::models::COMPANION_DISC),
        heart: GlList::parse(crate::models::COMPANION_HEART),
        speed,
        do_spin,
        do_wander,
        wireframe: g.res.bool("wireframe"),
    };

    for i in 0..nfloaters {
        let spin_speed = if do_spin { 0.7 } else { 10.0 };
        let wander_speed = if do_wander {
            0.02
        } else {
            (0.05 * speed * SPEED_SCALE) as f64
        };
        let spin_accel = 0.5;
        let mut f = Floater {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            ix: 0.0,
            iz: 0.0,
            dx: 0.0,
            dy: 0.0,
            dz: 0.0,
            ddx: 0.0,
            ddy: 0.0,
            ddz: 0.0,
            zr: 0.0,
            rot: Rotator::new(
                spin_speed,
                spin_speed,
                spin_speed,
                spin_accel,
                wander_speed,
                true,
            ),
            spinner_p: false,
        };
        if nfloaters == 2 {
            f.x = if i != 0 { 2.0 } else { -2.0 };
        } else if i != 0 {
            let th = (i - 1) as f64 * std::f64::consts::PI * 2.0 / (nfloaters - 1) as f64;
            let r = 3.0;
            f.x = (r * th.cos()) as f32;
            f.z = (r * th.sin()) as f32;
        }
        f.ix = f.x;
        f.iz = f.z;
        reset_floater(&mut f, nfloaters, speed, do_spin, do_wander);
        this.floaters.push(f);
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for CompanionCube {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let (mut height, mut y) = (height, 0);
        let mut h = height as f32 / width as f32;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.7, 0.2, 0.4, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        }

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.scale(2.0, 2.0, 2.0);

        for i in 0..self.floaters.len() {
            self.draw_floater(g, i);
            self.tick_floater(i);
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*count:        3",
    "*wireframe:    False",
    "*speed:        1.0",
    "*spin:         False",
    "*wander:       False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Bounce", 0.05, 2.0, 0.05, 2, "1.0"),
    Opt::slider("count", "Number of cubes", 1.0, 20.0, 1.0, 0, "3"),
    Opt::boolean("spin", "Spin", "false"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "companioncube",
    label: "Companion Cube",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2011",
        video: Some("https://www.youtube.com/watch?v=Q54NVuxhGso"),
        blurb: "The Enrichment Center reminds you that the weighted companion \
                cube will never threaten to stab you and, in fact, cannot \
                speak. In the event that the Weighted Companion Cube does \
                speak, the Enrichment Center urges you to disregard its advice.",
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
    fn a_cube_is_six_faces_of_the_same_thing() {
        // Whatever a face is worth, the cube is exactly six of them, and the
        // three modelled shapes appear four, one and one time in each.
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260812));
        r.step();
        let f = r.frame();
        let verts: usize = f.batches.iter().map(|b| b.count).sum();
        let quad = GlList::parse(crate::models::COMPANION_QUAD).points;
        let disc = GlList::parse(crate::models::COMPANION_DISC).points;
        let heart = GlList::parse(crate::models::COMPANION_HEART).points;
        // Nine flat quads a face as well, each cut into two triangles.
        let plate = 9 * 6;
        assert_eq!(verts, 6 * (4 * quad + disc + heart + plate), "{verts}");
    }

    #[test]
    fn they_are_thrown_up_from_below_and_fall_back_out() {
        // Nothing is ever visible below the bottom, and every cube that
        // reaches it is thrown again rather than left there.
        let mut r = start(StartArgs::new(640, 480, "count=2&speed=2", 20260812));
        let mut rose = 0;
        let mut last = f32::MIN;
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            let mut top = f32::MIN;
            for b in &f.batches {
                for v in &f.vertices[b.first..b.first + b.count] {
                    top = top.max(b.mvp.transform(v.pos)[1]);
                }
            }
            if top > last {
                rose += 1;
            }
            last = top;
        }
        assert!(rose > 100, "nothing ever came back up: {rose} of 600");
    }

    #[test]
    fn spinning_holds_them_still_instead_of_throwing_them() {
        // With spin or wander on there is no gravity at all: the cubes stay
        // where they were put and turn on the spot.
        let mut r = start(StartArgs::new(640, 480, "count=1&spin=true", 20260812));
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for _ in 0..300 {
            r.step();
            let f = r.frame();
            for b in &f.batches {
                for v in &f.vertices[b.first..b.first + b.count] {
                    let y = b.mvp.transform(v.pos)[1];
                    lo = lo.min(y);
                    hi = hi.max(y);
                }
            }
        }
        // It turns, so its extent moves a little, but it never leaves the
        // screen the way a thrown one does.
        assert!(lo > -1.5 && hi < 1.5, "{lo} to {hi}");
    }

    #[test]
    fn more_cubes_are_drawn_smaller() {
        // Each cube writes the same number of vertices, so the first one's
        // worth of them is the first cube; the rest are spread out around it
        // and would swamp the measurement.
        let one_cube = 6
            * (4 * GlList::parse(crate::models::COMPANION_QUAD).points
                + GlList::parse(crate::models::COMPANION_DISC).points
                + GlList::parse(crate::models::COMPANION_HEART).points
                + 9 * 6);
        let size = |count: i32| {
            let mut r = start(StartArgs::new(
                640,
                480,
                &format!("count={count}&spin=true"),
                20260812,
            ));
            r.step();
            let f = r.frame();
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for b in f.batches.iter().filter(|b| b.first < one_cube) {
                for v in &f.vertices[b.first..b.first + b.count] {
                    let y = b.mvp.transform(v.pos)[1];
                    lo = lo.min(y);
                    hi = hi.max(y);
                }
            }
            hi - lo
        };
        // One fills the screen; twenty of them share it.
        assert!(size(1) > size(20) * 3.0, "{} vs {}", size(1), size(20));
    }
}
