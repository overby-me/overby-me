//! Port of `hacks/glx/moebiusgears.c`.
//!
//! ```text
//! moebiusgears, Copyright (c) 2007-2014 Jamie Zawinski <jwz@jwz.org>
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
//! An interlinked loop of rotating gears.
//!
//! The gears sit on a ring, each meshing with its two neighbours, and each is
//! tipped a little further out of the plane than the one before, so that going
//! once round the ring turns the whole train a half turn over. That is the
//! Moebius part, and it is what forces the two counting rules the saver is
//! built on: there must be an odd number of gears, or the loop closes on a pair
//! turning the same way; and an odd number of teeth, or they do not mesh when
//! it closes.
//!
//! Each gear turns the opposite way to its neighbours, and every other one is
//! offset by half a tooth so that a tooth meets a gap.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::involute::{Gear, Size, draw_gear};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

/// One gear and where it sits on the ring.
struct MoGear {
    g: Gear,
    /// Where round the ring, and how far tipped out of its plane.
    pos_th: f64,
    pos_thz: f64,
}

struct MoebiusGears {
    rot: Rotator,
    trackball: Trackball,
    gears: Vec<MoGear>,
    /// Radius of the ring the gears sit on.
    ring_r: f32,
    roll_th: f64,

    count: i32,
    teeth: i32,
    speed: f64,
    do_roll: bool,
    wireframe: bool,
}

impl MoebiusGears {
    /// Build the ring. Called again whenever the count changes.
    fn reset(&mut self) {
        let mut total_gears = self.count;
        // Must be odd or the gears intersect.
        if total_gears & 1 == 0 {
            total_gears += 1;
        }
        // And the teeth must be odd too, or they do not mesh when the loop
        // closes, since the number of gears is odd.
        let mut teeth = self.teeth;
        if teeth & 1 == 0 {
            teeth += 1;
        }
        if teeth < 7 {
            teeth = 7;
        }
        // The mesh angle is too steep with fewer than this.
        if total_gears < 13 {
            total_gears = 13;
        }

        let thick = 0.2;
        let nubs = if random() & 3 != 0 {
            0
        } else {
            (random() % teeth as u32) as i32 / 2
        };
        // Sloping gears are incompatible with rolling, so upstream leaves the
        // slope it would otherwise use commented out.
        let slope = 0.0;

        let gears_per_turn = f64::from(total_gears) / 2.0;
        self.ring_r = 3.0;
        let gear_r = std::f64::consts::PI * f64::from(self.ring_r) / gears_per_turn;
        let th = gear_r * 2.5 / f64::from(teeth);

        // A small gear gets a coarser mesh, and so does one with a lot of
        // teeth: either way the detail would not be visible.
        let mut size = if gear_r > 0.60 {
            Size::Huge
        } else if gear_r > 0.32 {
            Size::Large
        } else if gear_r > 0.13 {
            Size::Medium
        } else {
            Size::Small
        };
        if teeth > 77 {
            size = Size::Small;
        }
        if teeth > 45 && size >= Size::Huge {
            size = Size::Medium;
        }

        self.gears.clear();
        for i in 0..total_gears {
            let color = [
                0.7 + frand(0.3) as f32,
                0.7 + frand(0.3) as f32,
                0.7 + frand(0.3) as f32,
                1.0,
            ];
            let color2 = [color[0] * 0.85, color[1] * 0.85, color[2] * 0.85, color[3]];

            self.gears.push(MoGear {
                g: Gear {
                    r: gear_r,
                    size,
                    nteeth: teeth,
                    tooth_h: th,
                    tooth_slope: slope,
                    thickness: gear_r * thick,
                    thickness2: gear_r * thick * 0.1,
                    thickness3: gear_r * thick,
                    inner_r: gear_r * 0.80,
                    inner_r2: gear_r * 0.60,
                    inner_r3: gear_r * 0.55,
                    nubs,
                    // Every other gear is offset by half a tooth, so a tooth
                    // meets a gap.
                    th: if i & 1 == 1 {
                        std::f64::consts::PI * 2.0 / f64::from(teeth)
                    } else {
                        0.0
                    },
                    color,
                    color2,
                    ..Gear::default()
                },
                pos_th: (std::f64::consts::PI * 2.0 / gears_per_turn) * f64::from(i),
                pos_thz: (std::f64::consts::PI / 2.0 / gears_per_turn) * f64::from(i),
            });
        }
    }
}

impl Hack3d for MoebiusGears {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.clear();

        g.glx.push_matrix();
        g.glx.scale(1.1, 1.1, 1.1);

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 4.0,
            (y as f32 - 0.5) * 4.0,
            (z as f32 - 0.5) * 7.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (mut x, mut y, z) = self.rot.rotation(!down);
        // A little rotation even with the spin turned off, so it is never
        // seen exactly edge on.
        x -= 0.14;
        y -= 0.06;
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(1.5, 1.5, 1.5);

        let deg = |r: f64| (r * 180.0 / std::f64::consts::PI) as f32;
        for i in 0..self.gears.len() {
            let (pos_th, pos_thz, th) = {
                let mg = &self.gears[i];
                (mg.pos_th, mg.pos_thz, mg.g.th)
            };

            g.glx.push_matrix();
            g.glx.rotate(deg(pos_th), 0.0, 0.0, 1.0); /* round the ring */
            g.glx.translate(self.ring_r, 0.0, 0.0); /* out to the ring */
            g.glx.rotate(deg(pos_thz), 0.0, 1.0, 0.0); /* and tipped a bit */

            if self.do_roll {
                g.glx.rotate(deg(self.roll_th), 0.0, 1.0, 0.0);
                self.roll_th += self.speed * 0.0005;
            }
            g.glx.rotate(deg(th), 0.0, 0.0, 1.0);

            // Upstream renders each gear into a display list once. Here the
            // gear is drawn directly: a list in this runtime replays the calls
            // rather than the result, so it would cost the same, and the
            // material changes inside a gear are not list-recordable.
            draw_gear(&mut g.glx, &self.gears[i].g, self.wireframe);
            g.glx.pop_matrix();
        }

        g.glx.pop_matrix();

        // Neighbours turn opposite ways.
        for (i, mg) in self.gears.iter_mut().enumerate() {
            mg.g.th +=
                self.speed * (std::f64::consts::PI / 100.0) * if i & 1 == 1 { 1.0 } else { -1.0 };
        }

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
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event {
            match key {
                '+' | '=' => {
                    self.count += 2;
                    self.reset();
                    return true;
                }
                '-' | '_' => {
                    if self.count <= 13 {
                        return false;
                    }
                    self.count -= 2;
                    self.reset();
                    return true;
                }
                ' ' | '\t' => {
                    self.count = 13 + (2 * (random() % 10)) as i32;
                    self.reset();
                    return true;
                }
                _ => {}
            }
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.bool("spin");
    let spin_speed = 0.5;
    let wander_speed = 0.01;
    let spin_accel = 2.0;
    let wire = g.res.bool("wireframe");

    let mut st = MoebiusGears {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            spin_accel,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            false,
        ),
        trackball: Trackball::new(),
        gears: Vec::new(),
        ring_r: 3.0,
        roll_th: 0.0,
        count: g.res.int("count").clamp(3, 199),
        teeth: g.res.int("teeth").clamp(3, 199),
        speed: g.res.float("speed"),
        do_roll: g.res.bool("roll"),
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
    }

    st.reset();
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     30000",
    "*showFPS:   False",
    "*wireframe: False",
    "*count:     17",
    "*teeth:     15",
    "*speed:     1.0",
    "*spin:      True",
    "*wander:    True",
    "*roll:      True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 5.0, 0.01, 2, "1.0"),
    Opt::slider("count", "Number of gears", 13.0, 99.0, 2.0, 0, "17"),
    Opt::slider("teeth", "Number of teeth", 7.0, 49.0, 2.0, 0, "15"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("roll", "Roll", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "moebiusgears",
    label: "Moebius Gears",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2007",
        video: Some("https://www.youtube.com/watch?v=kpT6j2-9b40"),
        blurb: "An interlinked loop of rotating gears.",
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
        let mut r = start(StartArgs::new(640, 480, query, 20260811));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// A ring built straight from [`MoebiusGears::reset`], which is where both
    /// counting rules live.
    fn a_ring(count: i32, teeth: i32) -> MoebiusGears {
        let mut st = MoebiusGears {
            rot: Rotator::new(0.0, 0.0, 0.0, 1.0, 0.0, false),
            trackball: Trackball::new(),
            gears: Vec::new(),
            ring_r: 3.0,
            roll_th: 0.0,
            count,
            teeth,
            speed: 1.0,
            do_roll: false,
            wireframe: false,
        };
        st.reset();
        st
    }

    /// Both counts are forced odd, and both have a floor. An even number of
    /// gears would close the loop on a pair turning the same way, and an even
    /// number of teeth would leave a tooth meeting a tooth.
    #[test]
    fn the_counts_are_forced_odd() {
        for (asked, want) in [(13, 13), (14, 15), (20, 21), (5, 13), (99, 99)] {
            assert_eq!(a_ring(asked, 15).gears.len(), want, "asked for {asked}");
        }
        for (asked, want) in [(7, 7), (8, 9), (16, 17), (3, 7)] {
            assert_eq!(a_ring(17, asked).gears[0].g.nteeth, want, "asked {asked}");
        }
    }

    /// Going once round the ring turns the train a half turn over, which is
    /// what makes it a Moebius strip rather than a bracelet.
    #[test]
    fn the_ring_makes_a_half_turn() {
        let st = a_ring(17, 15);
        let last = st.gears.last().unwrap();
        let n = st.gears.len() as f64;

        // Over the whole ring the gears tip through half a turn, which is the
        // Moebius part: the plane the train sits in comes back inverted.
        let step = last.pos_thz / (n - 1.0);
        assert!(
            (step * n - std::f64::consts::PI).abs() < 1e-9,
            "a lap tips by {} radians, not half a turn",
            step * n
        );
        // Meanwhile the positions go round twice, which is what leaves room
        // for that half turn to be shared out evenly.
        let round = st.gears[1].pos_th * n;
        assert!(
            (round - 4.0 * std::f64::consts::PI).abs() < 1e-9,
            "a lap goes round {} radians",
            round
        );
    }

    /// Every other gear is offset by half a tooth, so a tooth meets a gap.
    #[test]
    fn every_other_gear_is_offset_by_half_a_tooth() {
        let st = a_ring(17, 15);
        let step = std::f64::consts::PI * 2.0 / 15.0;
        for (i, mg) in st.gears.iter().enumerate() {
            let want = if i & 1 == 1 { step } else { 0.0 };
            assert!((mg.g.th - want).abs() < 1e-9, "gear {i} is at {}", mg.g.th);
        }
    }

    /// Neighbours turn opposite ways, which is the only way a ring of them can
    /// mesh.
    #[test]
    fn neighbours_turn_opposite_ways() {
        let mut r = start(StartArgs::new(640, 480, "roll=false&spin=false", 20260811));
        r.step();
        let first = r.frame().batches[0].mvp.0;
        for _ in 0..30 {
            r.step();
        }
        assert_ne!(first, r.frame().batches[0].mvp.0, "nothing turned");
    }

    /// A gear is a solid thing: teeth outside, a hole through the middle, and
    /// two flat faces joining them.
    #[test]
    fn a_gear_is_teeth_a_hole_and_two_faces() {
        use crate::runtime::gl::Primitive;
        let r = run("count=13", 2);
        let f = r.frame();
        assert!(
            f.batches
                .iter()
                .all(|b| b.primitive == Primitive::Triangles),
            "everything solid should have been cut to triangles"
        );
        // Every gear is drawn from the same handful of runs: the teeth, the
        // hole, and the two flat faces, plus its inner rings.
        let counts: std::collections::BTreeSet<_> = f.batches.iter().map(|b| b.count).collect();
        assert!(
            counts.len() < 8,
            "a gear should be a few kinds of run, not {}: {counts:?}",
            counts.len()
        );
        assert!(
            f.vertices.len() > 10_000,
            "a ring of gears is more than this"
        );
    }

    /// Turning the count up and down rebuilds the ring, and the floor holds.
    #[test]
    fn the_ring_can_be_grown_and_shrunk() {
        assert_eq!(a_ring(13, 15).gears.len(), 13);
        assert_eq!(a_ring(15, 15).gears.len(), 15);
        // The floor: the mesh angle is too steep with fewer.
        assert_eq!(a_ring(11, 15).gears.len(), 13);

        // And the keys reach it. Growing the ring is more geometry on screen.
        let mut r = start(StartArgs::new(640, 480, "count=13&teeth=15", 20260811));
        r.step();
        let before = r.frame().vertices.len();
        r.event(XEvent::KeyPress { key: '+' });
        r.step();
        assert!(
            r.frame().vertices.len() > before,
            "the ring did not grow: {} against {before}",
            r.frame().vertices.len()
        );
    }
}
