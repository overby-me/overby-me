//! Port of `hacks/glx/lockward.c`.
//!
//! ```text
//! lockward.c:  First attempt at an Xscreensaver.
//!
//! Leo L. Schwab                                       2007.08.17
//! ****
//! Copyright (c) 2007 Leo L. Schwab
//!
//! Permission is hereby granted, free of charge, to any person obtaining a
//! copy of this software and associated documentation files (the
//! "Software"), to deal in the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to permit
//! persons to whom the Software is furnished to do so, subject to the
//! following conditions:
//!
//! The above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
//! NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//! OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
//! USE OR OTHER DEALINGS IN THE SOFTWARE.
//! ```
//!
//! A translucent spinning, blinking thing: the wards in an old combination
//! lock crossed with a backlit information display that changed colour by
//! polarised light.
//!
//! Four spinners are stacked on top of each other, each twelve pie-wedge
//! blades of its own random inner and outer radius, drawn translucent so the
//! overlaps make the pattern. A spinner does not turn continuously: it picks a
//! whole number of blade-widths to travel and a time to take over it, sits
//! still for a few seconds, and goes again. Turning by an exact division and
//! subtracting a fixed per-frame step from the remaining count rather than
//! adding one up is what keeps it landing exactly on a division however long
//! it has been running.
//!
//! The blinks are ten different sweeps over the same geometry: one blade, all
//! blades in random order, all in sequence one way or the other, two sequences
//! running in opposite directions at once, one ring, all rings, and a scatter
//! of random segments. A blink is drawn with `GL_DST_COLOR, GL_SRC_ALPHA`, so
//! it multiplies what is already on the screen: a flash shows on the spinner
//! and not on the black around it.
//!
//! The scatter is the nicest piece of arithmetic in it. Each blade gets a
//! random bit pattern, and the runs of set bits in it become the segments to
//! light. The runs are found by repeatedly taking the lowest set bit, then
//! inverting and masking, so each pair of `ffs` calls yields one run's two
//! ends. And the pattern is `random() & random()`, so a bit is set a quarter
//! of the time rather than half, which upstream's comment says looks right and
//! a half looks too busy.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, random, random_below,
};

const NBLADES: usize = 12;
const NSPINNERS: usize = 4;
const NRADII: usize = 8;
/// The colour index is fixed point with this many fractional bits, so a
/// spinner can creep along its colourmap slower than one colour a frame.
const COLORIDX_SHF: i32 = 4;
/// How many segments each blade's curved edge is drawn in, so that a ring of
/// them looks like a circle rather than a polygon.
const SUBDIV: usize = 6;
/// How many of the ten blink sweeps there are.
const MAX_BTYPE: u32 = 10;

const BTYPE_RADIAL_SINGLE: u32 = 0;
const BTYPE_RADIAL_RANDOM: u32 = 1;
const BTYPE_RADIAL_SEQ: u32 = 2;
const BTYPE_RADIAL_DOUBLESEQ: u32 = 3;
const BTYPE_SEGMENT_SINGLE: u32 = 4;
const BTYPE_SEGMENT_RANDOM: u32 = 5;
const BTYPE_CONCENTRIC_SINGLE: u32 = 6;
const BTYPE_CONCENTRIC_RANDOM: u32 = 7;
const BTYPE_CONCENTRIC_SEQ: u32 = 8;
const BTYPE_SEGMENT_SCATTER: u32 = 9;

/// Which of the blink sweeps is running. Upstream keeps a function pointer;
/// several of the ten share one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlinkDraw {
    RadialRandom,
    RadialSeq,
    RadialDoubleSeq,
    ConcentricRandom,
    ConcentricSeq,
    SegmentScatter,
}

/// One blade's two radii.
#[derive(Clone, Copy, Default)]
struct BladeState {
    outer: usize,
    inner: usize,
}

struct SpinnerState {
    /// Terminal rotation after count expires, and the per-frame increment.
    rot: f32,
    rotinc: f32,
    colors: Vec<XColor>,
    bladeidx: Vec<BladeState>,
    /// All three in n.4 fixed point.
    ncolors: i32,
    ccolor: i32,
    colorinc: i32,
    rotcount: i32,
    nblades: usize,
}

struct BlinkState {
    drawfunc: Option<BlinkDraw>,
    /// One random bit pattern per blade, for the scatter sweep.
    noise: Vec<u32>,
    color: [f32; 4],
    val: u32,
    /// Negative is a sharp blink, positive fades out over its dwell.
    dwell: i32,
    dwellcnt: i32,
    btype: u32,
    counter: i32,
    direction: i32,
    radius: i32,
}

struct Lockward {
    spinners: Vec<SpinnerState>,
    blink: BlinkState,
    /// The two arcs a blade is bounded by, one array per radius.
    points_outer: [[[f32; 3]; SUBDIV + 1]; NRADII],
    points_inner: [[[f32; 3]; SUBDIV + 1]; NRADII],
    rings: u32,
    blendmode: bool,
    nextblink: i32,
    fps: i32,

    blink_p: bool,
    blades: usize,
    rotateidle: (i32, i32),
    blinkidle: (i32, i32),
    blinkdwell: (i32, i32),
}

/// `ffs`: the one-based index of the lowest set bit, or zero.
fn ffs(x: u32) -> i32 {
    if x == 0 {
        0
    } else {
        x.trailing_zeros() as i32 + 1
    }
}

impl Lockward {
    /// Pick a new target rotation and how long to take getting there.
    ///
    /// The circle is divided up into `blades` divisions and the target is
    /// always one of them. The time is at most six seconds per division and at
    /// least one second however far away it is. During rendering the target is
    /// approached by *subtracting* the per-frame step times the outstanding
    /// ticks, so it lands exactly on the division rather than accumulating
    /// low-order error.
    fn random_blade_rot(&self, ss: &mut SpinnerState) {
        let mut dist = random_below(self.blades as i32) + 1;

        ss.rotcount = random_below(6 * dist * self.fps - self.fps) + self.fps;

        if random() & 4 != 0 {
            dist = -dist;
        }
        let d = dist as f32 * 360.0 / self.blades as f32;
        ss.rot += d;
        ss.rotinc = d / ss.rotcount as f32;
    }

    /// The two arcs every blade is drawn between, one per radius. A blade is a
    /// triangle fan from the outer arc round to the inner one, centred at
    /// three o'clock.
    fn gen_blade_arcs(&mut self) {
        let there = std::f64::consts::PI * 2.0 / self.blades as f64;
        let step = there / SUBDIV as f64;
        let here = -(SUBDIV as f64) * step / 2.0;

        for n in 0..NRADII {
            let r = n as f64 + 1.0;
            for (j, i) in (0..=SUBDIV).rev().enumerate() {
                let th = here + step * i as f64;
                self.points_outer[n][j] = [(th.cos() * r) as f32, (th.sin() * r) as f32, 0.0];
            }
            for i in 0..=SUBDIV {
                let th = here + step * i as f64;
                self.points_inner[n][i] = [(th.cos() * r) as f32, (th.sin() * r) as f32, 0.0];
            }
        }
    }

    fn gen_rings(&mut self, g: &mut Gl) {
        let step = std::f64::consts::PI * 2.0 / (self.blades * SUBDIV) as f64;

        for n in 0..NRADII - 1 {
            g.glx.new_list(self.rings + n as u32);
            g.glx.begin(Shape::TriangleStrip);
            for i in (0..=self.blades * SUBDIV).rev() {
                let th = step * i as f64;
                let (c, s) = (th.cos(), th.sin());
                g.glx.vertex3f(
                    (c * (n as f64 + 1.0)) as f32,
                    (s * (n as f64 + 1.0)) as f32,
                    0.0,
                );
                g.glx.vertex3f(
                    (c * (n as f64 + 2.0)) as f32,
                    (s * (n as f64 + 2.0)) as f32,
                    0.0,
                );
            }
            g.glx.end();
            g.glx.end_list();
        }
    }

    /// Compute a random interval between min and max milliseconds, in frames.
    fn calc_interval_frames(&self, min: i32, max: i32) -> i32 {
        let mut i = min;
        if max > min {
            i += random_below(max - min);
        }
        i * self.fps / 1000
    }

    fn draw_blink_blade(&self, g: &mut Gl, inner: usize, outer: usize, begin_p: bool) {
        if begin_p {
            g.glx.begin(Shape::TriangleFan);
        }
        for p in &self.points_outer[outer.min(NRADII - 1)] {
            g.glx.vertex3f(p[0], p[1], p[2]);
        }
        for p in &self.points_inner[inner.min(NRADII - 1)] {
            g.glx.vertex3f(p[0], p[1], p[2]);
        }
        if begin_p {
            g.glx.end();
        }
    }

    fn set_alpha_by_dwell(&mut self) {
        let bs = &mut self.blink;
        bs.color[3] = if bs.dwell > 0 {
            bs.dwellcnt as f32 / bs.dwell as f32
        } else if bs.dwellcnt > (-bs.dwell >> 2) {
            1.0
        } else {
            0.0
        };
    }

    /// Set the blend and colour every sweep draws with.
    fn blink_state(&self, g: &mut Gl) {
        g.glx.blend(Blend::DstColorAlpha);
        let c = self.blink.color;
        g.glx.color4f(c[0], c[1], c[2], c[3]);
    }

    /// One blade at a time in a random order, which is also how the single and
    /// the segment sweeps are drawn: there is no sense of direction in a
    /// random sweep, so `direction` is reused to hold the current blade.
    fn draw_blink_radial_random(&mut self, g: &mut Gl) {
        if self.blink.dwellcnt < 0 {
            if self.blink.counter <= 0 {
                self.blink.drawfunc = None;
                return;
            }

            /* Find available blade.  Potentially very slow, depending on how
            unlucky we are. */
            let mut i;
            loop {
                i = random_below(self.blades as i32);
                if self.blink.val & (1 << i) == 0 {
                    break;
                }
            }
            self.blink.val |= 1 << i; /*  Mark as used.  */
            self.blink.direction = i;
            self.blink.dwellcnt = self.blink.dwell.abs();

            if self.blink.btype == BTYPE_SEGMENT_SINGLE || self.blink.btype == BTYPE_SEGMENT_RANDOM
            {
                self.blink.radius = random_below(NRADII as i32 - 1);
            }

            self.blink.counter -= 1;
        }

        self.set_alpha_by_dwell();
        self.blink_state(g);
        g.glx.rotate(
            self.blink.direction as f32 * 360.0 / self.blades as f32,
            0.0,
            0.0,
            1.0,
        );
        if self.blink.radius >= 0 {
            let r = self.blink.radius as usize;
            self.draw_blink_blade(g, r, r + 1, true);
        } else {
            self.draw_blink_blade(g, 0, NRADII - 1, true);
        }

        self.blink.dwellcnt -= 1;
    }

    fn draw_blink_radial_sequential(&mut self, g: &mut Gl) {
        if self.blink.dwellcnt < 0 {
            if self.blink.counter <= 0 {
                self.blink.drawfunc = None;
                return;
            }
            self.blink.dwellcnt = self.blink.dwell.abs();
            self.blink.counter -= 1;
        }

        self.set_alpha_by_dwell();
        self.blink_state(g);
        let n = (self.blink.counter * self.blink.direction + self.blink.val as i32) as f32;
        g.glx.rotate(n * 360.0 / self.blades as f32, 0.0, 0.0, 1.0);
        self.draw_blink_blade(g, 0, NRADII - 1, true);

        self.blink.dwellcnt -= 1;
    }

    /// Two sequences at once, running in opposite directions from the same
    /// starting blade, which meet on the far side.
    fn draw_blink_radial_doubleseq(&mut self, g: &mut Gl) {
        if self.blink.dwellcnt < 0 {
            if self.blink.counter <= 0 {
                self.blink.drawfunc = None;
                return;
            }
            self.blink.dwellcnt = self.blink.dwell.abs();
            self.blink.counter -= 1;
        }

        self.set_alpha_by_dwell();
        self.blink_state(g);

        g.glx.push_matrix();
        let n = (self.blink.val as i32 + self.blink.counter) as f32;
        g.glx.rotate(n * 360.0 / self.blades as f32, 0.0, 0.0, 1.0);
        self.draw_blink_blade(g, 0, NRADII - 1, true);
        g.glx.pop_matrix();

        if self.blink.counter != 0 && self.blink.counter < self.blades as i32 / 2 {
            let n = (self.blink.val as i32 - self.blink.counter) as f32;
            g.glx.rotate(n * 360.0 / self.blades as f32, 0.0, 0.0, 1.0);
            self.draw_blink_blade(g, 0, NRADII - 1, true);
        }

        self.blink.dwellcnt -= 1;
    }

    fn draw_blink_concentric_random(&mut self, g: &mut Gl) {
        if self.blink.dwellcnt < 0 {
            if self.blink.counter <= 0 {
                self.blink.drawfunc = None;
                return;
            }

            let mut i;
            loop {
                i = random_below(NRADII as i32 - 1);
                if self.blink.val & (1 << i) == 0 {
                    break;
                }
            }
            self.blink.val |= 1 << i;
            self.blink.direction = i;
            self.blink.dwellcnt = self.blink.dwell.abs();

            self.blink.counter -= 1;
        }

        self.set_alpha_by_dwell();
        self.blink_state(g);
        g.glx.call_list(self.rings + self.blink.direction as u32);

        self.blink.dwellcnt -= 1;
    }

    fn draw_blink_concentric_sequential(&mut self, g: &mut Gl) {
        if self.blink.dwellcnt < 0 {
            if self.blink.counter <= 0 {
                self.blink.drawfunc = None;
                return;
            }
            self.blink.dwellcnt = self.blink.dwell.abs();
            self.blink.counter -= 1;
        }

        self.set_alpha_by_dwell();
        self.blink_state(g);
        let n = if self.blink.direction > 0 {
            (NRADII as i32 - 2) - self.blink.counter
        } else {
            self.blink.counter
        };
        g.glx
            .call_list(self.rings + n.clamp(0, NRADII as i32 - 2) as u32);

        self.blink.dwellcnt -= 1;
    }

    fn draw_blink_segment_scatter(&mut self, g: &mut Gl) {
        if self.blink.dwellcnt < 0 {
            if self.blink.counter <= 0 {
                self.blink.drawfunc = None;
                return;
            }

            /* Init random noise array.  On average, 1/4 of the bits will be
            set, which should look nice.  (1/2 looks too busy.) */
            for i in (0..self.blades).rev() {
                self.blink.noise[i] = random() & random() & ((1 << (NRADII - 1)) - 1);
            }

            self.blink.dwellcnt = self.blink.dwell.abs();
            self.blink.counter -= 1;
        }

        self.set_alpha_by_dwell();
        self.blink_state(g);

        for i in (0..self.blades).rev() {
            /* Find consecutive runs of 1 bits.  Keep going until we run out
            of them. */
            let mut bits = self.blink.noise[i];
            while bits != 0 {
                let inner = ffs(bits) - 1;
                bits = !bits & !((1u32 << inner) - 1);
                let outer = ffs(bits) - 1;
                bits = !bits & !((1u32 << outer) - 1);

                g.glx.push_matrix();
                g.glx
                    .rotate(i as f32 * 360.0 / self.blades as f32, 0.0, 0.0, 1.0);
                self.draw_blink_blade(g, inner as usize, outer as usize, true);
                g.glx.pop_matrix();
            }
        }

        self.blink.dwellcnt -= 1;
    }

    fn random_blink(&mut self) {
        let dwell = self.calc_interval_frames(self.blinkdwell.0, self.blinkdwell.1);
        let blades = self.blades as i32;
        let bs = &mut self.blink;
        bs.color = [1.0, 1.0, 1.0, 1.0];
        bs.dwellcnt = -1;
        bs.radius = -1;
        bs.dwell = dwell;
        if random() & 2 != 0 {
            bs.dwell = -bs.dwell;
        }

        bs.btype = random() % MAX_BTYPE;

        match bs.btype {
            BTYPE_RADIAL_SINGLE | BTYPE_SEGMENT_SINGLE => {
                bs.drawfunc = Some(BlinkDraw::RadialRandom);
                bs.val = 0;
                bs.counter = 1;
            }
            BTYPE_RADIAL_RANDOM | BTYPE_SEGMENT_RANDOM => {
                bs.drawfunc = Some(BlinkDraw::RadialRandom);
                bs.val = 0;
                bs.counter = blades;
            }
            BTYPE_RADIAL_SEQ => {
                bs.drawfunc = Some(BlinkDraw::RadialSeq);
                bs.val = random() % blades as u32; /*  Initial offset  */
                bs.direction = if random() & 8 != 0 { 1 } else { -1 };
                bs.counter = blades;
            }
            BTYPE_RADIAL_DOUBLESEQ => {
                bs.drawfunc = Some(BlinkDraw::RadialDoubleSeq);
                bs.val = random() % blades as u32; /*  Initial offset  */
                bs.counter = blades / 2 + 1;
            }
            BTYPE_CONCENTRIC_SINGLE => {
                bs.drawfunc = Some(BlinkDraw::ConcentricRandom);
                bs.val = 0;
                bs.counter = 1;
            }
            BTYPE_CONCENTRIC_RANDOM => {
                bs.drawfunc = Some(BlinkDraw::ConcentricRandom);
                bs.val = 0;
                bs.counter = NRADII as i32 - 1;
            }
            BTYPE_CONCENTRIC_SEQ => {
                bs.drawfunc = Some(BlinkDraw::ConcentricSeq);
                bs.direction = if random() & 8 != 0 { 1 } else { -1 };
                bs.counter = NRADII as i32 - 1;
            }
            _ => {
                // The scatter, and there is no other value: the type is a
                // remainder by the number of types.
                debug_assert_eq!(bs.btype, BTYPE_SEGMENT_SCATTER);
                bs.drawfunc = Some(BlinkDraw::SegmentScatter);
                bs.counter = random_below(blades / 2) + (blades / 2) + 1;
            }
        }
    }
}

impl Hack3d for Lockward {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.cull_face(true);
        g.glx.depth_test(false);
        g.glx.front_face_cw(true);

        let spin_blend = if self.blendmode {
            Blend::Add
        } else {
            Blend::Alpha
        };

        g.glx.push_matrix();
        g.glx.load_identity();

        for n in (0..self.spinners.len()).rev() {
            /*  Set color.  */
            let (color, rot, rotcount, rotinc, nblades) = {
                let ss = &self.spinners[n];
                let i = (ss.ccolor >> COLORIDX_SHF).clamp(0, ss.colors.len() as i32 - 1) as usize;
                let c = &ss.colors[i];
                (
                    [
                        f32::from(c.red) / 65535.0,
                        f32::from(c.green) / 65535.0,
                        f32::from(c.blue) / 65535.0,
                        0.5,
                    ],
                    ss.rot,
                    ss.rotcount,
                    ss.rotinc,
                    ss.nblades,
                )
            };
            g.glx.blend(spin_blend);
            g.glx.color4f(color[0], color[1], color[2], color[3]);

            g.glx.push_matrix();
            g.glx.rotate(rot - rotcount as f32 * rotinc, 0.0, 0.0, 1.0);
            for i in (0..nblades).rev() {
                let b = self.spinners[n].bladeidx[i];
                g.glx.push_matrix();
                g.glx
                    .rotate(360.0 * i as f32 / nblades as f32, 0.0, 0.0, 1.0);
                g.glx.begin(Shape::TriangleFan);
                self.draw_blink_blade(g, b.inner, b.outer, false);
                g.glx.end();
                g.glx.pop_matrix();
            }
            g.glx.pop_matrix();

            /*  Advance rotation.  */
            if self.spinners[n].rotcount != 0 {
                if self.spinners[n].rotcount > 0 {
                    self.spinners[n].rotcount -= 1;
                }
            } else if self.spinners[n].rotinc == 0.0 {
                let mut ss = std::mem::replace(&mut self.spinners[n], SpinnerState::blank());
                self.random_blade_rot(&mut ss);
                self.spinners[n] = ss;
            } else {
                /*  Compute # of ticks to sit idle.  */
                self.spinners[n].rotinc = 0.0;
                self.spinners[n].rotcount =
                    self.calc_interval_frames(self.rotateidle.0, self.rotateidle.1);
            }

            /*  Advance colors.  */
            let ss = &mut self.spinners[n];
            ss.ccolor += ss.colorinc;
            if ss.ccolor >= ss.ncolors {
                ss.ccolor -= ss.ncolors;
            } else if ss.ccolor < 0 {
                ss.ccolor += ss.ncolors;
            }
        }

        if self.blink_p {
            match self.blink.drawfunc {
                Some(BlinkDraw::RadialRandom) => self.draw_blink_radial_random(g),
                Some(BlinkDraw::RadialSeq) => self.draw_blink_radial_sequential(g),
                Some(BlinkDraw::RadialDoubleSeq) => self.draw_blink_radial_doubleseq(g),
                Some(BlinkDraw::ConcentricRandom) => self.draw_blink_concentric_random(g),
                Some(BlinkDraw::ConcentricSeq) => self.draw_blink_concentric_sequential(g),
                Some(BlinkDraw::SegmentScatter) => self.draw_blink_segment_scatter(g),
                None => {
                    if self.nextblink > 0 {
                        self.nextblink -= 1;
                    } else {
                        /* Compute # of frames for blink idle time. */
                        self.nextblink =
                            self.calc_interval_frames(self.blinkidle.0, self.blinkidle.1);
                        self.random_blink();
                    }
                }
            }
        }
        g.glx.pop_matrix();

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
        if height > width {
            g.glx.ortho(-8.0, 8.0, -8.0 * h, 8.0 * h, -1.0, 1.0);
        } else {
            g.glx.ortho(-8.0 / h, 8.0 / h, -8.0, 8.0, -1.0, 1.0);
        }
        g.glx.matrix_mode_modelview();
    }

    fn event(&mut self, _g: &mut Gl, event: &XEvent) -> bool {
        if let XEvent::KeyPress { key } = event
            && (*key == ' ' || *key == '\t')
        {
            self.blendmode = !self.blendmode;
            return true;
        }
        false
    }
}

impl SpinnerState {
    fn blank() -> SpinnerState {
        SpinnerState {
            rot: 0.0,
            rotinc: 0.0,
            colors: Vec::new(),
            bladeidx: Vec::new(),
            ncolors: 1,
            ccolor: 0,
            colorinc: 1,
            rotcount: -1,
            nblades: 0,
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let blades = NBLADES;
    let mut st = Lockward {
        spinners: Vec::new(),
        blink: BlinkState {
            drawfunc: None,
            noise: vec![0; blades],
            color: [1.0; 4],
            val: 0,
            dwell: 0,
            dwellcnt: 0,
            btype: 0,
            counter: 0,
            direction: 0,
            radius: -1,
        },
        points_outer: [[[0.0; 3]; SUBDIV + 1]; NRADII],
        points_inner: [[[0.0; 3]; SUBDIV + 1]; NRADII],
        rings: 0,
        blendmode: false,
        nextblink: 0,
        // Upstream computes this from the delay, decides it does not like the
        // answer, and hardcodes sixty. Every interval below is in frames.
        fps: 60,
        blink_p: g.res.bool("blink"),
        blades,
        rotateidle: (
            g.res.int("rotateidle-min").max(0),
            g.res.int("rotateidle-max").max(0),
        ),
        blinkidle: (
            g.res.int("blinkidle-min").max(0),
            g.res.int("blinkidle-max").max(0),
        ),
        blinkdwell: (
            g.res.int("blinkdwell-min").max(0),
            g.res.int("blinkdwell-max").max(0),
        ),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    st.rings = g.glx.gen_lists(NRADII - 1);
    st.nextblink = st.calc_interval_frames(st.blinkidle.0, st.blinkidle.1);

    st.gen_blade_arcs();
    st.gen_rings(g);

    for i in (0..NSPINNERS).rev() {
        let mut ss = SpinnerState::blank();

        /*  Establish rotation  */
        st.random_blade_rot(&mut ss);

        /* Establish color cycling path and rate.  Rate avoids zero. */
        ss.colors = make_smooth_colormap(128);
        ss.ncolors = 128;
        ss.colorinc = (random() & ((2 << COLORIDX_SHF) - 1)) as i32 - (1 << COLORIDX_SHF);
        if ss.colorinc >= 0 {
            ss.colorinc += 1;
        }
        ss.ncolors <<= COLORIDX_SHF;

        /* Create blades. */
        ss.nblades = blades;
        ss.bladeidx = (0..blades).map(|_| BladeState::default()).collect();
        for n in (0..blades).rev() {
            /* Establish blade radii.  Can't be equal.  Ensure outer > inner. */
            let (mut outer, mut inner);
            loop {
                outer = (random() & 7) as usize;
                inner = (random() & 7) as usize;
                if outer != inner {
                    break;
                }
            }
            if outer < inner {
                std::mem::swap(&mut outer, &mut inner);
            }
            ss.bladeidx[n] = BladeState { outer, inner };
        }
        // Built back to front, as upstream does, and pushed in that order so
        // the first spinner drawn is the last one built.
        let _ = i;
        st.spinners.push(ss);
    }
    st.spinners.reverse();

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*blink:        True",
    "*rotateidle-min: 1000",
    "*rotateidle-max: 6000",
    "*blinkidle-min:  1000",
    "*blinkidle-max:  9000",
    "*blinkdwell-min: 100",
    "*blinkdwell-max: 600",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider(
        "rotateidle-min",
        "Minimum rotator idle time",
        500.0,
        10000.0,
        100.0,
        0,
        "1000",
    ),
    Opt::slider(
        "rotateidle-max",
        "Maximum rotator idle time",
        500.0,
        10000.0,
        100.0,
        0,
        "6000",
    ),
    Opt::slider(
        "blinkidle-min",
        "Minimum blink idle time",
        500.0,
        20000.0,
        100.0,
        0,
        "1000",
    ),
    Opt::slider(
        "blinkidle-max",
        "Maximum blink idle time",
        500.0,
        20000.0,
        100.0,
        0,
        "9000",
    ),
    Opt::slider(
        "blinkdwell-min",
        "Minimum blink dwell time",
        50.0,
        1500.0,
        10.0,
        0,
        "100",
    ),
    Opt::slider(
        "blinkdwell-max",
        "Maximum blink dwell time",
        50.0,
        1500.0,
        10.0,
        0,
        "600",
    ),
    Opt::boolean("blink", "Blinking effects", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "lockward",
    label: "Lockward",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Leo L. Schwab",
        year: "2007",
        video: Some("https://www.youtube.com/watch?v=MGwySGVQZ2M"),
        blurb: "A translucent spinning, blinking thing.",
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

    /// Runs of set bits are found by taking the lowest one and then inverting
    /// and masking, twice per run. It is the whole of the scatter sweep, and
    /// getting it wrong is either no segments or an endless loop.
    #[test]
    fn runs_of_bits_come_out_in_pairs() {
        let runs = |noise: u32| {
            let mut out = Vec::new();
            let mut bits = noise;
            while bits != 0 {
                let inner = ffs(bits) - 1;
                bits = !bits & !((1u32 << inner) - 1);
                let outer = ffs(bits) - 1;
                bits = !bits & !((1u32 << outer) - 1);
                out.push((inner, outer));
            }
            out
        };
        // One run of one bit, from 1 to 2.
        assert_eq!(runs(0b10), vec![(1, 2)]);
        // Two separate runs.
        assert_eq!(runs(0b0001010), vec![(1, 2), (3, 4)]);
        // One run of three bits is one segment three deep.
        assert_eq!(runs(0b0011100), vec![(2, 5)]);
        assert_eq!(runs(0), vec![]);
        // And it always terminates, whatever the seven bits are.
        for noise in 0..128u32 {
            let r = runs(noise);
            assert!(r.iter().all(|(a, b)| b > a && *b < 8));
        }
    }

    /// A blade is a fan from one arc round to another, so its vertices all sit
    /// on two circles of the radii it was given.
    #[test]
    fn a_blade_lies_between_its_two_radii() {
        let mut r = start(StartArgs::new(640, 480, "blink=false", 20260811));
        r.step();
        let f = r.frame();
        for b in &f.batches {
            let radii: Vec<f32> = f.vertices[b.first..b.first + b.count]
                .iter()
                .map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1]).sqrt())
                .collect();
            // Every radius is a whole number from one to eight.
            for r in radii {
                assert!(
                    (0.99..=8.01).contains(&r) && (r - r.round()).abs() < 1e-4,
                    "a vertex {r} from the middle"
                );
            }
        }
    }

    /// A spinner lands on an exact blade division rather than drifting: after
    /// its count runs out, its rotation is a whole number of blade widths.
    #[test]
    fn a_spinner_lands_on_a_division() {
        let mut r = start(StartArgs::new(640, 480, "blink=false", 20260811));
        let step = 360.0 / NBLADES as f32;
        let mut idle = 0;
        let mut prev: Option<f32> = None;
        for _ in 0..6000 {
            r.step();
            let f = r.frame();
            // The first batch is the last spinner's first blade, and its
            // matrix is that spinner's rotation. Recover the angle from it: a
            // rotation about z puts the cosine and sine in the first column.
            let Some(b) = f.batches.first() else { continue };
            let angle = b.modelview.0[1].atan2(b.modelview.0[0]).to_degrees();
            // A frame where the angle did not move at all is one where the
            // spinner is sitting between turns, and that is where it has to be
            // on an exact division.
            if prev == Some(angle) {
                idle += 1;
                let k = angle / step;
                assert!(
                    (k - k.round()).abs() < 1e-2,
                    "sat still at {angle}, which is not a whole blade width"
                );
            }
            prev = Some(angle);
        }
        assert!(idle > 20, "it never sat still: {idle} frames");
    }

    /// The blinks happen, and they are drawn multiplied rather than added, or
    /// they would light up the black background as well.
    #[test]
    fn the_blinks_multiply_what_is_there() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "blinkidle-min=500&blinkidle-max=600",
            20260811,
        ));
        let mut blinked = 0;
        for _ in 0..600 {
            r.step();
            if r.frame()
                .batches
                .iter()
                .any(|b| b.blend == Blend::DstColorAlpha)
            {
                blinked += 1;
            }
        }
        assert!(blinked > 20, "it only blinked on {blinked} frames");
    }
}
