//! Port of `hacks/glx/bubble3d.c`, `b_sphere.c`, `b_draw.c` and
//! `b_lockglue.c`.
//!
//! ```text
//! BUBBLE3D (C) 1998 Richard W.M. Jones.
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
//! ```
//!
//! Rising, undulating 3D bubbles, with transparency and specular reflections.
//!
//! One sphere is built at startup by subdividing an icosahedron three times
//! and pushing every new point out onto the unit sphere, and every bubble is
//! that same sphere. What makes them look like soap rather than billiard balls
//! is the wobble: each bubble picks five random axes at birth and works out,
//! once, how much every vertex leans towards each of them. Thereafter a vertex
//! is pushed out or pulled in by the cosine of five slowly turning angles
//! weighted by those five numbers, which is cheap per frame and never repeats.
//!
//! Bubbles are born below the screen in groups of one to four stacked on top of
//! each other, rise, grow by half again on the way up, and are forgotten at the
//! top. The normal at a vertex is the vertex itself, which is exactly right for
//! a sphere and near enough for a wobbling one.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, SelectItem, StartArgs, frand, random,
};

/// How close two vertices have to be to count as the same one.
const VERTICES_EPSILON: f32 = 0.0005;

/// How many times the icosahedron is subdivided. Two and three are the useful
/// values; upstream picks three on anything with a graphics card.
const SUBDIVISION_DEPTH: i32 = 3;

/// How many directions each bubble is stretched in. Between three and seven
/// gives good results.
const NR_NUDGE_AXES: usize = 5;

/// How far a nudge angle turns each frame, which is how fast it wobbles.
const NUDGE_ANGLE_FACTOR: f32 = 0.01;
/// How far any single nudge can move a vertex. Must not exceed one over the
/// number of axes.
const NUDGE_FACTOR: f32 = 0.20;
const ROTATION_FACTOR: f32 = 0.1;

const CREATE_BUBBLES_EVERY: i32 = 8;
const MAX_BUBBLES: usize = 8;
/// The chance of a group being one, two, three or four bubbles, cumulative.
const P_BUBBLE_GROUP: [f64; 4] = [0.7, 0.8, 0.9, 1.0];

const MAX_SIZE: f32 = 0.5;
const MIN_SIZE: f32 = 0.1;
const MAX_SPEED: f32 = 0.03;
const MIN_SPEED: f32 = 0.005;
/// How much bigger a bubble gets between the bottom of the screen and the top.
const SCALE_FACTOR: f32 = 1.5;
const SCREEN_BOTTOM: f32 = -4.0;
const SCREEN_TOP: f32 = 4.0;

const ICO_X: f32 = 0.525_731_1;
const ICO_Z: f32 = 0.850_650_8;

const ICO_VERTICES: [[f32; 3]; 12] = [
    [-ICO_X, 0.0, ICO_Z],
    [ICO_X, 0.0, ICO_Z],
    [-ICO_X, 0.0, -ICO_Z],
    [ICO_X, 0.0, -ICO_Z],
    [0.0, ICO_Z, ICO_X],
    [0.0, ICO_Z, -ICO_X],
    [0.0, -ICO_Z, ICO_X],
    [0.0, -ICO_Z, -ICO_X],
    [ICO_Z, ICO_X, 0.0],
    [-ICO_Z, ICO_X, 0.0],
    [ICO_Z, -ICO_X, 0.0],
    [-ICO_Z, -ICO_X, 0.0],
];

const ICO_TRIANGLES: [[usize; 3]; 20] = [
    [0, 4, 1],
    [0, 9, 4],
    [9, 5, 4],
    [4, 5, 8],
    [4, 8, 1],
    [8, 10, 1],
    [8, 3, 10],
    [5, 3, 8],
    [5, 2, 3],
    [2, 7, 3],
    [7, 10, 3],
    [7, 6, 10],
    [7, 11, 6],
    [11, 0, 6],
    [0, 1, 6],
    [6, 1, 10],
    [9, 0, 11],
    [9, 11, 2],
    [9, 2, 5],
    [7, 2, 11],
];

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if d == 0.0 {
        return [0.0; 3];
    }
    [v[0] / d, v[1] / d, v[2] / d]
}

/// The one sphere every bubble is made from: a subdivided icosahedron with its
/// points shared between the triangles that meet at them.
struct Sphere {
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[usize; 3]>,
}

impl Sphere {
    fn new(depth: i32) -> Sphere {
        let mut s = Sphere {
            vertices: Vec::new(),
            triangles: Vec::new(),
        };
        for t in ICO_TRIANGLES {
            let (a, b, c) = (ICO_VERTICES[t[0]], ICO_VERTICES[t[1]], ICO_VERTICES[t[2]]);
            let (ia, ib, ic) = (s.save_vertex(a), s.save_vertex(b), s.save_vertex(c));
            s.subdivide(a, ia, b, ib, c, ic, depth);
        }
        s
    }

    /// The index of this point, reusing one that is already close enough.
    /// Slow, but it runs once.
    fn save_vertex(&mut self, v: [f32; 3]) -> usize {
        for (i, e) in self.vertices.iter().enumerate() {
            if (0..3).all(|k| (v[k] - e[k]).abs() <= VERTICES_EPSILON) {
                return i;
            }
        }
        self.vertices.push(v);
        self.vertices.len() - 1
    }

    #[allow(clippy::too_many_arguments)]
    fn subdivide(
        &mut self,
        v1: [f32; 3],
        i1: usize,
        v2: [f32; 3],
        i2: usize,
        v3: [f32; 3],
        i3: usize,
        depth: i32,
    ) {
        if depth == 0 {
            self.triangles.push([i1, i2, i3]);
            return;
        }
        // The midpoint of each edge, pushed back out onto the sphere.
        let mid = |a: [f32; 3], b: [f32; 3]| normalize([a[0] + b[0], a[1] + b[1], a[2] + b[2]]);
        let (v12, v23, v31) = (mid(v1, v2), mid(v2, v3), mid(v3, v1));
        let (i12, i23, i31) = (
            self.save_vertex(v12),
            self.save_vertex(v23),
            self.save_vertex(v31),
        );
        self.subdivide(v1, i1, v12, i12, v31, i31, depth - 1);
        self.subdivide(v2, i2, v23, i23, v12, i12, depth - 1);
        self.subdivide(v3, i3, v31, i31, v23, i23, depth - 1);
        self.subdivide(v12, i12, v23, i23, v31, i31, depth - 1);
    }
}

struct Bubble {
    /// How much each nudge axis pulls on each vertex, worked out once at birth:
    /// `nr_vertices * NR_NUDGE_AXES` of them.
    contributions: Vec<f32>,
    position: [f32; 3],
    scale: f32,
    y_incr: f32,
    scale_incr: f32,
    rot: [f32; 3],
    rot_incr: [f32; 3],
    /// Where each nudge is in its cycle, and how fast it turns.
    nudge_angle: [f32; NR_NUDGE_AXES],
    nudge_angle_incr: [f32; NR_NUDGE_AXES],
    color: [f32; 4],
}

impl Bubble {
    fn new(
        sphere: &Sphere,
        at: [f32; 3],
        scale: f32,
        y_incr: f32,
        scale_incr: f32,
        color: [f32; 4],
    ) -> Bubble {
        let mut nudge_angle_incr = [0.0; NR_NUDGE_AXES];
        for a in &mut nudge_angle_incr {
            *a = frand(1.0) as f32 * NUDGE_ANGLE_FACTOR;
        }

        // Some random directions to be stretched in.
        let axes: Vec<[f32; 3]> = (0..NR_NUDGE_AXES)
            .map(|_| {
                normalize([
                    frand(1.0) as f32 * 2.0 - 1.0,
                    frand(1.0) as f32 * 2.0 - 1.0,
                    frand(1.0) as f32 * 2.0 - 1.0,
                ])
            })
            .collect();

        // How much each axis pulls on each vertex: nothing at all on the far
        // side of the bubble, which is what keeps a nudge local.
        let mut contributions = Vec::with_capacity(sphere.vertices.len() * NR_NUDGE_AXES);
        for v in &sphere.vertices {
            for a in &axes {
                contributions.push((v[0] * a[0] + v[1] * a[1] + v[2] * a[2]).max(0.0));
            }
        }

        let r = || frand(1.0) as f32 * ROTATION_FACTOR * 2.0 - ROTATION_FACTOR;
        Bubble {
            contributions,
            position: at,
            scale,
            y_incr,
            scale_incr,
            rot: [0.0; 3],
            rot_incr: [r(), r(), r()],
            nudge_angle: [0.0; NR_NUDGE_AXES],
            nudge_angle_incr,
            color,
        }
    }

    fn step(&mut self) {
        for k in 0..3 {
            self.rot[k] += self.rot_incr[k];
        }
        for k in 0..NR_NUDGE_AXES {
            self.nudge_angle[k] += self.nudge_angle_incr[k];
        }
        self.position[1] += self.y_incr;
        self.scale += self.scale_incr;
    }

    fn draw(&self, g: &mut Gl, sphere: &Sphere) {
        // Where every vertex has got to, given where the five nudges are in
        // their cycles.
        let moved: Vec<[f32; 3]> = sphere
            .vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let mut s = 0.0;
                for (j, angle) in self.nudge_angle.iter().enumerate() {
                    s += (angle.cos() * NUDGE_FACTOR - NUDGE_FACTOR / 2.0)
                        * self.contributions[i * NR_NUDGE_AXES + j];
                }
                [v[0] * (s + 1.0), v[1] * (s + 1.0), v[2] * (s + 1.0)]
            })
            .collect();

        g.glx.push_matrix();
        g.glx
            .translate(self.position[0], self.position[1], self.position[2]);
        g.glx.rotate(self.rot[0], 1.0, 0.0, 0.0);
        g.glx.rotate(self.rot[1], 0.0, 1.0, 0.0);
        g.glx.rotate(self.rot[2], 0.0, 0.0, 1.0);
        g.glx.scale(self.scale, self.scale, self.scale);

        // Bubbles are drawn inside out, which is what upstream's comment says
        // and what makes the far wall of a transparent one show through.
        g.glx.front_face_cw(true);
        g.glx.material_ambient_diffuse(self.color);
        g.glx.begin(Shape::Triangles);
        for t in &sphere.triangles {
            for &i in t {
                let v = moved[i];
                // On a sphere the point is its own normal, and on a gently
                // wobbling one it is near enough.
                g.glx.normal3f(v[0], v[1], v[2]);
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
        }
        g.glx.end();
        g.glx.pop_matrix();
    }
}

struct Bubble3d {
    sphere: Sphere,
    bubbles: Vec<Bubble>,
    /// Frames since the last group was made.
    bubble_count: i32,
    /// The colour every bubble takes, or `None` for a fresh one each time.
    color: Option<[f32; 4]>,
}

impl Bubble3d {
    fn bubble_color(&self) -> [f32; 4] {
        match self.color {
            Some(c) => c,
            None => [
                (random() % 100) as f32 / 100.0,
                (random() % 100) as f32 / 100.0,
                (random() % 100) as f32 / 100.0,
                0.3,
            ],
        }
    }

    /// A group of one to four bubbles, stacked below each other so they rise
    /// in a string.
    fn create_new_bubbles(&mut self) {
        let r = frand(1.0);
        let n = P_BUBBLE_GROUP.iter().position(|p| r < *p).unwrap_or(3) + 1;

        let mut at = [
            frand(1.0) as f32 * 4.0 - 2.0,
            SCREEN_BOTTOM,
            frand(1.0) as f32 * 2.0 - 2.0,
        ];
        let size = MIN_SIZE + frand(1.0) as f32 * (MAX_SIZE - MIN_SIZE);
        let speed = MIN_SPEED + frand(1.0) as f32 * (MAX_SPEED - MIN_SPEED);
        // Grow by the scale factor over the whole climb.
        let scale_incr = (size * SCALE_FACTOR - size) / ((SCREEN_TOP - SCREEN_BOTTOM) / speed);

        let mut made = Vec::with_capacity(n);
        for _ in 0..n {
            let color = self.bubble_color();
            made.push(Bubble::new(
                &self.sphere,
                at,
                size,
                speed,
                scale_incr,
                color,
            ));
            at[1] -= size * 3.0;
        }
        // The lowest of the group goes first, which is upstream's order.
        made.reverse();
        self.bubbles.extend(made);
    }
}

impl Hack3d for Bubble3d {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        if self.bubbles.len() < MAX_BUBBLES {
            self.bubble_count += 1;
            if self.bubble_count > CREATE_BUBBLES_EVERY {
                self.create_new_bubbles();
                self.bubble_count = 0;
            }
        }

        g.glx.clear();

        for i in 0..self.bubbles.len() {
            self.bubbles[i].step();
            let b = &self.bubbles[i];
            b.draw(g, &self.sphere);
        }
        // Anything that has reached the top is forgotten.
        self.bubbles.retain(|b| b.position[1] < SCREEN_TOP);

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, 1.0 / h, 3.0, 8.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.translate(0.0, 0.0, -5.0);
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let transparent = g.res.bool("transparent");
    let spec = g.res.get("bubblecolor").unwrap_or("auto").to_string();

    // "auto" picks one colour for every bubble, keeping more blue in it;
    // "random" gives each bubble its own; anything else is a colour.
    let color = match spec.as_str() {
        "random" => None,
        "auto" => Some([
            (random() % 100) as f32 / 100.0,
            (random() % 100) as f32 / 100.0,
            (random() % 50) as f32 / 100.0 + 0.50,
            0.3,
        ]),
        _ => {
            // Anything else is a colour name or hex, resolved the same way any
            // other colour resource is.
            let (r, gr, b) = crate::runtime::color::unrgb(g.res.pixel("bubblecolor"));
            Some([
                f32::from(r) / 255.0,
                f32::from(gr) / 255.0,
                f32::from(b) / 255.0,
                0.3,
            ])
        }
    };

    let mut st = Bubble3d {
        sphere: Sphere::new(SUBDIVISION_DEPTH),
        bubbles: Vec::new(),
        bubble_count: CREATE_BUBBLES_EVERY,
        color,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    g.glx.lighting(true);
    // Three lights: below, above-right and behind-left, all white, so a bubble
    // catches a highlight whichever way it is turned.
    for (i, pos) in [
        [0.0, -1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0, 0.0],
        [-1.0, 0.0, 1.0, 0.0],
    ]
    .into_iter()
    .enumerate()
    {
        g.glx.light_enable(i, true);
        g.glx.light_position(i, pos[0], pos[1], pos[2], pos[3]);
        g.glx.light_diffuse(i, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(i, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_ambient(i, [0.0, 0.0, 0.0, 1.0]);
    }
    g.glx.light_model_ambient([0.5, 0.5, 0.5, 1.0]);
    g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
    g.glx.material_shininess(100.0);

    if transparent {
        g.glx.blend(Blend::Alpha);
        g.glx.depth_test(false);
    } else {
        g.glx.blend(Blend::Off);
        g.glx.depth_test(true);
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:       10000",
    "*showFPS:     False",
    "*transparent: True",
    "*bubblecolor: auto",
];

const COLORS: &[SelectItem] = &[
    SelectItem {
        value: "auto",
        label: "Random",
    },
    SelectItem {
        value: "#FF0000",
        label: "Amber",
    },
    SelectItem {
        value: "#FFFF00",
        label: "Yellow",
    },
    SelectItem {
        value: "#00FF00",
        label: "Green",
    },
    SelectItem {
        value: "#00FFFF",
        label: "Cyan",
    },
    SelectItem {
        value: "#0000FF",
        label: "Blue",
    },
    SelectItem {
        value: "#FF00FF",
        label: "Magenta",
    },
    SelectItem {
        value: "random",
        label: "Random per bubble",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::select("bubblecolor", "Bubble color", COLORS, "auto"),
    Opt::boolean("transparent", "Transparent bubbles", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "bubble3d",
    label: "Bubble 3D",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Richard Jones",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=4vcj8sq9FO8"),
        blurb: "Rising, undulating 3D bubbles, with transparency and \
                specular reflections.",
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

    /// Subdividing an icosahedron n times gives four times as many triangles
    /// each round, and every point lands on the unit sphere.
    #[test]
    fn the_sphere_is_a_subdivided_icosahedron() {
        for depth in 0..4 {
            let s = Sphere::new(depth);
            assert_eq!(s.triangles.len(), 20 * 4usize.pow(depth as u32));
            for v in &s.vertices {
                let r = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                assert!((r - 1.0).abs() < 1e-5, "a point is {r} from the middle");
            }
        }
    }

    /// The points are shared between the triangles that meet at them, which is
    /// the whole reason for the close-enough search.
    #[test]
    fn the_points_are_shared_between_triangles() {
        let s = Sphere::new(SUBDIVISION_DEPTH);
        // Euler: a closed triangulated surface has V - E + F = 2, and with
        // every edge in two triangles that is V = F/2 + 2.
        assert_eq!(s.vertices.len(), s.triangles.len() / 2 + 2);
        // And no two of them are in the same place.
        for (i, a) in s.vertices.iter().enumerate() {
            for b in &s.vertices[i + 1..] {
                let d = (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max);
                assert!(d > VERTICES_EPSILON, "two points at {a:?} and {b:?}");
            }
        }
    }

    /// A nudge pulls on the vertices facing its axis and not at all on the
    /// ones facing away, which is what keeps a wobble local.
    #[test]
    fn a_nudge_only_pulls_on_its_own_side() {
        let s = Sphere::new(2);
        let b = Bubble::new(&s, [0.0; 3], 1.0, 0.0, 0.0, [1.0; 4]);
        assert_eq!(b.contributions.len(), s.vertices.len() * NR_NUDGE_AXES);
        assert!(
            b.contributions.iter().all(|c| *c >= 0.0),
            "a contribution should never be negative"
        );
        // Every axis pulls on some vertices and not on others.
        for j in 0..NR_NUDGE_AXES {
            let mine: Vec<f32> = (0..s.vertices.len())
                .map(|i| b.contributions[i * NR_NUDGE_AXES + j])
                .collect();
            assert!(mine.iter().any(|c| *c > 0.5), "axis {j} pulls on nothing");
            assert!(mine.contains(&0.0), "axis {j} pulls on all of it");
        }
    }

    /// Every bubble is the same sphere, so they all have the same triangles.
    #[test]
    fn every_bubble_is_the_same_sphere() {
        let r = run("", 40);
        let f = r.frame();
        let counts: std::collections::BTreeSet<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
            .map(|b| b.count)
            .collect();
        assert_eq!(counts.len(), 1, "the bubbles differ in shape: {counts:?}");
        let n = *counts.iter().next().unwrap();
        assert_eq!(n, 20 * 4usize.pow(SUBDIVISION_DEPTH as u32) * 3);
    }

    /// Bubbles rise, grow as they go, and are forgotten at the top, so there
    /// are always some and never too many.
    #[test]
    fn the_bubbles_rise_and_are_replaced() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        let mut seen_max = 0;
        for _ in 0..600 {
            r.step();
            let n = r
                .frame()
                .batches
                .iter()
                .filter(|b| b.primitive == Primitive::Triangles)
                .count();
            assert!(n <= MAX_BUBBLES + 4, "{n} bubbles is too many");
            seen_max = seen_max.max(n);
        }
        assert!(seen_max >= 4, "only ever {seen_max} bubbles");

        // They climb: the same bubble is higher a moment later.
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..20 {
            r.step();
        }
        let lowest = |r: &Runner3d| {
            r.frame()
                .batches
                .iter()
                .filter(|b| b.primitive == Primitive::Triangles)
                .map(|b| b.modelview.0[13])
                .fold(f32::MAX, f32::min)
        };
        let before = lowest(&r);
        for _ in 0..20 {
            r.step();
        }
        assert!(lowest(&r) > before, "nothing rose");
    }

    /// The wobble moves the surface without moving the bubble, and it is a
    /// wobble rather than a pulse: not every vertex goes the same way at once.
    #[test]
    fn the_surface_wobbles() {
        let s = Sphere::new(2);
        let mut b = Bubble::new(&s, [0.0; 3], 1.0, 0.0, 0.0, [1.0; 4]);
        let radii = |b: &Bubble| -> Vec<f32> {
            s.vertices
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let mut sum = 0.0;
                    for (j, angle) in b.nudge_angle.iter().enumerate() {
                        sum += (angle.cos() * NUDGE_FACTOR - NUDGE_FACTOR / 2.0)
                            * b.contributions[i * NR_NUDGE_AXES + j];
                    }
                    let _ = v;
                    sum + 1.0
                })
                .collect()
        };

        let before = radii(&b);
        for _ in 0..200 {
            b.step();
        }
        let after = radii(&b);
        assert!(
            before.iter().zip(&after).any(|(a, c)| (a - c).abs() > 0.01),
            "the surface never moved"
        );
        // Not a uniform pulse: the amount of movement differs across it.
        let deltas: Vec<f32> = before.iter().zip(&after).map(|(a, c)| c - a).collect();
        let hi = deltas.iter().copied().fold(f32::MIN, f32::max);
        let lo = deltas.iter().copied().fold(f32::MAX, f32::min);
        assert!(hi - lo > 0.01, "the whole bubble moved together");
    }

    /// The colour knob picks one colour for every bubble, or one each.
    #[test]
    fn the_colour_knob_works() {
        let one = run("bubblecolor=%2300FF00", 40);
        let f = one.frame();
        let colours: std::collections::BTreeSet<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
            .map(|b| b.material.ambient_diffuse.map(f32::to_bits))
            .collect();
        assert_eq!(colours.len(), 1, "one colour was asked for");
        let c = f.batches[0].material.ambient_diffuse;
        assert!(c[1] > 0.9 && c[0] < 0.1, "green was asked for, got {c:?}");

        let many = run("bubblecolor=random", 60);
        let f = many.frame();
        let colours: std::collections::BTreeSet<_> = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
            .map(|b| b.material.ambient_diffuse.map(f32::to_bits))
            .collect();
        assert!(colours.len() > 1, "every bubble came out the same colour");
    }

    /// Transparent bubbles blend and do not hide each other; solid ones use
    /// the depth buffer instead.
    #[test]
    fn transparency_turns_off_the_depth_buffer() {
        let clear = run("transparent=true", 20);
        let b = &clear.frame().batches[0];
        assert_eq!(b.blend, Blend::Alpha);
        assert!(!b.depth_test, "a transparent bubble needs no depth test");

        let solid = run("transparent=false", 20);
        let b = &solid.frame().batches[0];
        assert_eq!(b.blend, Blend::Off);
        assert!(b.depth_test);
    }
}
