//! Port of `hacks/glx/noof.c`.
//!
//! ```text
//! noof, Copyright (c) 2004-2018 Bill Torzewski <billt@worksitez.com>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Originally a demo included with GLUT;
//! (Apparently this was called "diatoms" on Irix.)
//! ported to raw GL and xscreensaver by jwz, 12-Feb-2004.
//! ```
//!
//! Flowery, rotatey patterns.
//!
//! Seven flowers drift about a flat square, each a ring of two to eighteen
//! identical petals, and each petal one filled quad with a bright outline over
//! it. A flower's shape breathes: the two numbers that make the petal are sums
//! of sines at incommensurate rates, so it opens and closes without ever
//! repeating, and the whole thing pulses in and out of the screen on a third.
//!
//! A flower that closes right down to nothing is meant to be quietly replaced
//! with a new one, but is not: the gate on that is the low bit of a counter
//! which only the replacement itself advances, so it shuts after the first one
//! and never opens again. Since every flower starts at phase zero, which is
//! closed, the one replacement happens on the first frame and nothing is ever
//! replaced after that. This is upstream's arithmetic and the port keeps it;
//! the visible difference is none, because a flower re-rolled on frame one
//! looks exactly like a flower rolled at start-up.
//!
//! What makes the picture rather than the motion is that nothing is ever
//! erased. Each frame draws the last one back before drawing on it, so the
//! petals leave every position they have been in and the screen fills with a
//! dense weave of outlines. It does not silt up into a solid block, because
//! the fill of every petal is black at a bit over a third alpha, so a flower
//! sweeping its own disc darkens what is under it as it goes.
//!
//! Upstream keeps the old frame in a texture and copies the screen into it at
//! the end of every frame, because a colour buffer is no longer guaranteed to
//! survive to the next one; this does the same, through
//! [`crate::runtime::gl::Glx::copy_tex_sub_image_2d`].
//!
//! The flowers also pull on each other, weakly and repulsively, whenever two
//! come within a third of the screen.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, frand};

const N_SHAPES: usize = 7;

/// How far out a petal may open before it overlaps its neighbour, indexed by
/// how many petals the flower has. Upstream's table, which starts at two.
const BLADERATIO: [f32; 20] = [
    /* nblades = 2..7 */
    0.0, 0.0, 3.00000, 1.73205, 1.00000, 0.72654, 0.57735, 0.48157, /* 8..13 */
    0.41421, 0.36397, 0.19076, 0.29363, 0.26795, 0.24648, /* 14..19 */
    0.22824, 0.21256, 0.19891, 0.18693, 0.17633, 0.16687,
];

struct Noof {
    /// Three numbers per flower throughout, though only the first two are ever
    /// used: the drawing is flat.
    pos: [f32; N_SHAPES * 3],
    dir: [f32; N_SHAPES * 3],
    col: [f32; N_SHAPES * 3],
    hsv: [f32; N_SHAPES * 3],
    /// How fast each flower's hue, saturation and value drift.
    hpr: [f32; N_SHAPES * 3],
    ang: [f32; N_SHAPES],
    spn: [f32; N_SHAPES],
    sca: [f32; N_SHAPES],
    /// The phase the petal shape is drawn at, and how fast it advances.
    geep: [f32; N_SHAPES],
    peep: [f32; N_SHAPES],
    blad: [usize; N_SHAPES],

    ht: f32,
    wd: f32,

    /// Counts rebirths, and its lowest bit gates them. Since nothing else
    /// touches it, one rebirth is all there is: see the note at the top.
    tko: u32,

    /// The last frame, kept so this one can be drawn on top of it.
    screenshot: u32,
    tex_w: i32,
    tex_h: i32,
}

/// `to_pow2` from `pow2.h`: the smallest power of two that fits this.
fn to_pow2(n: i32) -> i32 {
    (n.max(1) as u32).next_power_of_two() as i32
}

impl Noof {
    fn initshapes(&mut self, i: usize) {
        // Random init of pos, dir, color.
        for k in i * 3..=i * 3 + 2 {
            self.pos[k] = frand(1.0) as f32;
            self.dir[k] = (frand(1.0) as f32 - 0.5) * 0.05;
            // Upstream also seeds an acceleration here. Every line that used
            // it is commented out in the C, so it is left out rather than
            // carried along doing nothing.
            let _acc = (frand(1.0) as f32 - 0.5) * 0.0002;
            self.col[k] = frand(1.0) as f32;
        }

        self.blad[i] = 2 + (frand(1.0) * 17.0) as usize;
        self.ang[i] = frand(1.0) as f32;
        self.spn[i] = (frand(1.0) as f32 - 0.5) * 40.0 / (10 + self.blad[i]) as f32;
        self.sca[i] = frand(1.0) as f32 * 0.1 + 0.08;
        self.dir[i * 3] *= self.sca[i];
        self.dir[i * 3 + 1] *= self.sca[i];

        self.hsv[i * 3] = frand(1.0) as f32 * 360.0;
        self.hsv[i * 3 + 1] = frand(1.0) as f32 * 0.6 + 0.4;
        self.hsv[i * 3 + 2] = frand(1.0) as f32 * 0.7 + 0.3;

        self.hpr[i * 3] = frand(1.0) as f32 * 0.005 * 360.0;
        self.hpr[i * 3 + 1] = frand(1.0) as f32 * 0.03;
        self.hpr[i * 3 + 2] = frand(1.0) as f32 * 0.02;

        self.geep[i] = 0.0;
        self.peep[i] = 0.01 + frand(1.0) as f32 * 0.2;
    }

    /// One flower: its petals, each a filled quad with its outline over it.
    fn drawleaf(&mut self, g: &mut Gl, l: usize) {
        let blades = self.blad[l];
        let rad = |d: f32| d * std::f32::consts::PI / 180.0;

        let mut y = 0.10 * rad(self.geep[l]).sin() + 0.099 * rad(self.geep[l] * 5.12).sin();
        if y < 0.0 {
            y = -y;
        }
        let mut x = 0.15 * rad(self.geep[l]).cos() + 0.149 * rad(self.geep[l] * 5.12).cos();
        if x < 0.0 {
            x = -x;
        }

        // Closed right down: let it become reborn as something else. Once,
        // because tko only turns over here.
        if y < 0.001 && x > 0.000002 && (self.tko & 0x1) == 0 {
            self.initshapes(l);
            self.tko += 1;
            return;
        }

        let wobble = {
            let w1 = rad(self.geep[l] * 15.3).sin();
            3.0 + 2.00 * rad(self.geep[l] * 0.4).sin() + 3.94261 * w1
        };

        if y > x * BLADERATIO[blades] {
            y = x * BLADERATIO[blades];
        }

        for b in 0..blades {
            g.glx.push_matrix();
            g.glx
                .translate(self.pos[l * 3], self.pos[l * 3 + 1], self.pos[l * 3 + 2]);
            g.glx.rotate(
                self.ang[l] + b as f32 * (360.0 / blades as f32),
                0.0,
                0.0,
                1.0,
            );
            let s = wobble * self.sca[l];
            g.glx.scale(s, s, s);

            // The fill is black at a bit over a third alpha, so a petal darkens
            // whatever it crosses rather than hiding it. That is what stops the
            // accumulated frames from silting up into a solid block.
            g.glx.color4f(0.0, 0.0, 0.0, 0x60 as f32 / 255.0);
            g.glx.blend(Blend::Alpha);

            g.glx.begin(Shape::TriangleStrip);
            g.glx.vertex3f(x * self.sca[l], 0.0, 0.0);
            g.glx.vertex3f(x, y, 0.0);
            g.glx.vertex3f(x, -y, 0.0);
            g.glx.vertex3f(0.3, 0.0, 0.0);
            g.glx.end();

            g.glx.color4f(
                self.col[l * 3],
                self.col[l * 3 + 1],
                self.col[l * 3 + 2],
                1.0,
            );
            g.glx.begin(Shape::LineLoop);
            g.glx.vertex3f(x * self.sca[l], 0.0, 0.0);
            g.glx.vertex3f(x, y, 0.0);
            g.glx.vertex3f(0.3, 0.0, 0.0);
            g.glx.vertex3f(x, -y, 0.0);
            g.glx.end();
            g.glx.blend(Blend::Off);

            g.glx.pop_matrix();
        }
    }

    /// Drift, and bounce off the edges. The test is on the direction as well as
    /// the position, so a flower that starts outside walks back in rather than
    /// rattling against the wall.
    fn motion_update(&mut self, t: usize) {
        // Upstream writes this as a chain of `else if`, and the chain is load
        // bearing: only the first wall a flower is past turns it, so one that
        // leaves through a corner takes two frames to come back.
        let (x, y) = (t * 3, t * 3 + 1);
        let walls = [
            (x, self.pos[x] < -self.sca[t] * self.wd && self.dir[x] < 0.0),
            (
                x,
                self.pos[x] > (1.0 + self.sca[t]) * self.wd && self.dir[x] > 0.0,
            ),
            (y, self.pos[y] < -self.sca[t] * self.ht && self.dir[y] < 0.0),
            (
                y,
                self.pos[y] > (1.0 + self.sca[t]) * self.ht && self.dir[y] > 0.0,
            ),
        ];
        if let Some((k, _)) = walls.into_iter().find(|(_, past)| *past) {
            self.dir[k] = -self.dir[k];
        }

        self.pos[t * 3] += self.dir[t * 3];
        self.pos[t * 3 + 1] += self.dir[t * 3 + 1];

        self.ang[t] += self.spn[t];
        self.geep[t] += self.peep[t];
        if self.geep[t] > 360.0 * 5.0 {
            self.geep[t] -= 360.0 * 5.0;
        }
        if self.ang[t] < 0.0 {
            self.ang[t] += 360.0;
        }
        if self.ang[t] > 360.0 {
            self.ang[t] -= 360.0;
        }
    }

    /// Walk the colour around HSV space, turning each of the three round when
    /// it reaches an end, and convert what comes out to RGB. Written out rather
    /// than handed to the shared helper because it clamps and reflects the HSV
    /// in place, and those are the numbers that carry to the next frame.
    fn color_update(&mut self, i: usize) {
        let (h, s, v) = (i * 3, i * 3 + 1, i * 3 + 2);

        if self.hsv[s] <= 0.5 && self.hpr[s] < 0.0 {
            self.hpr[s] = -self.hpr[s];
        }
        if self.hsv[s] >= 1.0 && self.hpr[s] > 0.0 {
            self.hpr[s] = -self.hpr[s];
        }
        if self.hsv[v] <= 0.4 && self.hpr[v] < 0.0 {
            self.hpr[v] = -self.hpr[v];
        }
        if self.hsv[v] >= 1.0 && self.hpr[v] > 0.0 {
            self.hpr[v] = -self.hpr[v];
        }

        self.hsv[h] += self.hpr[h];
        self.hsv[s] += self.hpr[s];
        self.hsv[v] += self.hpr[v];

        self.hsv[v] = self.hsv[v].clamp(0.0, 1.0);

        let (r, g, b) = (i * 3, i * 3 + 1, i * 3 + 2);
        if self.hsv[s] <= 0.0 {
            self.col[r] = self.hsv[v];
            self.col[g] = self.hsv[v];
            self.col[b] = self.hsv[v];
            return;
        }

        while self.hsv[h] < 0.0 {
            self.hsv[h] += 360.0;
        }
        while self.hsv[h] >= 360.0 {
            self.hsv[h] -= 360.0;
        }
        self.hsv[s] = self.hsv[s].clamp(0.0, 1.0);

        let hh = self.hsv[h] / 60.0;
        let hi = hh as i32;
        let f = hh - hi as f32;
        let v0 = self.hsv[v];
        let p = self.hsv[v] * (1.0 - self.hsv[s]);
        let q = self.hsv[v] * (1.0 - self.hsv[s] * f);
        let t = self.hsv[v] * (1.0 - self.hsv[s] * (1.0 - f));

        let rgb = match hi {
            i32::MIN..=0 => [v0, t, p],
            1 => [q, v0, p],
            2 => [p, v0, t],
            3 => [p, q, v0],
            4 => [t, p, v0],
            _ => [v0, p, q],
        };
        self.col[r] = rgb[0];
        self.col[g] = rgb[1];
        self.col[b] = rgb[2];
    }

    /// Every pair of flowers within a third of the screen of each other pushes
    /// apart, harder the closer they are, and a heavy flower pushes harder than
    /// a light one. `fx` is negative, which is what turns gravity into this.
    fn gravity(&mut self, fx: f32) {
        for a in 0..N_SHAPES {
            for b in 0..a {
                let mut t = self.pos[b * 3] - self.pos[a * 3];
                let mut d2 = t * t;
                t = self.pos[b * 3 + 1] - self.pos[a * 3 + 1];
                d2 += t * t;
                if d2 < 0.000_001 {
                    d2 = 0.000_01;
                }
                if d2 >= 0.1 {
                    continue;
                }

                let v0 = self.pos[b * 3] - self.pos[a * 3];
                let v1 = self.pos[b * 3 + 1] - self.pos[a * 3 + 1];
                let z = 0.000_000_01 * fx / d2;

                self.dir[a * 3] += v0 * z * self.sca[b];
                self.dir[b * 3] += -v0 * z * self.sca[a];
                self.dir[a * 3 + 1] += v1 * z * self.sca[b];
                self.dir[b * 3 + 1] += -v1 * z * self.sca[a];
            }
        }
    }
}

impl Hack3d for Noof {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(false);
        g.glx.clear();

        // Frame N+1 accumulates atop frame N, and the only thing that survives
        // from one frame to the next is a texture, so put last frame back
        // before drawing on it.
        let tw = g.width() as f32 / self.tex_w as f32;
        let th = g.height() as f32 / self.tex_h as f32;
        g.glx.blend(Blend::Off);
        g.glx.texturing(true);
        g.glx.bind_texture(self.screenshot);
        g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        g.glx.begin(Shape::Quads);
        g.glx.tex_coord2f(0.0, 0.0);
        g.glx.vertex3f(0.0, 0.0, 0.0);
        g.glx.tex_coord2f(tw, 0.0);
        g.glx.vertex3f(self.wd, 0.0, 0.0);
        g.glx.tex_coord2f(tw, th);
        g.glx.vertex3f(self.wd, self.ht, 0.0);
        g.glx.tex_coord2f(0.0, th);
        g.glx.vertex3f(0.0, self.ht, 0.0);
        g.glx.end();
        g.glx.texturing(false);
        g.glx.clear_depth();

        self.gravity(-2.0);
        for i in 0..N_SHAPES {
            self.motion_update(i);
            self.color_update(i);
            self.drawleaf(g, i);
        }

        // And keep what that came to for the next frame.
        g.glx.texturing(true);
        g.glx.bind_texture(self.screenshot);
        g.glx.copy_tex_sub_image_2d();
        g.glx.texturing(false);

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        if width <= height {
            self.wd = 1.0;
            self.ht = height as f32 / width.max(1) as f32;
            g.glx.ortho(0.0, 1.0, 0.0, self.ht, -16.0, 4.0);
        } else {
            self.wd = width as f32 / height.max(1) as f32;
            self.ht = 1.0;
            g.glx.ortho(0.0, self.wd, 0.0, 1.0, -16.0, 4.0);
        }
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        if self.screenshot == 0 {
            self.screenshot = g.glx.gen_texture();
        }
        // A fresh, black texture the size of the window rounded up: the old one
        // is the wrong shape now, and there is nothing worth keeping in it.
        self.tex_w = to_pow2(width);
        self.tex_h = to_pow2(height);
        g.glx.bind_texture(self.screenshot);
        g.glx.tex_image_2d(self.tex_w, self.tex_h, Vec::new());
        g.glx.clear();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut st = Noof {
        pos: [0.0; N_SHAPES * 3],
        dir: [0.0; N_SHAPES * 3],
        col: [0.0; N_SHAPES * 3],
        hsv: [0.0; N_SHAPES * 3],
        hpr: [0.0; N_SHAPES * 3],
        ang: [0.0; N_SHAPES],
        spn: [0.0; N_SHAPES],
        sca: [0.0; N_SHAPES],
        geep: [0.0; N_SHAPES],
        peep: [0.0; N_SHAPES],
        blad: [0; N_SHAPES],
        ht: 1.0,
        wd: 1.0,
        tko: 0,
        screenshot: 0,
        tex_w: 1,
        tex_h: 1,
    };

    for i in 0..N_SHAPES {
        st.initshapes(i);
    }

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        10000",
    "*showFPS:      False",
    "*fpsSolid:     True",
    "*doubleBuffer: False",
    "*suppressRotationAnimation: True",
];

const OPTS: &[Opt] =
    &[Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted()];

pub static DEF: SaverDef = SaverDef {
    slug: "noof",
    label: "Noof",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Bill Torzewski",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=x5DQjgYqmn0"),
        blurb: "Flowery, rotatey patterns.",
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

    fn run(frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// Every frame ends by copying the screen into the texture it started by
    /// drawing, which is the whole reason the picture accumulates rather than
    /// starting over.
    #[test]
    fn the_frame_is_kept_for_the_next_one() {
        let r = run(3);
        let f = r.frame();

        let copies: Vec<_> = f.batches.iter().filter_map(|b| b.copy_to_texture).collect();
        assert_eq!(copies.len(), 1, "one screenshot a frame");

        // Drawn back at the top, so the copy is the last thing in the frame and
        // the textured quad is the first.
        assert_eq!(
            f.batches.last().and_then(|b| b.copy_to_texture),
            Some(copies[0]),
            "the copy comes after everything it is meant to catch"
        );
        assert_eq!(
            f.batches[0].texture,
            Some(copies[0]),
            "and the same texture is drawn back first"
        );
        assert_eq!(f.batches[0].count, 6, "as one quad over the whole window");
    }

    /// A flower is a ring of petals, each a filled quad under a line loop, so
    /// the frame is triangles and line loops in equal numbers.
    #[test]
    fn every_flower_is_a_ring_of_petals() {
        let r = run(2);
        let f = r.frame();

        let loops = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::LineLoop)
            .count();
        // Two to eighteen petals a flower, seven flowers, and a flower that is
        // reborn this frame draws nothing.
        assert!(
            (2..=18 * N_SHAPES).contains(&loops),
            "{loops} outlines is not a screenful of petals"
        );

        // Each outline sits over its own fill, at the same place in the matrix.
        for b in f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::LineLoop)
        {
            assert_eq!(b.count, 4, "a petal outline is four corners");
        }
    }

    /// The petal fill is black and translucent and the outline is not, which is
    /// what keeps an accumulating screen from silting up.
    #[test]
    fn the_fill_darkens_and_the_outline_colours() {
        let r = run(2);
        let f = r.frame();

        let strips: Vec<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::TriangleStrip)
            .collect();
        assert!(!strips.is_empty(), "no petals were filled");
        for b in &strips {
            assert_eq!(b.blend, Blend::Alpha);
            let c = f.vertices[b.first].color;
            assert_eq!([c[0], c[1], c[2]], [0.0, 0.0, 0.0], "the fill is black");
            assert!((c[3] - 0x60 as f32 / 255.0).abs() < 1e-6, "and translucent");
        }

        let lit = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::LineLoop)
            .any(|b| {
                let c = f.vertices[b.first].color;
                c[0] + c[1] + c[2] > 0.1
            });
        assert!(lit, "every outline came out black");
    }

    /// One rebirth, on the first frame, and none after: the gate on it is a
    /// parity that only the rebirth advances.
    ///
    /// Seen from the outside, because a flower keeps its petal count for its
    /// whole life: the number of outlines on screen is constant except on a
    /// frame where a flower is reborn instead of drawn, and that frame is
    /// short by exactly that flower's petals.
    #[test]
    fn only_the_first_frame_rebirths_a_flower() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut counts = Vec::new();
        // Far longer than a phase takes to come round: geep advances by up to
        // 0.21 degrees a frame and closes twice per turn, so every flower has
        // shut many times over by the end of this.
        for _ in 0..4000 {
            r.step();
            counts.push(
                r.frame()
                    .batches
                    .iter()
                    .filter(|b| b.primitive == Primitive::LineLoop)
                    .count(),
            );
        }

        let steady = counts[counts.len() - 1];
        let odd: Vec<usize> = (0..counts.len()).filter(|&i| counts[i] != steady).collect();
        assert_eq!(odd, vec![0], "the rebirth gate opened more than once");
        assert!(
            counts[0] < steady && counts[0] > 0,
            "frame one should be short one flower, not {} of {steady}",
            counts[0]
        );
    }
}
