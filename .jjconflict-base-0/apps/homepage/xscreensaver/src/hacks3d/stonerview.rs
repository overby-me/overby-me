//! Port of `hacks/glx/stonerview.c`, `stonerview-view.c`,
//! `stonerview-move.c` and `stonerview-osc.c`.
//!
//! ```text
//! StonerView: An eccentric visual toy.
//! Copyright 1998-2021 by Andrew Plotkin (erkyrath@eblong.com)
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Ported away from GLUT (so that it can do `-root' and work with xscreensaver)
//! by Jamie Zawinski <jwz@jwz.org>, 22-Jan-2001.
//! ```
//!
//! Chains of colorful squares dance around in spirals. Inspired by the classic
//! SGI "ElectroPaint" screen saver from the 1980s.
//!
//! There is nothing on screen but forty flat squares, all the same size, each
//! outlined in grey. Everything the saver does it does by deciding where to put
//! them, and that is done by a little machine of its own, in [`Oscillators`]:
//! a graph of small integer generators, each of which turns a step of the clock
//! into a number, some of them by asking others. Four of them come out at the
//! top, giving each square its angle, its distance from the axis, its height
//! and its colour, and the picture is whatever those four happen to be doing.
//!
//! The generator that makes it look alive rather than merely periodic is
//! `Buffer`, which answers for square *n* with what its source said *n* steps
//! ago. So a change made once at the top of the chain arrives at each square in
//! turn, and the chain ripples. Everything else, the wraps and bounces and the
//! multiplexer that switches between four of them at random intervals, is there
//! to keep feeding it something new.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
};

/// Forty polygons at a time.
const NUM_ELS: usize = 40;

/// Some of the osc functions switch between P alternatives. We arbitrarily
/// choose P=4.
const NUM_PHASES: usize = 4;

/// Where an oscillator lives in [`Oscillators::all`]. Upstream threads them on
/// a linked list in creation order; the index is the same thing, and the order
/// is load bearing, so a `Buffer` reading its source on a step reads a source
/// that has already been stepped.
type OscId = usize;

/// One generator. `f(i, n)`, for the current step `i` and the square `n`.
enum Osc {
    /// Always the same value.
    Constant(i32),
    /// Slides up and down between two ends, turning round at each.
    Bounce {
        min: i32,
        max: i32,
        step: i32,
        val: i32,
    },
    /// Slides one way between two ends and jumps back to the far one.
    Wrap {
        min: i32,
        max: i32,
        step: i32,
        val: i32,
    },
    /// A wrap whose step is itself a generator.
    VeloWrap {
        min: i32,
        max: i32,
        step: OscId,
        val: i32,
    },
    /// Counts out a run of each of the four phases in turn, picking a new
    /// length for every one. Upstream also has a fixed-length phaser, which
    /// nothing in this saver builds, so it is not here.
    RandPhaser {
        min_len: i32,
        max_len: i32,
        count: i32,
        cur_len: i32,
        curphase: i32,
    },
    /// `a(i, n) + n * b(i, n)`: the easy way to make a run of squares into a
    /// sequence rather than a heap.
    Linear { base: OscId, diff: OscId },
    /// `g(i - n, 0)`: what the source said `n` steps ago, so a change ripples
    /// down the chain of squares instead of hitting them all at once.
    Buffer {
        val: OscId,
        firstel: usize,
        el: Box<[i32; NUM_ELS]>,
    },
    /// Four generators and a selector that says which of them to ask.
    Multiplex {
        sel: OscId,
        val: [OscId; NUM_PHASES],
    },
}

/// A random number between min and max, inclusive.
fn rand_range(min: i32, max: i32) -> i32 {
    let diff = (max + 1) - min;
    if diff <= 1 {
        return min;
    }
    min + (random() % diff as u32) as i32
}

/// The whole machine: every generator ever made, in the order they were made.
#[derive(Default)]
struct Oscillators {
    all: Vec<Osc>,
}

impl Oscillators {
    fn add(&mut self, osc: Osc) -> OscId {
        self.all.push(osc);
        self.all.len() - 1
    }

    fn constant(&mut self, val: i32) -> OscId {
        self.add(Osc::Constant(val))
    }

    /// Both of the sliding generators start somewhere random between their
    /// ends, on a whole number of steps from the bottom.
    fn start_between(min: i32, max: i32, step: i32) -> i32 {
        let step = step.abs();
        let diff = (max - min) / step;
        min + step * rand_range(0, diff - 1)
    }

    fn bounce(&mut self, min: i32, max: i32, step: i32) -> OscId {
        let val = Self::start_between(min, max, step);
        self.add(Osc::Bounce {
            min,
            max,
            step,
            val,
        })
    }

    fn wrap(&mut self, min: i32, max: i32, step: i32) -> OscId {
        let val = Self::start_between(min, max, step);
        self.add(Osc::Wrap {
            min,
            max,
            step,
            val,
        })
    }

    fn velowrap(&mut self, min: i32, max: i32, step: OscId) -> OscId {
        let val = rand_range(min, max);
        self.add(Osc::VeloWrap {
            min,
            max,
            step,
            val,
        })
    }

    fn randphaser(&mut self, min_len: i32, max_len: i32) -> OscId {
        let cur_len = rand_range(min_len, max_len);
        let curphase = rand_range(0, NUM_PHASES as i32 - 1);
        self.add(Osc::RandPhaser {
            min_len,
            max_len,
            count: 0,
            cur_len,
            curphase,
        })
    }

    fn linear(&mut self, base: OscId, diff: OscId) -> OscId {
        self.add(Osc::Linear { base, diff })
    }

    fn multiplex(&mut self, sel: OscId, val: [OscId; NUM_PHASES]) -> OscId {
        self.add(Osc::Multiplex { sel, val })
    }

    /// The ring starts full of the source's current value, so the first few
    /// steps do not read a hole.
    fn buffer(&mut self, val: OscId) -> OscId {
        let now = self.get(val, 0);
        self.add(Osc::Buffer {
            val,
            firstel: NUM_ELS - 1,
            el: Box::new([now; NUM_ELS]),
        })
    }

    /// `f(i, el)` for the current `i`.
    fn get(&self, osc: OscId, el: usize) -> i32 {
        match &self.all[osc] {
            Osc::Constant(v) => *v,
            Osc::Bounce { val, .. } | Osc::Wrap { val, .. } | Osc::VeloWrap { val, .. } => *val,
            Osc::Linear { base, diff } => self.get(*base, el) + el as i32 * self.get(*diff, el),
            Osc::Multiplex { sel, val } => {
                let s = self.get(*sel, el);
                self.get(val[s.unsigned_abs() as usize % NUM_PHASES], el)
            }
            Osc::RandPhaser { curphase, .. } => *curphase,
            Osc::Buffer {
                firstel, el: ring, ..
            } => ring[(firstel + el) % NUM_ELS],
        }
    }

    /// Step `i`. Every generator moves, in the order they were made, which is
    /// why a `Buffer` can take its source's value for this step rather than the
    /// last one.
    fn increment(&mut self) {
        for i in 0..self.all.len() {
            // What this one needs from the others, read before it is touched.
            let feed = match &self.all[i] {
                Osc::VeloWrap { step, .. } => Some(self.get(*step, 0)),
                Osc::Buffer { val, .. } => Some(self.get(*val, 0)),
                _ => None,
            };
            match &mut self.all[i] {
                Osc::Bounce {
                    min,
                    max,
                    step,
                    val,
                } => {
                    *val += *step;
                    if *val < *min && *step < 0 {
                        *step = -*step;
                        *val = *min + (*min - *val);
                    }
                    if *val > *max && *step > 0 {
                        *step = -*step;
                        *val = *max + (*max - *val);
                    }
                }
                Osc::Wrap {
                    min,
                    max,
                    step,
                    val,
                } => {
                    *val += *step;
                    if *val < *min && *step < 0 {
                        *val += *max - *min;
                    }
                    if *val > *max && *step > 0 {
                        *val -= *max - *min;
                    }
                }
                Osc::VeloWrap { min, max, val, .. } => {
                    let diff = *max - *min;
                    *val += feed.unwrap_or(0);
                    while *val < *min {
                        *val += diff;
                    }
                    while *val > *max {
                        *val -= diff;
                    }
                }
                Osc::RandPhaser {
                    min_len,
                    max_len,
                    count,
                    cur_len,
                    curphase,
                } => {
                    *count += 1;
                    if *count >= *cur_len {
                        *count = 0;
                        *cur_len = rand_range(*min_len, *max_len);
                        *curphase = (*curphase + 1) % NUM_PHASES as i32;
                    }
                }
                Osc::Buffer { firstel, el, .. } => {
                    *firstel = (*firstel + NUM_ELS - 1) % NUM_ELS;
                    el[*firstel] = feed.unwrap_or(0);
                }
                Osc::Constant(_) | Osc::Linear { .. } | Osc::Multiplex { .. } => {}
            }
        }
    }
}

/// One square: where its middle is, which way it is turned, and what colour.
#[derive(Clone, Copy, Default)]
struct Elem {
    pos: [f32; 3],
    vervec: [f32; 2],
    col: [f32; 4],
}

struct StonerView {
    trackball: Trackball,
    wireframe: bool,
    transparent: bool,
    elist: [Elem; NUM_ELS],

    osc: Oscillators,
    /// Angle around the axis, in hundredths of a degree, so 0 to 36000.
    theta: OscId,
    /// Distance from the axis, up to 1000. Negative is allowed and just means
    /// the other side of the circle.
    rad: OscId,
    /// Height, -1000 to 1000.
    alti: OscId,
    /// An angle around the colour wheel, in tenths of a degree.
    color: OscId,
}

const VIEW_ROTX: f32 = -45.0;
const VIEW_ROTY: f32 = 0.0;
const VIEW_ROTZ: f32 = 15.0;
const VIEW_SCALE: f32 = 4.0;

impl StonerView {
    /// Build the machine. This shape is the saver: everything it does is here.
    fn init_move(&mut self) {
        let o = &mut self.osc;

        let phase = o.randphaser(300, 600);
        let (c25, c75, c50, c100) = (
            o.constant(25),
            o.constant(75),
            o.constant(50),
            o.constant(100),
        );
        let speed = o.multiplex(phase, [c25, c75, c50, c100]);
        let base = o.velowrap(0, 36000, speed);

        let p2 = o.randphaser(300, 600);
        let b0 = o.buffer(p2);
        let w1 = o.wrap(0, 36000, 10);
        let b1 = o.buffer(w1);
        let w2 = o.wrap(0, 36000, -8);
        let b2 = o.buffer(w2);
        let b3 = o.wrap(0, 36000, 4);
        let bo = o.bounce(-2000, 2000, 20);
        let b4 = o.buffer(bo);
        let diff = o.multiplex(b0, [b1, b2, b3, b4]);
        self.theta = o.linear(base, diff);

        let rp = o.randphaser(250, 500);
        let r0 = o.bounce(-1000, 1000, 10);
        let r1 = o.bounce(200, 1000, -15);
        let r2 = o.bounce(400, 1000, 10);
        let r3 = o.bounce(-1000, 1000, -20);
        let rmux = o.multiplex(rp, [r0, r1, r2, r3]);
        self.rad = o.buffer(rmux);

        // A constant base and a constant difference, which is what stacks the
        // squares evenly from the bottom of the cylinder to the top.
        let bottom = o.constant(-1000);
        let apart = o.constant(2000 / NUM_ELS as i32);
        self.alti = o.linear(bottom, apart);

        let cp = o.randphaser(150, 300);
        let cb = o.buffer(cp);
        let c0 = o.wrap(0, 3600, 13);
        let cb0 = o.buffer(c0);
        let c1 = o.wrap(0, 3600, 32);
        let cb1 = o.buffer(c1);
        let c2 = o.wrap(0, 3600, 17);
        let cb2 = o.buffer(c2);
        let c3 = o.wrap(0, 3600, 7);
        let cb3 = o.buffer(c3);
        self.color = o.multiplex(cb, [cb0, cb1, cb2, cb3]);

        self.move_increment();
    }

    /// Ask the four top generators where every square goes.
    fn move_increment(&mut self) {
        for ix in 0..NUM_ELS {
            let pttheta =
                f64::from(self.osc.get(self.theta, ix)) * (0.01 * std::f64::consts::PI / 180.0);
            let ptrad = f64::from(self.osc.get(self.rad, ix)) * 0.001;

            let el = &mut self.elist[ix];
            el.pos = [
                (ptrad * pttheta.cos()) as f32,
                (ptrad * pttheta.sin()) as f32,
                (f64::from(self.osc.get(self.alti, ix)) * 0.001) as f32,
            ];

            // Which way the square is rotated. Fixed, though it would be
            // trivial to make them spin as they revolve.
            el.vervec = [0.11, 0.0];

            // HSV to RGB, for an S and a V that are always one, which is three
            // straight ramps rather than anything worth a helper.
            let val = f32::from(self.osc.get(self.color, ix) as i16);
            el.col = if val < 1200.0 {
                [val / 1200.0, 0.0, (1200.0 - val) / 1200.0, 1.0]
            } else if val < 2400.0 {
                [(2400.0 - val) / 1200.0, (val - 1200.0) / 1200.0, 0.0, 1.0]
            } else {
                [0.0, (3600.0 - val) / 1200.0, (val - 2400.0) / 1200.0, 1.0]
            };
        }
        self.osc.increment();
    }

    /// The four corners of a square, turned by its `vervec`.
    fn corners(el: &Elem) -> [[f32; 3]; 4] {
        let (p, v) = (el.pos, el.vervec);
        [
            [p[0] - v[0], p[1] - v[1], p[2]],
            [p[0] + v[1], p[1] - v[0], p[2]],
            [p[0] + v[0], p[1] + v[1], p[2]],
            [p[0] - v[1], p[1] + v[0], p[2]],
        ]
    }
}

impl Hack3d for StonerView {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        g.glx.push_matrix();
        g.glx.scale(VIEW_SCALE, VIEW_SCALE, VIEW_SCALE);
        g.glx.rotate(VIEW_ROTX, 1.0, 0.0, 0.0);
        g.glx.rotate(VIEW_ROTY, 0.0, 1.0, 0.0);
        g.glx.rotate(VIEW_ROTZ, 0.0, 0.0, 1.0);

        let outline = if self.wireframe {
            [1.0, 1.0, 1.0, 1.0]
        } else {
            [0.6, 0.6, 0.6, 1.0]
        };

        for ix in 0..NUM_ELS {
            let el = self.elist[ix];
            let c = Self::corners(&el);
            g.glx.normal3f(0.0, 0.0, 1.0);

            g.glx.material_ambient_diffuse(outline);
            g.glx.begin(Shape::LineLoop);
            for v in c {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();

            if self.wireframe {
                continue;
            }

            g.glx.material_ambient_diffuse(el.col);
            g.glx.begin(Shape::Quads);
            for v in c {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();
        }

        g.glx.pop_matrix();

        if !self.trackball.button_down() {
            self.move_increment();
        }
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width.max(1) as f32;
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -h, h, 5.0, 60.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        // Set once and left there: every frame's own matrix work is pushed and
        // popped on top of this.
        g.glx.translate(0.0, 0.0, -40.0);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut st = StonerView {
        trackball: Trackball::new(),
        wireframe: g.res.bool("wireframe"),
        transparent: g.res.bool("transparent"),
        elist: [Elem::default(); NUM_ELS],
        osc: Oscillators::default(),
        theta: 0,
        rad: 0,
        alti: 0,
        color: 0,
    };

    g.glx.cull_face(true);
    g.glx.depth_test(true);
    if !st.wireframe {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
    }
    // Blending is on either way upstream, but with the default function, which
    // is the same as not blending at all. Only the translucent knob asks for
    // one that does something: colours add, so where the squares of a spiral
    // overlap they light up.
    g.glx.blend(if st.transparent {
        Blend::AlphaAdd
    } else {
        Blend::Off
    });

    st.init_move();

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*transparent:  True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::boolean("transparent", "Translucent", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "stonerview",
    label: "Stoner View",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Andrew Plotkin",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=xvDK_wwnXWs"),
        blurb: "Chains of colorful squares dance around in spirals. \
                Inspired by the classic SGI \"ElectroPaint\" screen saver \
                from the 1980s.",
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
    use crate::runtime::gl::Primitive;

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260811));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// A buffer answers for square n with what its source said n steps ago, so
    /// a run of them holds the source's recent history in order.
    #[test]
    fn a_buffer_remembers_backwards() {
        let mut o = Oscillators::default();
        // A wrap that climbs by one from a known place, so the history is
        // simply consecutive numbers.
        let src = o.add(Osc::Wrap {
            min: 0,
            max: 1_000_000,
            step: 1,
            val: 100,
        });
        let buf = o.buffer(src);

        // Fresh, every slot holds the one value the source has had.
        assert!((0..NUM_ELS).all(|n| o.get(buf, n) == 100));

        for _ in 0..NUM_ELS {
            o.increment();
        }
        // Now slot 0 is the newest and each one after it is a step older.
        for n in 0..NUM_ELS {
            assert_eq!(
                o.get(buf, n),
                100 + NUM_ELS as i32 - n as i32,
                "slot {n} is not {n} steps behind"
            );
        }
    }

    /// The three sliding generators keep to their ends: a bounce turns round, a
    /// wrap jumps back, and a velowrap does the same on a step it is given.
    #[test]
    fn nothing_slides_out_of_its_range() {
        let mut o = Oscillators::default();
        let bounce = o.bounce(-1000, 1000, 37);
        let wrap = o.wrap(0, 3600, -13);
        let speed = o.constant(700);
        let velo = o.velowrap(0, 36000, speed);

        for _ in 0..5000 {
            o.increment();
            assert!((-1000..=1000).contains(&o.get(bounce, 0)));
            assert!((0..=3600).contains(&o.get(wrap, 0)));
            assert!((0..=36000).contains(&o.get(velo, 0)));
        }
    }

    /// Every square is drawn twice, as an outline and as a fill, and the
    /// outline is the same grey for all of them while the fill is not.
    #[test]
    fn forty_squares_outlined_and_filled() {
        let r = run("", 30);
        let f = r.frame();

        let loops: Vec<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::LineLoop)
            .collect();
        assert_eq!(loops.len(), NUM_ELS, "one outline a square");
        assert!(
            loops.iter().all(|b| b.count == 4),
            "an outline is four corners"
        );
        assert!(
            loops
                .iter()
                .all(|b| b.material.ambient_diffuse == [0.6, 0.6, 0.6, 1.0]),
            "the outlines are not all the same grey"
        );

        let fills: Vec<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
            .collect();
        assert_eq!(fills.len(), NUM_ELS, "one fill a square");
        let colours: std::collections::BTreeSet<_> = fills
            .iter()
            .map(|b| format!("{:?}", b.material.ambient_diffuse))
            .collect();
        assert!(colours.len() > 4, "the squares are all the same colour");
    }

    /// Wireframe draws the outlines only, and in white rather than grey.
    #[test]
    fn wireframe_is_outlines_alone() {
        let r = run("wireframe=true", 5);
        let f = r.frame();
        assert!(
            f.batches.iter().all(|b| b.primitive == Primitive::LineLoop),
            "something was filled in in wireframe"
        );
        assert!(
            f.batches
                .iter()
                .all(|b| b.material.ambient_diffuse == [1.0; 4])
        );
    }

    /// The squares climb the cylinder in order, which is what makes a chain
    /// rather than a cloud: `alti` is a linear whose difference is a constant.
    #[test]
    fn the_squares_stack_evenly_up_the_axis() {
        let r = run("", 100);
        let f = r.frame();
        let mut heights: Vec<f32> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::LineLoop)
            .map(|b| f.vertices[b.first].pos[2])
            .collect();
        assert_eq!(heights.len(), NUM_ELS);

        // Evenly spaced, and spanning the cylinder from bottom to top.
        let gaps: Vec<f32> = heights.windows(2).map(|w| w[1] - w[0]).collect();
        let first = gaps[0];
        assert!(first > 0.0, "the chain does not climb");
        assert!(
            gaps.iter().all(|g| (g - first).abs() < 1e-5),
            "the squares are not evenly spaced"
        );
        heights.sort_by(f32::total_cmp);
        assert!(heights[0] < -0.9 && heights[NUM_ELS - 1] > 0.8);
    }

    /// The translucent knob is what makes overlapping squares light up rather
    /// than hide each other.
    #[test]
    fn translucency_is_what_adds() {
        let clear = run("transparent=false", 3);
        assert!(clear.frame().batches.iter().all(|b| b.blend == Blend::Off));
        let lit = run("transparent=true", 3);
        assert!(
            lit.frame()
                .batches
                .iter()
                .all(|b| b.blend == Blend::AlphaAdd)
        );
    }
}
