//! Port of `hacks/glx/hydrostat.c`.
//!
//! ```text
//! hydrostat, Copyright © 2012-2025 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in
//! all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
//! THE SOFTWARE.
//!
//! Tentacle simulation using inverse kinematics.
//!
//!   http://soulwire.co.uk/experiments/muscular-hydrostats/
//!   https://github.com/soulwire/Muscular-Hydrostats/
//!
//! Ported to C from Javascript by jwz, May 2016
//! ```
//!
//! Squid, drifting, with tentacles that trail behind them.
//!
//! A tentacle is a chain of nodes and no springs. Each node in turn is pulled
//! back to a fixed distance from the one above it, its velocity is read off as
//! the distance it actually moved, and that velocity is damped and pushed on by
//! gravity and a sideways current. Running the constraint down the chain once
//! per frame is the whole of the inverse kinematics, and the wave that travels
//! down a tentacle is a consequence of doing it in order rather than anything
//! anybody wrote.
//!
//! The head does not steer the body; the body reports where the head should
//! point. Every frame it averages the position of the node a third of the way
//! down each tentacle and tilts the head to face away from where the trail has
//! got to, so a squid that is being dragged looks like it is straining.
//!
//! Squid are drawn back to front with additive blending and the depth buffer
//! cleared between them, which is why they glow through each other rather than
//! occluding.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::shapes::unit_dome;
use crate::runtime::{
    About, Ease, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, ease, frand, random,
};

const TENTACLE_FACES: usize = 5;

#[derive(Clone, Copy, Default)]
struct Node {
    pos: [f32; 3],
    opos: [f32; 3],
    v: [f32; 3],
}

struct Tentacle {
    length: usize,
    radius: f32,
    spacing: f32,
    friction: f32,
    nodes: Vec<Node>,
    color: [f32; 4],
}

struct Squid {
    pos: [f32; 3],
    from: [f32; 3],
    to: [f32; 3],
    ratio: f32,
    pulse: f32,
    rate: f32,
    head_radius: f32,
    tentacles: Vec<Tentacle>,
    color: [f32; 4],
}

struct Hydrostat {
    squids: Vec<Squid>,
    /// Which squid the pointer has hold of, if any.
    dragging: Option<usize>,
    button_down: bool,
    cos_sin_table: [f32; 2 * (TENTACLE_FACES + 1)],

    wireframe: bool,
    speed: f32,
    head_radius: f32,
    thickness: f32,
    length: f32,
    gravity: f32,
    /// Signed, because the flow reverses now and then.
    current: f32,
    friction: f32,
    opacity: f32,
    pulse: bool,
}

impl Hydrostat {
    /// `move_tentacle`. One pass down the chain: constrain, read the velocity
    /// off the movement, damp it, then push it along.
    fn move_tentacle(&self, t: &mut Tentacle) {
        for i in 1..t.length {
            let prev = t.nodes[i - 1].pos;
            let n = &mut t.nodes[i];

            n.pos[0] += n.v[0];
            n.pos[1] += n.v[1];
            n.pos[2] += n.v[2];

            let d = [prev[0] - n.pos[0], prev[1] - n.pos[1], prev[2] - n.pos[2]];
            let da = d[2].atan2(d[0]);

            // Still computing motion in a 2d plane, which is why the tentacles
            // look dumb if the scene is rotated. Upstream's note, and its
            // reason for leaving the trackball out.
            let reach = t.spacing * t.length as f32;
            let p = [
                n.pos[0] + da.cos() * reach,
                n.pos[1] + da.cos() * reach,
                n.pos[2] + da.sin() * reach,
            ];

            n.pos[0] = prev[0] - (p[0] - n.pos[0]);
            n.pos[1] = prev[1] - (p[1] - n.pos[1]);
            n.pos[2] = prev[2] - (p[2] - n.pos[2]);

            for k in 0..3 {
                n.v[k] = n.pos[k] - n.opos[k];
                n.v[k] *= t.friction * (1.0 - self.friction);
            }

            // The device is never rotated here, so this is upstream's default
            // arm: the current pushes sideways and gravity pulls back.
            n.v[0] += self.current;
            n.v[1] += self.current;
            n.v[2] += self.gravity;

            n.opos = n.pos;
        }
    }

    /// `move_squid`. Drift towards the next waypoint, pulse, and drag the
    /// tentacle roots around with the head.
    fn move_squid(&self, sq: &mut Squid) {
        let step = std::f32::consts::PI * 2.0 / sq.tentacles.len() as f32;
        let mut radius = self.head_radius;

        if !self.button_down {
            sq.ratio += self.speed * 0.01;
            if sq.ratio >= 1.0 {
                // A negative ratio is a pause: it eases nowhere until it
                // climbs back through zero.
                sq.ratio = -((frand(2.0) + frand(2.0) + frand(2.0)) as f32);
                sq.from = sq.to;
                sq.to = [
                    250.0 - frand(500.0) as f32,
                    250.0 - frand(500.0) as f32,
                    250.0 - frand(500.0) as f32,
                ];
            }

            let r = if sq.ratio > 0.0 {
                ease(Ease::InOutSine, sq.ratio as f64) as f32
            } else {
                0.0
            };
            for k in 0..3 {
                sq.pos[k] = sq.from[k] + r * (sq.to[k] - sq.from[k]);
            }
        }

        if self.pulse {
            let p = (sq.pulse * std::f32::consts::PI).sin().powi(18);
            sq.head_radius = self.head_radius * 0.7 + self.head_radius * 0.3 * p;
            radius = sq.head_radius * 0.25;
            sq.pulse += sq.rate * self.speed * 0.02;
            if sq.pulse > 1.0 {
                sq.pulse = 0.0;
            }
        }

        for (i, tt) in sq.tentacles.iter_mut().enumerate() {
            let th = i as f32 * step;
            tt.nodes[0].pos = [
                sq.pos[0] + th.cos() * radius,
                sq.pos[1] + th.sin() * radius,
                sq.pos[2],
            ];
            self.move_tentacle(tt);
        }
    }

    /// `head_angle`. Where the trail has got to, averaged, which is the
    /// direction the head leans away from.
    fn head_angle(sq: &Squid) -> f32 {
        let mut sum = [0.0f32; 3];
        for t in &sq.tentacles {
            // Pick a node toward the top.
            let n = t.nodes[t.length / 3].pos;
            for k in 0..3 {
                sum[k] += n[k];
            }
        }
        let n = sq.tentacles.len() as f32;
        for (k, s) in sum.iter_mut().enumerate() {
            *s = *s / n - sq.pos[k];
        }
        -sum[0].atan2(sum[2]) * (180.0 / std::f32::consts::PI)
    }

    /// The head itself: a tall dome for the mantle and a squashed one under it
    /// for the skirt.
    fn head_shape(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let slices = if wire { 4 } else { 24 };
        g.glx.scale(1.0, 1.1, 1.0);
        unit_dome(&mut g.glx, if wire { 8 } else { 16 }, slices, wire);
        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        g.glx.scale(1.0, 0.5, 1.0);
        unit_dome(&mut g.glx, 8, slices, wire);
    }

    /// `draw_head`. Drawn twice when the squid is translucent: once smaller,
    /// so the inside of the mantle shows through the outside.
    fn draw_head(&self, g: &mut Gl, sq: &Squid, scale: f32) {
        let angle = Self::head_angle(sq);
        let scale = scale * 1.1;

        g.glx.push_matrix();
        g.glx.translate(sq.pos[0], sq.pos[1], sq.pos[2]);
        g.glx.scale(sq.head_radius, sq.head_radius, sq.head_radius);
        g.glx.scale(scale, scale, scale);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);

        let mut c2 = sq.color;
        if self.opacity < 1.0 && scale >= 1.0 {
            c2[3] *= 0.6;
        }
        g.glx.color4f(c2[0], c2[1], c2[2], c2[3]);
        g.glx.material_ambient_diffuse(c2);

        g.glx.translate(0.0, 0.3, 0.0);
        g.glx.rotate(angle, 0.0, 0.0, 1.0);
        self.head_shape(g);

        g.glx.pop_matrix();
    }

    /// `draw_squid`. Each tentacle is a tube swept along the chain, and the
    /// head goes on last so it sits over the roots.
    fn draw_squid(&self, g: &mut Gl, sq: &Squid) {
        let wire = self.wireframe;
        g.glx.push_matrix();
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);

        if self.opacity < 1.0 {
            self.draw_head(g, sq, 0.75);
        }

        if !wire {
            g.glx.front_face_cw(false);
        }

        for t in &sq.tentacles {
            g.glx
                .color4f(t.color[0], t.color[1], t.color[2], t.color[3]);
            g.glx.material_ambient_diffuse(t.color);

            if wire {
                g.glx.begin(Shape::LineStrip);
                for n in &t.nodes[..t.length] {
                    g.glx.vertex3f(n.pos[0], n.pos[1], n.pos[2]);
                }
                g.glx.end();
                continue;
            }

            // Upstream keeps one strip open across every tentacle of the squid
            // and changes the material inside it, which real GL allows and a
            // recorder that carries one material per block does not. Closing
            // the strip per tentacle draws the same thing: the joins between
            // them were already degenerate triangles.
            g.glx.begin(Shape::TriangleStrip);
            let mut radius = t.radius * self.thickness;
            let rstep = radius / t.length as f32;

            for j in 0..t.length - 1 {
                let n1 = t.nodes[j].pos;
                let n2 = t.nodes[j + 1].pos;
                let x = n1[0] - n2[0];
                let y = n1[1] - n2[1];
                let z = n1[2] - n2[2];
                let l = (x * x + y * y + z * z).sqrt();
                let r2 = radius - rstep;
                let l2 = (x * x + y * y).sqrt();

                for k in 0..=TENTACLE_FACES {
                    let c = self.cos_sin_table[2 * k];
                    let s = self.cos_sin_table[2 * k + 1];
                    let (x1, y1, z1) = (radius * c, radius * s, 0.0);
                    let (x2, y2, z2) = (r2 * c, r2 * s, l);

                    let mut x1t = (l2 * x * z1 - x * z * y1 + l * y * x1) / (l * l2);
                    let mut z1t = (l2 * y * z1 - y * z * y1 - l * x * x1) / (l * l2);
                    let mut y1t = (z * z1 + l2 * y1) / l;

                    let x2t = (l2 * x * z2 - x * z * y2 + l * y * x2) / (l * l2) + n1[0];
                    let z2t = (l2 * y * z2 - y * z * y2 - l * x * x2) / (l * l2) + n1[1];
                    let y2t = (z * z2 + l2 * y2) / l + n1[2];

                    g.glx.normal3f(x1t, z1t, y1t);

                    x1t += n1[0];
                    z1t += n1[1];
                    y1t += n1[2];

                    // The repeated corners are degenerate triangles that stitch
                    // one segment of the tube to the next.
                    if k == 0 {
                        g.glx.vertex3f(x1t, z1t, y1t);
                    }
                    g.glx.vertex3f(x1t, z1t, y1t);
                    g.glx.vertex3f(x2t, z2t, y2t);
                    if k == TENTACLE_FACES {
                        g.glx.vertex3f(x2t, z2t, y2t);
                    }
                }
                radius = r2;
            }
            g.glx.end();
        }

        self.draw_head(g, sq, 1.0);
        g.glx.pop_matrix();
    }

    /// `make_squid`. The first one starts on screen and the rest come in from
    /// outside it.
    fn make_squid(&self, which: usize) -> Squid {
        let color = [
            0.1 + frand(0.7) as f32,
            0.5 + frand(0.5) as f32,
            0.1 + frand(0.7) as f32,
            self.opacity,
        ];

        let mut pos = [
            200.0 - frand(400.0) as f32,
            200.0 - frand(400.0) as f32,
            -(frand(200.0) as f32),
        ];
        let mut ratio = -(frand(3.0) as f32);

        if which > 0 {
            // Start the others off screen, and moving in.
            let sign = || if random() & 1 != 0 { 1.0 } else { -1.0 };
            pos[0] = 800.0 + frand(500.0) as f32 * sign();
            pos[1] = 800.0 + frand(500.0) as f32 * sign();
            ratio = 0.0;
        }

        Squid {
            pos,
            from: pos,
            to: pos,
            ratio,
            pulse: if self.pulse { frand(1.0) as f32 } else { 0.0 },
            rate: 0.8 + frand(0.2) as f32,
            head_radius: self.head_radius,
            tentacles: Vec::new(),
            color,
        }
    }

    fn make_tentacle(&self, color: [f32; 4], pos: [f32; 3]) -> Tentacle {
        let shade = 0.75 + frand(0.25) as f32;
        let length = (2.0 + self.length * (0.8 + frand(0.4) as f32)) as usize;
        let mut nodes = vec![Node::default(); length + 1];
        for (j, n) in nodes.iter_mut().enumerate() {
            n.pos = [pos[0], pos[1], pos[2] + j as f32];
        }
        Tentacle {
            length,
            radius: 0.05 + frand(0.95) as f32,
            spacing: 0.02 + frand(0.08) as f32,
            friction: 0.7 + frand(0.18) as f32,
            nodes,
            color: [
                shade * color[0],
                shade * color[1],
                shade * color[2],
                color[3],
            ],
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wireframe = g.res.bool("wireframe");
    let count = g.res.int("count").max(1) as usize;
    let mut opacity = g.res.float("opacity") as f32;
    if count == 1 || wireframe {
        opacity = 1.0;
    }
    let opacity = opacity.clamp(0.1, 1.0);

    let mut current = g.res.float("current") as f32;
    if random() & 1 != 0 {
        current = -current;
    }

    let ntentacles = g.res.float("tentacles").max(1.0) as usize;

    let mut cos_sin_table = [0.0f32; 2 * (TENTACLE_FACES + 1)];
    for k in 0..=TENTACLE_FACES {
        let th = k as f32 * std::f32::consts::PI * 2.0 / TENTACLE_FACES as f32;
        cos_sin_table[2 * k] = th.cos();
        cos_sin_table[2 * k + 1] = th.sin();
    }

    let mut this = Hydrostat {
        squids: Vec::new(),
        dragging: None,
        button_down: false,
        cos_sin_table,
        wireframe,
        speed: g.res.float("speed") as f32,
        head_radius: g.res.float("headRadius") as f32,
        thickness: g.res.float("thickness") as f32,
        length: g.res.float("length") as f32,
        gravity: g.res.float("gravity") as f32,
        current,
        friction: g.res.float("friction") as f32,
        opacity,
        pulse: g.res.bool("pulse"),
    };

    for which in 0..count {
        let mut sq = this.make_squid(which);
        sq.tentacles = (0..ntentacles)
            .map(|_| this.make_tentacle(sq.color, sq.pos))
            .collect();
        this.squids.push(sq);
    }

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Hydrostat {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width as f32;
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let s = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        let (w, h) = (g.width(), g.height());
        let point = |x: i32, y: i32| ((x - w / 2) as f32 * 0.7, (y - h / 2) as f32 * 0.7);

        match *event {
            XEvent::ButtonPress { x, y, .. } => {
                let (x, y) = point(x, y);
                // Pretty halfassed hit detection, but it works ok. Upstream's.
                let mut best = f32::MAX;
                let mut hit = None;
                for (i, s) in self.squids.iter().enumerate() {
                    let dx = s.pos[0] - x;
                    let dy = s.pos[2] - y;
                    let d = (dx * dx + dy * dy).sqrt();
                    if d < best {
                        best = d;
                        hit = Some(i);
                    }
                }
                if best > 300.0 {
                    self.dragging = None;
                    return false;
                }
                self.dragging = hit;
                if let Some(i) = hit {
                    self.squids[i].ratio = -3.0;
                }
                self.button_down = true;
                true
            }
            XEvent::ButtonRelease { .. } if self.dragging.is_some() => {
                self.button_down = false;
                self.dragging = None;
                true
            }
            XEvent::MotionNotify { x, y } => {
                let Some(i) = self.dragging else { return false };
                let (x, y) = point(x, y);
                let s = &mut self.squids[i];
                s.pos[0] = x;
                s.pos[2] = y;
                s.from[0] = x;
                s.to[0] = x;
                s.from[2] = y;
                s.to[2] = y;
                s.from[1] = s.pos[1];
                s.to[1] = s.pos[1];
                true
            }
            _ => false,
        }
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.cull_face(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
        }
        g.glx.blend(if self.opacity < 1.0 {
            Blend::AlphaAdd
        } else {
            Blend::Off
        });

        g.glx.push_matrix();
        g.glx.scale(0.03, 0.03, 0.03);

        if self.opacity < 1.0 {
            // Back to front, so the nearer ones are added over the farther.
            self.squids.sort_by(|a, b| b.pos[1].total_cmp(&a.pos[1]));
        }

        for i in 0..self.squids.len() {
            let mut sq = std::mem::replace(&mut self.squids[i], Squid::empty());
            self.move_squid(&mut sq);
            self.draw_squid(g, &sq);
            self.squids[i] = sq;
            if self.opacity < 1.0 {
                g.glx.clear_depth();
            }
        }

        // Reverse the flow every now and then.
        if random().is_multiple_of(700) {
            self.current = -self.current;
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

impl Squid {
    /// A placeholder to swap out while a squid is being moved and drawn, which
    /// needs the rest of the saver at the same time.
    fn empty() -> Self {
        Squid {
            pos: [0.0; 3],
            from: [0.0; 3],
            to: [0.0; 3],
            ratio: 0.0,
            pulse: 0.0,
            rate: 1.0,
            head_radius: 1.0,
            tentacles: Vec::new(),
            color: [0.0; 4],
        }
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*count:        3",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*speed:        1.0",
    "*pulse:        True",
    "*headRadius:   60",
    "*tentacles:    35",
    "*thickness:    18",
    "*length:       55",
    "*gravity:      0.5",
    "*current:      0.25",
    "*friction:     0.02",
    "*opacity:      0.8",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Animation speed", 0.01, 4.0, 0.01, 2, "1.0"),
    Opt::slider("count", "Number of squid", 1.0, 100.0, 1.0, 0, "3"),
    Opt::slider("headRadius", "Head size", 10.0, 100.0, 1.0, 0, "60"),
    Opt::slider("tentacles", "Number of tentacles", 3.0, 100.0, 1.0, 0, "35"),
    Opt::slider("thickness", "Thickness", 3.0, 40.0, 1.0, 0, "18"),
    Opt::slider("length", "Length of tentacles", 10.0, 150.0, 1.0, 0, "55"),
    Opt::slider("gravity", "Gravity", 0.0, 10.0, 0.1, 2, "0.5"),
    Opt::slider("current", "Current", 0.0, 10.0, 0.1, 2, "0.25"),
    Opt::slider("friction", "Viscosity", 0.0, 0.1, 0.001, 3, "0.02"),
    Opt::boolean("pulse", "Pulse", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "hydrostat",
    label: "Hydrostat",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2012",
        video: Some("https://www.youtube.com/watch?v=nn-nA18hFt0"),
        blurb: "Wiggly squid with tentacles simulated by inverse kinematics.",
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
    fn each_tentacle_is_its_own_colour() {
        // Upstream keeps one strip open across the whole squid and changes the
        // material inside it. Closing the strip per tentacle is what makes the
        // shading survive here, so it is worth an assertion.
        let mut r = start(StartArgs::new(
            640,
            480,
            "count=1&tentacles=8&length=12",
            20260811,
        ));
        r.step();
        let f = r.frame();
        let mut colours: Vec<[u32; 3]> = f
            .batches
            .iter()
            .filter(|b| b.primitive == crate::runtime::gl::Primitive::TriangleStrip)
            .map(|b| {
                let m = b.material.ambient_diffuse;
                [m[0].to_bits(), m[1].to_bits(), m[2].to_bits()]
            })
            .collect();
        // Eight tentacles, plus the two domes the head is made of.
        assert_eq!(colours.len(), 8 + 2, "expected one strip per tentacle");
        colours.sort_unstable();
        colours.dedup();
        assert!(colours.len() > 5, "only {} shades", colours.len());
    }

    #[test]
    fn a_tentacle_hangs_below_the_head_it_grows_from() {
        // Gravity acts along z before the scene is turned, so a settled
        // tentacle trails away from its root.
        let mut r = start(StartArgs::new(
            640,
            480,
            "count=1&tentacles=6&length=30&pulse=false",
            20260811,
        ));
        for _ in 0..200 {
            r.step();
        }
        let f = r.frame();
        let zs: Vec<f32> = f.vertices.iter().map(|v| v.pos[2]).collect();
        let lo = zs.iter().copied().fold(f32::MAX, f32::min);
        let hi = zs.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            hi - lo > 10.0,
            "the tentacles did not spread out, {lo}..{hi}"
        );
        assert!(zs.iter().all(|z| z.is_finite()), "a vertex went to NaN");
    }

    #[test]
    fn the_head_leans_away_from_where_the_trail_has_got_to() {
        // The head angle is read off the tentacles, so it has to change as
        // they swing rather than stay put.
        let mut r = start(StartArgs::new(
            640,
            480,
            "count=1&tentacles=6&length=30",
            20260811,
        ));
        let head_matrix = |r: &Runner3d| {
            // The head goes on last, so its batch is the final one.
            let f = r.frame();
            f.batches.last().map(|b| b.modelview.0)
        };
        r.step();
        let a = head_matrix(&r).expect("nothing drawn");
        for _ in 0..120 {
            r.step();
        }
        let b = head_matrix(&r).expect("nothing drawn");
        assert_ne!(a, b, "the head never moved");
    }

    #[test]
    fn one_squid_is_opaque_and_several_are_not() {
        // Upstream forces full opacity when there is nothing to see through.
        let mut one = start(StartArgs::new(640, 480, "count=1&tentacles=4", 20260811));
        one.step();
        assert_eq!(one.frame().batches[0].blend, Blend::Off);

        let mut many = start(StartArgs::new(640, 480, "count=3&tentacles=4", 20260811));
        many.step();
        assert_eq!(many.frame().batches[0].blend, Blend::AlphaAdd);
    }

    #[test]
    fn dragging_moves_the_nearest_squid_and_a_far_click_misses() {
        let mut r = start(StartArgs::new(640, 480, "count=1&tentacles=4", 20260811));
        r.step();
        // The squid starts within 400 units of the middle, so the centre of
        // the window is always a hit.
        assert!(r.event(XEvent::ButtonPress {
            x: 320,
            y: 240,
            button: 1
        }));
        assert!(r.event(XEvent::MotionNotify { x: 500, y: 300 }));
        r.step();
        let moved = r.frame().batches.last().map(|b| b.modelview.0);
        assert!(r.event(XEvent::ButtonRelease {
            x: 500,
            y: 300,
            button: 1
        }));
        // And once let go, a click far from any squid grabs nothing.
        assert!(!r.event(XEvent::ButtonPress {
            x: 10_000,
            y: 10_000,
            button: 1
        }));
        assert!(moved.is_some());
    }
}
